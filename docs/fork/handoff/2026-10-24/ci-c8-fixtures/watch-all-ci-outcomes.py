import argparse
import json
from pathlib import Path
import subprocess
import time

parser = argparse.ArgumentParser()
parser.add_argument('expected_sha')
parser.add_argument('output_root', type=Path)
parser.add_argument('run_ids', nargs='+')
args = parser.parse_args()
args.output_root.mkdir(parents=True, exist_ok=True)
seen = set()
records = {}

def fail(key, detail):
    if key not in seen:
        seen.add(key)
        with (args.output_root / 'failures.jsonl').open('a') as output:
            output.write(json.dumps({'key': key, 'detail': detail}) + '\n')
        print('FAILED', flush=True)

try:
    while True:
        for run_id in args.run_ids:
            response = subprocess.run([
                'gh', 'run', 'view', run_id, '--repo', 'farukg/grok-build',
                '--json', 'headSha,status,conclusion,jobs,url',
            ], text=True, capture_output=True, timeout=90)
            if response.returncode:
                fail('api:' + run_id, 'GitHub query failed')
                raise SystemExit(1)
            record = json.loads(response.stdout)
            (args.output_root / ('ci-' + run_id + '.json')).write_text(json.dumps(record, indent=2) + '\n')
            if record['headSha'] != args.expected_sha:
                fail('sha:' + run_id, record['headSha'])
                raise SystemExit(1)
            records[run_id] = record
            for job in record['jobs']:
                for step in job['steps']:
                    if step['conclusion'] not in {'', None, 'success', 'neutral', 'skipped'}:
                        fail(f"{run_id}:job:{job['databaseId']}:step:{step['number']}", {
                            'job': job['name'], 'step': step['name'], 'conclusion': step['conclusion'],
                        })
                if job['conclusion'] not in {'', None, 'success', 'neutral', 'skipped'}:
                    fail(f"{run_id}:job:{job['databaseId']}", job['conclusion'])
            if record['conclusion'] not in {'', None, 'success'}:
                fail('run:' + run_id, record['conclusion'])
        if all(records[run_id]['status'] == 'completed' for run_id in args.run_ids):
            green = not seen and all(records[run_id]['conclusion'] == 'success' for run_id in args.run_ids)
            (args.output_root / 'terminal-summary.json').write_text(json.dumps({
                'expected_sha': args.expected_sha, 'run_ids': args.run_ids,
                'all_complete': True, 'run_is_green': green, 'failure_keys': sorted(seen),
            }, indent=2) + '\n')
            print('DONE' if green else 'FAILED', flush=True)
            raise SystemExit(0 if green else 1)
        time.sleep(20)
except (KeyboardInterrupt, SystemExit):
    raise
except Exception as error:
    fail('monitor', type(error).__name__)
    raise SystemExit(1)
