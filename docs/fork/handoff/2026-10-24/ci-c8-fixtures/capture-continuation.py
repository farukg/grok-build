from pathlib import Path
import subprocess
import hashlib
import json
repo = Path('/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format')
out = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures/continuation-baseline')
out.mkdir(exist_ok=True)
paths = subprocess.check_output(['git', 'diff', '--name-only'], cwd=repo).decode().splitlines()
extra = ['crates/codegen/xai-grok-shell/src/agent/mvp_agent/tests/subagent_spawn_context_tests.rs']
records = []
for path in dict.fromkeys(paths + extra):
    data = (repo / path).read_bytes()
    dest = out / path
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_bytes(data)
    records.append({'path': path, 'sha256': hashlib.sha256(data).hexdigest()})
(out / 'prior-twelve-files.patch').write_bytes(subprocess.check_output(['git', 'diff', '--no-ext-diff', '--binary'], cwd=repo))
(out / 'manifest.json').write_text(json.dumps(records, indent=2) + '\n')
for name in ['result.md', 'source-manifest.json', 'paths-and-hashes.md']:
    (out / name).write_bytes((out.parent / name).read_bytes())
print(f'Captured {len(paths)} previously edited files plus the untouched Goal test owner; no source mutations.')
