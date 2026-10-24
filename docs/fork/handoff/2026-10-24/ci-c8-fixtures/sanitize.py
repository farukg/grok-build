from pathlib import Path
import re
import json
root = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures')
src = root.parent / 'ci-c8-format/failed-ci-readable.log'
text = src.read_text(encoding='utf-8-sig')
text = re.sub(r'(?:\x1b\[|\^\[\[)[0-?]*[ -/]*[@-~]', '', text)
lines = [re.sub(r'^.*?2026-10-23T\S+Z ?', '', line) for line in text.splitlines()]
(root / 'captured-sanitized.log').write_text('\n'.join(lines) + '\n')
blocks = []
for i, line in enumerate(lines):
    if re.match(r'error(?:\[E\d+\])?:', line):
        j = i + 1
        while j < len(lines) and lines[j].strip() and j < i + 65 and not re.match(r'(?:error(?:\[E\d+\])?:|warning:|For more information|Some errors|error: could not)', lines[j]):
            j += 1
        block = '\n'.join(lines[i:j]).strip()
        if block not in blocks:
            blocks.append(block)
(root / 'compiler-blocks.txt').write_text('\n\n'.join(blocks) + '\n')
print(f'{len(blocks)} distinct diagnostic blocks (including summaries); evidence only')
