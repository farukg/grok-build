from pathlib import Path
import re
import json

base = Path('/tmp/grok-goal-4f7553a1dbe6/implementer')
source = base / 'ci-c8-fixtures/bc85-failed-ci.log'
out = base / 'ci-bc85-runtime'
raw = source.read_text()
text = re.sub(r'(?:\x1b|\^\[)\[[0-?]*[ -/]*[@-~]', '', raw)
(out / 'normalized.log').write_text(text)
failed = {}
hanging = {}
for number, line in enumerate(text.splitlines(), 1):
    match = re.search(r' test (\S+) \.\.\. FAILED$', line)
    if match:
        failed.setdefault(match[1], []).append(number)
    match = re.search(r' test (\S+) has been running for over 60 seconds', line)
    if match:
        hanging.setdefault(match[1], []).append(number)
(out / 'diagnostics.json').write_text(json.dumps({'failed_tests': failed, 'hanging_tests': hanging}, indent=2) + '\n')
print(json.dumps({'failed_tests': failed, 'hanging_tests': hanging}, indent=2))
