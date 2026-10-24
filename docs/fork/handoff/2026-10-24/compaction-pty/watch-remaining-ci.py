import json
import subprocess
import sys
import time
from pathlib import Path

expected_sha = 'c0e3db3ff50410c7d1b4abc8b17beb346b5b2884'
root = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/compaction-pty')
run_ids = ['39819829494', '39819830289']
failed = {'failure', 'timed_out', 'cancelled', 'action_required', 'startup_failure', 'stale'}
known_step = 'Check sampler, shell, tools and pager formatting'
while True:
    complete = True
    for run_id in run_ids:
        result = subprocess.run(['gh', 'run', 'view', run_id, '--repo', 'farukg/grok-build', '--json', 'headSha,status,conclusion,jobs,url'], text=True, capture_output=True)
        (root / f'ci-{run_id}.stderr.log').write_text(result.stderr)
        if result.returncode:
            print('FAILED', flush=True)
            sys.exit(1)
        (root / f'ci-{run_id}.json').write_text(result.stdout)
        record = json.loads(result.stdout)
        known_format_failure = False
        unexpected = record['headSha'] != expected_sha
        for job in record.get('jobs', []):
            for step in job.get('steps', []):
                if step.get('conclusion') in failed:
                    if run_id == '39819829494' and step['name'] == known_step and step['conclusion'] == 'failure':
                        known_format_failure = True
                    else:
                        unexpected = True
            if job.get('conclusion') in failed and not (known_format_failure and job['conclusion'] == 'failure'):
                unexpected = True
        if record.get('conclusion') in failed and not (known_format_failure and record['conclusion'] == 'failure'):
            unexpected = True
        if unexpected:
            print('FAILED', flush=True)
            sys.exit(1)
        if record['status'] != 'completed':
            complete = False
        elif record.get('conclusion') != 'success' and not known_format_failure:
            print('FAILED', flush=True)
            sys.exit(1)
    if complete:
        print('DONE', flush=True)
        sys.exit(0)
    time.sleep(30)
