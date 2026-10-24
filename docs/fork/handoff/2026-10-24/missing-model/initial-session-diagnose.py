import json
from pathlib import Path
p = Path('/home/fg/.grok/sessions/%2Fhome%2Ffg%2Fprojects/01a15e70-cc42-70d2-8b7b-a20615d0abf4/chat_history.jsonl')
from collections import Counter
p = p.with_name('events.jsonl')
counts = Counter()
selected = []
with p.open() as f:
    for line in f:
        item = json.loads(line)
        counts[item.get('type')] += 1
        ts = item.get('ts', '')
        kind = item.get('type', '')
        if '2026-10-22T09:35' <= ts <= '2026-10-22T11:00' and any(s in kind for s in ['goal', 'turn_', 'error', 'sampling']):
            selected.append(item)
print(counts)
for item in selected:
    print(json.dumps(item, ensure_ascii=False)[:1700])
