from pathlib import Path
import hashlib
import json
import subprocess

out = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/restart/grok-owner-handoff')
branches = {
    'canonical': 'handoff/grok-canonical-20261024',
    'runtime': 'handoff/grok-runtime-20261024',
}
results = {}
for label, branch in branches.items():
    manifest = json.loads((out / label / 'source.json').read_text())
    repo = Path(manifest['repo'])
    def git(*args, **kwargs):
        return subprocess.run(['git', '-C', str(repo), *args], check=True, **kwargs)
    def output(*args):
        return subprocess.check_output(['git', '-C', str(repo), *args])
    assert output('rev-parse', 'HEAD').decode().strip() == manifest['head']
    assert not output('diff', '--cached', '--name-only', '-z')
    exists = subprocess.run(['git', '-C', str(repo), 'show-ref', '--verify', '--quiet', 'refs/heads/' + branch]).returncode
    assert exists == 1, f'branch already exists: {branch}'
    paths = [entry['path'] for entry in manifest['files']]
    for entry in manifest['files']:
        assert hashlib.sha256((repo / entry['path']).read_bytes()).hexdigest() == entry['sha256'], entry['path']
    git('add', '--', *paths)
    staged = {path for path in output('diff', '--cached', '--name-only', '-z').decode().split('\0') if path}
    assert staged == set(paths)
    with (out / label / 'commit-output.log').open('w') as log:
        git('commit', '--no-verify', '--only', '-F', str(out / (label + '-message.txt')), '--', *paths, stdout=log, stderr=subprocess.STDOUT)
    commit = output('rev-parse', 'HEAD').decode().strip()
    git('branch', branch, commit)
    assert not output('status', '--porcelain')
    for entry in manifest['files']:
        committed = output('show', f"{commit}:{entry['path']}")
        assert hashlib.sha256(committed).hexdigest() == entry['sha256'], entry['path']
    results[label] = {'repo': str(repo), 'commit': commit, 'branch': branch, 'parent': manifest['head'], 'preserved_paths': len(paths), 'working_tree_clean': True, 'published': False}
    (out / 'checkpoints.json').write_text(json.dumps(results, indent=2) + '\n')
print(json.dumps(results, indent=2))
