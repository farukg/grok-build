from pathlib import Path
import subprocess
import hashlib
import json
import difflib

repo = Path('/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format')
out = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures')
prior = out / 'continuation-baseline'
review = out / 'review-baseline'
def git(*args):
    return subprocess.check_output(['git', *args], cwd=repo)
def sha(data):
    return hashlib.sha256(data).hexdigest()
tracked = set(git('ls-files').decode().splitlines())
paths = sorted(set(git('diff', '--no-ext-diff', '--name-only').decode().splitlines())
               | set(git('ls-files', '--others', '--exclude-standard').decode().splitlines()))
head_data = {path: git('show', f'HEAD:{path}') if path in tracked else None for path in paths}
base_data = {path: (prior / path).read_bytes() if (prior / path).is_file() else head_data[path] for path in paths}
review_data = {path: (review / path).read_bytes() if (review / path).is_file() else head_data[path] for path in paths}
current = {path: (repo / path).read_bytes() for path in paths}
def make_patch(selected, base):
    chunks = []
    for path in selected:
        old = base[path]
        new = current[path]
        if old == new:
            continue
        prefix = f'diff --git a/{path} b/{path}\n'
        if old is None:
            prefix += 'new file mode 100644\n'
        diff = difflib.unified_diff((old or b'').decode().splitlines(keepends=True),
                                    new.decode().splitlines(keepends=True),
                                    fromfile=f'a/{path}' if old is not None else '/dev/null',
                                    tofile=f'b/{path}')
        chunks.append(prefix + ''.join(diff))
    return ''.join(chunks).encode()
def producer(path):
    return '/agent/subagent/' in path or path.endswith('/agent/testkit/mod.rs') or '/app/acp_handler/tests/' in path
sync = [p for p in paths if p.endswith('subagent_spawn_context_tests.rs') or p.endswith('acp_session_tests/support.rs') or p.endswith('acp_session_tests/completion_persistence.rs')]
producer_paths = [p for p in paths if producer(p)]
pty = [p for p in paths if '/xai-grok-pager-pty-harness/' in p]
pruning = [p for p in paths if '/xai-chat-state/' in p]
segments = {
    'fixtures.patch': (paths, head_data),
    'continuation-fixtures.patch': (paths, base_data),
    'continuation-goal.patch': (sync, base_data),
    'continuation-producer-fixtures.patch': (producer_paths, base_data),
    'continuation-pty.patch': (pty, base_data),
    'continuation-pruning.patch': (pruning, base_data),
    'review-corrections.patch': (paths, review_data),
    'review-event-sync.patch': (sync, review_data),
    'review-replay.patch': (producer_paths, review_data),
    'review-pty.patch': (pty, review_data),
    'review-pruning.patch': (pruning, review_data),
}
artifacts = {}
for name, (selected, base) in segments.items():
    data = make_patch(selected, base)
    (out / name).write_bytes(data)
    artifacts[name] = sha(data)
for name in ['continuation-production.patch', 'review-production.patch']:
    (out / name).write_bytes(b'')
    artifacts[name] = sha(b'')
for name in ['continuation-baseline/prior-twelve-files.patch', 'review-baseline/fixtures.patch']:
    artifacts[name] = sha((out / name).read_bytes())
records = [{'path': str(repo / path), 'relative_path': path,
            'head_sha256': sha(head_data[path]) if head_data[path] is not None else None,
            'continuation_base_sha256': sha(base_data[path]) if base_data[path] is not None else None,
            'review_base_sha256': sha(review_data[path]) if review_data[path] is not None else None,
            'authored_sha256': sha(current[path])} for path in paths]
inputs = {}
for name in ['result.md', 'review.md', 'failed-ci.log', 'failed-ci-readable.log']:
    path = out.parent / 'ci-c8-format' / name
    inputs[str(path)] = sha(path.read_bytes())
for name in ['pty-focus-review.md', 'production-gap-explicit-stop.md', 'review-findings.md']:
    path = out / name
    inputs[str(path)] = sha(path.read_bytes())
protected_paths = [
    'crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator.rs',
    'crates/codegen/xai-grok-tools/src/implementations/grok_build/task/types.rs',
    'crates/codegen/xai-grok-shell/src/agent/subagent/completion_publication.rs',
    'crates/codegen/xai-grok-shell/src/agent/subagent/spawn.rs',
    'crates/codegen/xai-grok-shell/src/agent/subagent/metadata_store.rs',
    'crates/codegen/xai-grok-shell/src/session/storage/jsonl/completion.rs',
    'crates/codegen/xai-grok-shell/src/session/completion_delivery.rs',
    'crates/codegen/xai-grok-shell/src/agent/mvp_agent/session_setup.rs',
    'crates/codegen/xai-grok-shell/src/session/acp_session_impl/goal_support.rs',
    'crates/codegen/xai-grok-shell/src/session/acp_session_impl/notification_drain.rs',
    'crates/codegen/xai-grok-shell/src/session/acp_session_impl/prompt_queue.rs',
    'crates/codegen/xai-grok-shell/src/session/goal_tracker.rs',
    'crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/finished_child_manual_compact_cancel.rs',
]
protected = {path: {'head_sha256': sha(git('show', f'HEAD:{path}')),
                    'authored_sha256': sha((repo / path).read_bytes())} for path in protected_paths}
manifest = {'worktree': str(repo), 'branch': git('branch', '--show-current').decode().strip(),
            'head': git('rev-parse', 'HEAD').decode().strip(),
            'baseline': git('rev-parse', 'c8e075ea').decode().strip(),
            'preserved': {rev: git('rev-parse', rev).decode().strip() for rev in ['4302b167', '04121ef5', 'bb36f7bd']},
            'patch_sha256': artifacts['fixtures.patch'], 'artifacts': artifacts, 'inputs': inputs,
            'sources': records, 'protected_sources': protected}
(out / 'source-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
rows = ['# Paths and SHA256', '', 'Relative paths use the isolated worktree. Absolute paths, HEAD/continuation/review baseline hashes and protected source hashes are in source-manifest.json.', '', '| Source | Authored SHA256 |', '|---|---|']
for record in records:
    rows.append(f"| `{record['relative_path']}` | `{record['authored_sha256']}` |")
rows += ['', '## Artifact SHA256', '']
for name, digest in artifacts.items():
    rows.append(f'- `{name}`: `{digest}`.')
rows += ['', '## Captured inputs', '']
for path, digest in inputs.items():
    rows.append(f'- `{path}`: `{digest}`.')
(out / 'paths-and-hashes.md').write_text('\n'.join(rows) + '\n')
print(json.dumps({'head': manifest['head'], 'sources': len(records), 'artifacts': artifacts}))
