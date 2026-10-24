from pathlib import Path
import json
import re
import subprocess

root = Path('/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format')
out = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/ci-bc85-runtime')
paths = [root / f'crates/codegen/xai-grok-shell/src/agent/subagent/tests/{name}.rs' for name in ['mod', 'rest']]
regions = []
for path in paths:
    diff = subprocess.check_output(['git', 'diff', '--no-ext-diff', '--unified=0', '--', str(path)], cwd=root, text=True)
    for start, length in re.findall(r'^@@ .*?\+(\d+)(?:,(\d+))? @@', diff, re.M):
        count = int(length or 1)
        if count:
            regions.append({'file': str(path), 'range': [int(start), int(start) + count - 1]})
(out / 'caller-join-format-lines.json').write_text(json.dumps(regions) + '\n')
subprocess.run([
    'rustup', 'run', 'nightly-2026-08-13', 'rustfmt', '--unstable-features',
    '--edition', '2024', '--config-path', str(root / 'rustfmt.toml'),
    '--config', 'skip_children=true', '--file-lines', json.dumps(regions),
    *map(str, paths),
], cwd=root, check=True)
