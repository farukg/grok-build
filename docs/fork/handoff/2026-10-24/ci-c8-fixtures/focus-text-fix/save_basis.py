from pathlib import Path
import shutil
import hashlib
import json
p = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures')
o = p / 'focus-text-fix'
o.mkdir(exist_ok=True)
r = Path('/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format')
m = json.loads((p / 'source-manifest.json').read_text())
for name in ['source-manifest.json', 'fixtures.patch', 'result.md']:
    shutil.copyfile(p / name, o / ('before-' + name))
f = r / 'crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs'
shutil.copyfile(f, o / 'before-send_subagent_message_row.rs')
hashes = {s['relative_path']: hashlib.sha256((r / s['relative_path']).read_bytes()).hexdigest() for s in m['sources']}
(o / 'before-source-hashes.json').write_text(json.dumps(hashes, indent=2) + '\n')
