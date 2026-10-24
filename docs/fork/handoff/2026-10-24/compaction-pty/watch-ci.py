import json
import subprocess
import sys
import time
from pathlib import Path

expected_sha = sys.argv[1]
root = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/compaction-pty')
run_ids = sys.argv[2:]
failed = {'failure', 'timed_out', 'cancelled', 'action_required', 'startup_failure', 'stale'}
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
        conclusions = [record.get('conclusion')]
        for job in record.get('jobs', []):
            conclusions.append(job.get('conclusion'))
            conclusions.extend(step.get('conclusion') for step in job.get('steps', []))
        if record['headSha'] != expected_sha or any(value in failed for value in conclusions):
            print('FAILED', flush=True)
            sys.exit(1)
        if record['status'] != 'completed':
            complete = False
        elif record.get('conclusion') != 'success':
            print('FAILED', flush=True)
            sys.exit(1)
    if complete:
        print('DONE', flush=True)
        sys.exit(0)
    time.sleep(30)
