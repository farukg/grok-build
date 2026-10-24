from pathlib import Path
import hashlib
import shutil
import subprocess

owner = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/ci-bc85-runtime')
archive = owner / 'before-parent-format'
archive.mkdir(exist_ok=False)
for name in ['source-manifest.json', 'runtime.patch', 'production.patch', 'fixtures.patch', 'hashes.md', 'result.md']:
    shutil.copyfile(owner / name, archive / name)
subprocess.run(['python3', str(owner / 'artifacts.py')], check=True)
print('current_manifest_sha256:', hashlib.sha256((owner / 'source-manifest.json').read_bytes()).hexdigest())
