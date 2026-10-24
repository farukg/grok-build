from pathlib import Path
import json
import hashlib
import difflib
import subprocess
p = Path('/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures')
o = p / 'focus-text-fix'
r = Path('/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format')
rel = 'crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs'
before = json.loads((o / 'before-source-manifest.json').read_text())
subprocess.run(['python', str(p / 'manifest.py')], check=True)
after = json.loads((p / 'source-manifest.json').read_text())
patch = 'diff --git a/' + rel + ' b/' + rel + '\n' + ''.join(difflib.unified_diff((o / 'before-send_subagent_message_row.rs').read_text().splitlines(True), (r / rel).read_text().splitlines(True), fromfile='a/' + rel, tofile='b/' + rel, n=30))
(o / 'focus-text-fix.patch').write_text(patch)
s = (p / 'result.md').read_text()
for name, old in before['artifacts'].items():
    s = s.replace(old, after['artifacts'][name])
old_hash = next(x['authored_sha256'] for x in before['sources'] if x['relative_path'] == rel)
new_hash = next(x['authored_sha256'] for x in after['sources'] if x['relative_path'] == rel)
s = s.replace(old_hash, new_hash).replace(':149–236', ':149–237')
s = s.replace('Der Composertext muss gegenüber vorher identisch bleiben: kein unbeabsichtigter Draft.', 'Der Composertext nach dem Composer-Klick wird fokusgleich mit dem Snapshot vor Tab verglichen: kein unbeabsichtigter Draft; keine Placeholder-Ausfilterung.')
s = s.replace('Composertext bleibt auch danach identisch.', 'Composertext nach dem go-Klick bleibt gegenüber dem bisherigen unfokussierten Snapshot identisch.')
s = s.replace('1. Review der vier `review-*`-Sachpatches, insbesondere der nachgewiesenen **at-least-once-Transport / exactly-once-sichtbare-Wirkung**-Grenze und des ack-erhaltenden Beobachteradapters.', '1. Erneute unabhängige Reviewfreigabe des PTY-Fokusvergleichs offen; Goal-/Replay-/Pruningreview bereits statisch entblockt. Siehe [focus-text-fix/result.md](focus-text-fix/result.md).')
(p / 'result.md').write_text(s)
metadata = {'source_sha256': new_hash, 'fixtures_patch_sha256': after['patch_sha256'], 'single_hunk_patch_sha256': hashlib.sha256(patch.encode()).hexdigest()}
(o / 'final-hashes.json').write_text(json.dumps(metadata, indent=2) + '\n')
(o / 'result.md').write_text('# Fokusgleicher Composervergleich\n\nNur `send_subagent_message_row.rs` geändert: Snapshot vor Tab gegen nach Composer-Klick; bisheriger unfokussierter Snapshot gegen nach go-Klick. Keine Placeholder-Filterung oder zusätzliche Toleranz. Deselektion, neuer go-Rahmen, Draftvergleich, gemeinsame 60s-Deadline und Child-/Caret-/Enter-/ID-/Session-Infooracles bleiben erhalten. Alle anderen Sourcepfade wurden nicht editiert.\n\nBasis: `before-send_subagent_message_row.rs`, `before-source-hashes.json`, `before-source-manifest.json`, `before-fixtures.patch`. Exakter einzelner Hunk: `focus-text-fix.patch`. Finale Gesamtmetadaten: `../source-manifest.json`.\n\n' + '\n'.join('- ' + k + ': `' + v + '`.' for k, v in metadata.items()) + '\n\nKeine Builds, Tests, Checks, cargo, Formatierung, Commits oder Publikation ausgeführt. Nur Scratch-Patch-/Hashausgabe. Erneute unabhängige Reviewfreigabe sowie Kompilierung/Runtime bleiben offen.\n')
