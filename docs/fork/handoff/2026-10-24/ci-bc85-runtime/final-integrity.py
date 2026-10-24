from pathlib import Path
import hashlib
import json

root = Path('/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format')
out = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/ci-bc85-runtime')
manifest = json.loads((out / 'source-manifest.json').read_text())
records = []
for entry in manifest['sources']:
    data = (root / entry['path']).read_bytes()
    records.append({
        'path': entry['path'],
        'bytes': len(data),
        'matches_handoff': hashlib.sha256(data).hexdigest() == entry['current_sha256'],
        'ends_with_newline': data.endswith(b'\n'),
        'final_lines': data.decode().splitlines()[-8:],
    })
(out / 'final-integrity.json').write_text(json.dumps(records, indent=2) + '\n')
for entry in records:
    print(entry['matches_handoff'], entry['ends_with_newline'], entry['path'])
print('result_tail:', (out / 'result.md').read_text().splitlines()[-1])
