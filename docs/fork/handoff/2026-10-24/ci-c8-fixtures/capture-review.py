from pathlib import Path
import subprocess
import json
import hashlib
repo = Path('/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format')
root = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures')
out = root / 'review-baseline'
paths = sorted(set(subprocess.check_output(['git','diff','--name-only'], cwd=repo).decode().splitlines()) | set(subprocess.check_output(['git','ls-files','--others','--exclude-standard'], cwd=repo).decode().splitlines()))
records = []
for path in paths:
    data = (repo / path).read_bytes()
    dest = out / path
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_bytes(data)
    records.append({'path': path, 'sha256': hashlib.sha256(data).hexdigest()})
for name in ['result.md','source-manifest.json','paths-and-hashes.md','fixtures.patch','continuation-fixtures.patch','continuation-goal.patch','continuation-producer-fixtures.patch','continuation-pty.patch']:
    (out / name).write_bytes((root / name).read_bytes())
(out / 'manifest.json').write_text(json.dumps(records, indent=2) + '\n')
print(f'Preserved the reviewed {len(paths)}-source snapshot; no source mutations.')
