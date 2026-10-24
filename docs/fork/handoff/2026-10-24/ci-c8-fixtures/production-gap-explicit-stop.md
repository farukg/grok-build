# Zurückgezogener Produktionsverdacht: ExplicitStop

**Widerlegt durch den realen Klassifikationsowner; kein Produktionsblocker.** Rein statische Diagnose, keine Ausführung.

Der anfängliche Verdacht betraf die Kombination `ExplicitStop` + handgesetztes `CompletionReporting::Reportable`. Darunter würden `completion_event` (`shell/agent/subagent/completion_publication.rs:24–30`), das persistierte Angebot (`shell/session/storage/jsonl/completion.rs:93–98`) und `publish_child_completion` (`shell/agent/subagent/spawn.rs:511–529`) tatsächlich ein Wake erzeugen, obwohl die isolierte `will_wake_for`-Prüfung false liefert.

Der vollständige Aufrufervertrag widerlegt jedoch die angenommene produktive Erreichbarkeit: `tools/implementations/grok_build/task/coordinator.rs:1575–1582` klassifiziert `Reportable` ausschließlich bei `surface_completion && result.permits_completion_wake() && !explicitly_killed`. Der vorhandene semantische Owner `task/types.rs:573–594` verweigert ExplicitStop. Eine solche echte Completion ist deshalb **Silent**. Der Coordinator ist zugleich Attemptowner; ein manuell gesetzter Testattempt ist kein Ersatz.

Rootcause war die alte Fixture, die für alle vier Ergebniszustände pauschal Reportable konstruierte und den tatsächlichen Coordinator übersprang. Keine zusätzliche Produktionsklassifikation, Guard oder Default wäre hierfür gerechtfertigt.

Korrektur der Fixture am bestehenden Testowner:
- Die unveränderte Vier-Fall-Oracle `completion_errors_admit_parent_wake_but_explicit_stop_stays_silent` läuft nun über `ChannelBackend::spawn`, den realen `SubagentCoordinator`, dessen echte AttemptId und dessen echte `CompletionPreparation`/`CompletionReporting`.
- `tests/coordinator_fixture.rs` liefert ausschließlich den typisierten ChildRunOutput und verbindet die offiziellen ChildRunner-Callbacks mit den ausgelieferten Prepare-/Publishfunktionen und dem echten Persistenceactor. Kein Fake-Ack und keine private Kopie des Klassifizierers.
- Die historischen Wake-Erwartungen bleiben exakt true/true/true/false. Hinzu kommen die echte Silent-/Reportable-Entscheidung, tatsächliche Terminalmetadaten, Cause, Timestamp, typisierte persistierte Completionfact und coordinator-eigene Attemptbindung.
- Eine FIFO-Beobachtungsbarriere erfasst alle vorherigen Wake-Kommandos, auch im Null-Wake-Fall; kein Erfolg aufgrund einer ungepumpten asynchronen Queue.

Alle Pfade liegen unter `crates/codegen/xai-grok-shell/src/` bzw. `crates/codegen/xai-grok-tools/src/`. Produktivfunktionen wurden nicht verändert. Die frühere Zwischenmeldung ist hiermit ausdrücklich zurückgezogen; Kompilierung und Runtime bleiben unverified.
