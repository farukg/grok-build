from pathlib import Path
import hashlib
import json
import shutil

root = Path('/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format')
out = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/ci-bc85-runtime')
snapshot = out / 'before-caller-join'
snapshot.mkdir(exist_ok=False)
manifest = json.loads((out / 'source-manifest.json').read_text())
records = []
for entry in manifest['sources']:
    actual = hashlib.sha256((root / entry['path']).read_bytes()).hexdigest()
    records.append({'path': entry['path'], 'expected': entry['current_sha256'], 'actual': actual})
    if actual != entry['current_sha256']:
        raise RuntimeError('source differs from the agreed twelve-file baseline: ' + entry['path'])
for name in ['source-manifest.json', 'hashes.md', 'result.md', 'runtime.patch', 'production.patch', 'fixtures.patch']:
    shutil.copyfile(out / name, snapshot / name)
for name in ['mod', 'rest']:
    source = root / f'crates/codegen/xai-grok-shell/src/agent/subagent/tests/{name}.rs'
    shutil.copyfile(source, snapshot / f'tests-{name}.rs')
(snapshot / 'twelve-source-baseline.json').write_text(json.dumps(records, indent=2) + '\n')
print('Agreed twelve-source baseline preserved:', len(records))
