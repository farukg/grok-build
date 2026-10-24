import hashlib
import json
from pathlib import Path
import subprocess
import sys

scratch = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures')
manifest = json.loads((scratch / 'source-manifest.json').read_text())
root = Path(manifest['worktree'])
pruning = {'crates/codegen/xai-chat-state/src/actor/tests.rs'}
pty = {
    'crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs',
    'crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_selection.rs',
}
paths = {entry['relative_path'] for entry in manifest['sources']}
groups = {'pruning': pruning, 'pty': pty, 'completion': paths - pruning - pty}
selected = groups[sys.argv[1]]
assert selected and selected <= paths
subprocess.run(['git', '-C', str(root), 'diff', '--cached', '--quiet'], check=True)
for entry in manifest['sources']:
    assert hashlib.sha256(Path(entry['path']).read_bytes()).hexdigest() == entry['authored_sha256'], entry['relative_path']
subprocess.run(['git', '-C', str(root), 'add', '--', *sorted(selected)], check=True)
staged = set(subprocess.check_output(['git', '-C', str(root), 'diff', '--cached', '--name-only'], text=True).splitlines())
assert staged == selected, sorted(staged ^ selected)
print(f'Staged {len(selected)} verified {sys.argv[1]} source paths only.')
