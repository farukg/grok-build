from pathlib import Path
import json
import re
import subprocess

root = Path('/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format')
out = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/ci-bc85-runtime')
changed = subprocess.check_output(['git', 'diff', '--no-ext-diff', '--name-only'], cwd=root, text=True).splitlines()
new = subprocess.check_output(['git', 'ls-files', '--others', '--exclude-standard'], cwd=root, text=True).splitlines()
regions = []
for path in changed:
    diff = subprocess.check_output(['git', 'diff', '--no-ext-diff', '--unified=0', '--', path], cwd=root, text=True)
    for line, length in re.findall(r'^@@ .*?\+(\d+)(?:,(\d+))? @@', diff, re.M):
        count = int(length or 1)
        if count:
            regions.append({'file': str(root / path), 'range': [int(line), int(line) + count - 1]})
for path in new:
    regions.append({'file': str(root / path), 'range': [1, len((root / path).read_text().splitlines())]})
(out / 'format-lines.json').write_text(json.dumps(regions))
(out / 'format-paths.txt').write_text('\n'.join(str(root / path) for path in changed + new) + '\n')
