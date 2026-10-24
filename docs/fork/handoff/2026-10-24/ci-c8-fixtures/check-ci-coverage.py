from pathlib import Path
import yaml

path = Path('/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format/.github/workflows/linux-test.yml')
data = yaml.safe_load(path.read_text())
steps = data['jobs']['test']['steps']
by_name = {step.get('name'): step for step in steps if 'name' in step}
shell = by_name['Test shell']['run']
assert '"$TEST_FILTER"' in shell and ':-remove' not in shell
assert 'implementations::grok_build::task::' in by_name['Test subagent delivery and live steering']['run']
pty = by_name['Test finished child compaction through the terminal']['run']
assert pty.count('send_subagent_message_row') == 2
assert 'send_subagent_message_row -- --ignored' in pty
assert 'finished_child_manual_compact_cancel -- --ignored' in pty
print('YAML parsed; full shell default, task coordinator/backend scope and both PTY paths are present.')
