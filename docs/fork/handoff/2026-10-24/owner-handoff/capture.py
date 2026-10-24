from pathlib import Path
import hashlib
import json
import shutil
import stat
import subprocess
import tarfile

out = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/restart/grok-owner-handoff')
out.mkdir(parents=True, exist_ok=True)
repos = {
    'canonical': Path('/home/fg/projects/org/forks/grok-build'),
    'runtime': Path('/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format'),
}
summary = {}
for label, repo in repos.items():
    target = out / label
    target.mkdir(exist_ok=True)
    def git(*args):
        return subprocess.check_output(['git', '-C', str(repo), *args])
    head = git('rev-parse', 'HEAD').decode().strip()
    branch = git('branch', '--show-current').decode().strip()
    staged = git('diff', '--cached', '--name-only', '-z')
    assert not staged, f'{label}: pre-existing staged paths require separate handling'
    tracked = git('diff', '--name-only', '-z').decode().split('\0')
    untracked = git('ls-files', '--others', '--exclude-standard', '-z').decode().split('\0')
    paths = sorted(set(path for path in tracked + untracked if path))
    records = []
    for relative in paths:
        path = repo / relative
        assert stat.S_ISREG(path.lstat().st_mode), str(path)
        data = path.read_bytes()
        records.append({'path': relative, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest(), 'newline_at_eof': data.endswith(b'\n')})
    (target / 'tracked.patch').write_bytes(git('diff', '--binary', '--full-index', '--no-ext-diff'))
    (target / 'status.txt').write_bytes(git('status', '--short'))
    (target / 'diffstat.txt').write_bytes(git('diff', '--stat'))
    with tarfile.open(target / 'working-source.tar.gz', 'w:gz') as archive:
        for relative in paths:
            archive.add(repo / relative, arcname=relative, recursive=False)
    current = {'repo': str(repo), 'head': head, 'branch': branch, 'files': records}
    (target / 'source.json').write_text(json.dumps(current, indent=2) + '\n')
    summary[label] = {'repo': str(repo), 'head': head, 'branch': branch, 'changed_paths': len(paths), 'manifest_sha256': hashlib.sha256((target / 'source.json').read_bytes()).hexdigest()}
    for entry in records:
        assert hashlib.sha256((repo / entry['path']).read_bytes()).hexdigest() == entry['sha256'], entry['path']
(out / 'inventory.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps(summary, indent=2))
