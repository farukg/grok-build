# Unabhängiger PTY-Fokusbeleg

Reviewer `01a16a96-1016-7170-b835-a11f37023203`, rein statisch, keine Ausführung oder Änderung.

- Die feste `Space:prompt`-Erwartung in `xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs:147–150` ist stale: `xai-grok-pager/src/views/agent.rs:931–973` unterdrückt den Hint bei kollabierten Userprompts. Daraus folgt kein Produktfokusbug.
- Typunabhängige Fokus-Oracle: gegenüber dem Bild vor Tab neu erschienenen Selektionsrahmen im Nachrichtenbereich strukturell prüfen, mit tatsächlicher Geometrie und Clipping; ein beliebiges Rahmenzeichen oder ein Ersatzhint reicht nicht. Belege: `app/agent_view/render.rs:1597–1607,1741–1745`, `scrollback/scrollback_pane.rs:1032–1055`, `scrollback/selection.rs:171–239`.
- Danach begrenzt per Home zum ursprünglichen Prompt zurückgehen, den tatsächlichen `go`-Prompt anklicken und dessen Auswahlrahmen prüfen; Right öffnet den kollabierten Turn. Vorhandene Bindings und Router: `actions/defaults.rs:143–149,277–284`, `app/dispatch/router.rs:571–575`, `scrollback/state/nav.rs:118–131`.
- Danach den sichtbaren `Message sent …`-Childlink selektieren und Enter senden; `app/agent_view/panes.rs:82–86` öffnet die reale Session.
- Child-Identitätsoracles bleiben erhalten: `[↑]`, `CHILD_DONE` und `/session-info` mit exakter CHILD_ID vor und nach Wiederöffnung, `finished_child_manual_compact_cancel.rs:65–80,116–128`.

Keine Timeoutsteigerung, kein Ignore und kein Erfolg allein aufgrund ausgebliebener Timeouts. Laufzeitprüfung bleibt GitHub-PTY-Abnahme. Die im Review referenzierte Governance-Checkliste war am angegebenen Pfad nicht verfügbar; dies ist eine dokumentierte Reviewgrenze.
