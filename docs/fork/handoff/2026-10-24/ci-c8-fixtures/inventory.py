from pathlib import Path
import re
import json
from collections import Counter
root = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures')
lines = (root / 'captured-sanitized.log').read_text().splitlines()
items = {}
for i, line in enumerate(lines):
    if not re.match(r'error(?:\[E\d+\])?:', line):
        continue
    code = re.match(r'error(?:\[(E\d+)\])?: (.*)', line)
    if code is None:
        continue
    location = None
    for following in lines[i+1:i+5]:
        m = re.match(r'\s*--> (?:/home/runner/work/grok-build/grok-build/)?(crates/.*\.rs):(\d+):(\d+)', following)
        if m:
            location = (m[1], int(m[2]), int(m[3]))
            break
    if location is None:
        continue
    key = (code[1], code[2], *location)
    items.setdefault(key, {'code': code[1] or 'syntax', 'message': code[2], 'path': location[0], 'line': location[1], 'column': location[2], 'occurrences': 0})['occurrences'] += 1
inventory = sorted(items.values(), key=lambda item: (item['path'], item['line'], item['column']))
(root / 'diagnostics.json').write_text(json.dumps(inventory, indent=2) + '\n')
rows = ['# Captured compiler baseline (deduplicated)', '', f'{len(inventory)} primary source diagnostics; repeated CI steps are occurrences only.', '']
for item in inventory:
    rows.append(f"- {item['code']} `{item['path']}:{item['line']}:{item['column']}` — {item['message']} (captured {item['occurrences']} times)")
(root / 'diagnostics.md').write_text('\n'.join(rows) + '\n')
print(json.dumps({'primary_diagnostics': len(inventory), 'codes': dict(Counter(i['code'] for i in inventory)), 'crates': dict(Counter(i['path'].split('/')[2] for i in inventory))}))
