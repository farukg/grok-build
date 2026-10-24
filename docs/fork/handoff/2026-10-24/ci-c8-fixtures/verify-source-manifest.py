import hashlib
import json
import pathlib
import subprocess

scratch = pathlib.Path('/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures')
manifest = json.loads((scratch / 'source-manifest.json').read_text())
root = pathlib.Path(manifest['worktree'])
assert subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip() == manifest['head']
expected = {entry['relative_path'] for entry in manifest['sources']}
changed = subprocess.check_output(['git', '-C', str(root), 'diff', '--name-only', 'HEAD'], text=True).splitlines()
untracked = subprocess.check_output(['git', '-C', str(root), 'ls-files', '--others', '--exclude-standard'], text=True).splitlines()
assert set(changed + untracked) == expected, sorted(set(changed + untracked) ^ expected)
for entry in manifest['sources']:
    path = pathlib.Path(entry['path'])
    assert path.relative_to(root).as_posix() == entry['relative_path']
    assert hashlib.sha256(path.read_bytes()).hexdigest() == entry['authored_sha256'], str(path)
for relative, digest in manifest['artifacts'].items():
    assert hashlib.sha256((scratch / relative).read_bytes()).hexdigest() == digest, relative
message = f"Verified {len(expected)} exact source paths, preserved HEAD and all review artifact hashes."
(scratch / 'source-validation.log').write_text(message + '\n')
print(message)
