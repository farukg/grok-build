from pathlib import Path
import hashlib
import json
import subprocess

root = Path('/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format')
out = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/ci-bc85-runtime')
inputs = out.parent / 'ci-c8-fixtures'

def git(*args):
    return subprocess.check_output(['git', *args], cwd=root)

def digest(data):
    return hashlib.sha256(data).hexdigest()

tracked = git('diff', '--no-ext-diff', '--name-only').decode().splitlines()
new = git('ls-files', '--others', '--exclude-standard').decode().splitlines()
paths = sorted(tracked + new)
production_path = 'crates/codegen/xai-grok-shell/src/session/storage/jsonl/completion.rs'
patches = {}
for path in paths:
    if path in new:
        result = subprocess.run(['git', 'diff', '--no-ext-diff', '--no-index', '--', '/dev/null', path], cwd=root, stdout=subprocess.PIPE)
        if result.returncode != 1:
            raise RuntimeError('new source patch unavailable')
        patches[path] = result.stdout
    else:
        patches[path] = git('diff', '--no-ext-diff', '--', path)
(out / 'runtime.patch').write_bytes(b''.join(patches.values()))
(out / 'production.patch').write_bytes(patches[production_path])
(out / 'fixtures.patch').write_bytes(b''.join(patch for path, patch in patches.items() if path != production_path))
manifest = {
    'head': git('rev-parse', 'HEAD').decode().strip(),
    'source_tree': git('rev-parse', 'HEAD^{tree}').decode().strip(),
    'sources': [{
        'path': path,
        'head_sha256': None if path in new else digest(git('show', f'HEAD:{path}')),
        'current_sha256': digest((root / path).read_bytes()),
        'bytes': (root / path).stat().st_size,
        'ends_with_newline': (root / path).read_bytes().endswith(b'\n'),
        'final_lines': (root / path).read_text().splitlines()[-8:],
    } for path in paths],
    'protected_sources': [],
    'artifacts': [],
}
for path in [
    'crates/codegen/xai-grok-shell/src/agent/subagent/metadata_store.rs',
    'crates/codegen/xai-grok-shell/src/agent/subagent/completion_publication.rs',
    'crates/codegen/xai-grok-shell/src/agent/mvp_agent/tests/subagent_spawn_context_tests.rs',
    'crates/codegen/xai-grok-pager/src/app/acp_handler/tests/completion_replay.rs',
    'crates/codegen/xai-grok-shell/src/agent/subagent/tests/mod.rs',
]:
    manifest['protected_sources'].append({
        'path': path,
        'head_sha256': digest(git('show', f'HEAD:{path}')),
        'current_sha256': digest((root / path).read_bytes()),
    })
for path in [
    inputs / 'bc85-failed-ci.raw', inputs / 'bc85-failed-ci.log',
    out / 'normalized.log', out / 'diagnostics.json',
    out / 'runtime.patch', out / 'production.patch', out / 'fixtures.patch',
    out / 'resume-integrity.json', out / 'resume-baseline-source-manifest.json',
]:
    data = path.read_bytes()
    manifest['artifacts'].append({'path': str(path), 'bytes': len(data), 'sha256': digest(data)})
(out / 'source-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
lines = ['# Source- und Artefakthashes', '', '| Source | HEAD SHA256 | Aktuell SHA256 |', '|---|---|---|']
for row in manifest['sources']:
    lines.append(f"| `{row['path']}` | `{row['head_sha256'] or 'neu'}` | `{row['current_sha256']}` |")
lines += ['', '| Artefakt | Bytes | SHA256 |', '|---|---|---|']
for row in manifest['artifacts']:
    lines.append(f"| `{Path(row['path']).name}` | {row['bytes']} | `{row['sha256']}` |")
(out / 'hashes.md').write_text('\n'.join(lines) + '\n')
print('\n'.join(f"{Path(row['path']).name}: {row['sha256']} ({row['bytes']} Bytes)" for row in manifest['artifacts']))
