from pathlib import Path
import hashlib
import json
import re
import subprocess

owner = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/restart/grok-owner-handoff')
scratch = owner.parent.parent
report = owner / 'handoff.md'
for document in [report, scratch / 'ci-bc85-runtime/result.md', scratch / 'ci-bc85-runtime/review.md']:
    for reference in re.findall(r'\]\(([^)]+)\)', document.read_text()):
        assert (document.parent / reference.split('#')[0]).exists(), (str(document), reference)
for relative in [
    'delivery-shell/architect-restart-continuation.md',
    'ci-c8-fixtures/result.md',
    'ci-c8-fixtures/review-unblock.md',
    'ci-c8-fixtures/bc85-failed-ci.log',
    'ci-c8-fixtures/ci-bc85-results/terminal-summary.json',
    'pager-global-display.md',
    'pager-global-restart-fix.md',
    'missing-model',
    'compaction-pty',
]:
    assert (scratch / relative).exists(), relative
checkpoints = json.loads((owner / 'checkpoints.json').read_text())
for label, checkpoint in checkpoints.items():
    repo = Path(checkpoint['repo'])
    resolved = subprocess.check_output(['git', '-C', str(repo), 'rev-parse', checkpoint['branch']], text=True).strip()
    assert resolved == checkpoint['commit']
    assert not subprocess.check_output(['git', '-C', str(repo), 'status', '--porcelain'])
    source = json.loads((owner / label / 'source.json').read_text())
    for entry in source['files']:
        data = subprocess.check_output(['git', '-C', str(repo), 'show', f"{resolved}:{entry['path']}"])
        assert hashlib.sha256(data).hexdigest() == entry['sha256'], entry['path']
result = {'handoff_sha256': hashlib.sha256(report.read_bytes()).hexdigest(), 'links_exist': True, 'checkpoints_match_source_manifests': True, 'both_working_trees_clean': True, 'publication_authorized': False, 'new_compile_or_runtime_acceptance': False}
(owner / 'handoff-verification.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, indent=2))
