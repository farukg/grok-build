from pathlib import Path
import hashlib
import json
import shlex
import subprocess

scratch = Path('/tmp/grok-goal-4f7553a1dbe6/implementer')
owner = scratch / 'ci-bc85-runtime'
repo = Path('/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format')
manifest_bytes = (owner / 'source-manifest.json').read_bytes()
assert hashlib.sha256(manifest_bytes).hexdigest() == '66bc3b8b34bd588315a53079c404f3dd12f43b43850957e4b761917570b0dec6'
manifest = json.loads(manifest_bytes)
assert subprocess.check_output(['git', '-C', str(repo), 'rev-parse', 'HEAD'], text=True).strip() == manifest['head']
expected = {entry['path'] for entry in manifest['sources']}
tracked = subprocess.check_output(['git', '-C', str(repo), 'diff', '--name-only'], text=True).splitlines()
untracked = subprocess.check_output(['git', '-C', str(repo), 'ls-files', '--others', '--exclude-standard'], text=True).splitlines()
assert set(tracked + untracked) == expected, sorted(set(tracked + untracked) ^ expected)

def check_sources():
    for entry in manifest['sources'] + manifest['protected_sources']:
        assert hashlib.sha256((repo / entry['path']).read_bytes()).hexdigest() == entry['current_sha256'], entry['path']

check_sources()
output = owner / 'parent-format-check'
output.mkdir(exist_ok=True)
results = []
for invocation in (scratch / 'ci-c8-format/formatter-invocations.txt').read_text().splitlines():
    argv = shlex.split(invocation)
    package = next(path.split('/crates/codegen/')[1].split('/')[0] for path in argv if '/crates/codegen/' in path)
    with (output / (package + '.log')).open('w') as log:
        result = subprocess.run(argv + ['--check'], cwd=repo, stdout=log, stderr=subprocess.STDOUT)
    results.append({'package': package, 'exit_code': result.returncode})
check_sources()
(output / 'results.json').write_text(json.dumps(results, indent=2) + '\n')
print(json.dumps(results))
raise SystemExit(0 if all(item['exit_code'] == 0 for item in results) else 1)
