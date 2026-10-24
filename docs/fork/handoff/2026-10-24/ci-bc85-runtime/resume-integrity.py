from pathlib import Path
import hashlib
import json

root = Path('/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format')
out = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/ci-bc85-runtime')
manifest = json.loads((out / 'source-manifest.json').read_text())
records = []
for entry in manifest['sources']:
    data = (root / entry['path']).read_bytes()
    actual = hashlib.sha256(data).hexdigest()
    records.append({
        'path': entry['path'], 'bytes': len(data),
        'matches_previous_handoff': actual == entry['current_sha256'],
        'sha256': actual,
        'ends_with_newline': data.endswith(b'\n'),
        'final_lines': data.decode().splitlines()[-8:],
    })
(out / 'resume-integrity.json').write_text(json.dumps(records, indent=2) + '\n')
for entry in records:
    print(entry['matches_previous_handoff'], entry['bytes'], entry['path'])
    print('\n'.join(entry['final_lines']))
