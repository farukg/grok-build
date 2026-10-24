# Architect restart continuation — Quellclosure zur Lead-Abnahme

## Status

Die beauftragte Forward-/Recovery-Transaktion und ihre unabhängigen Regressionen sind im bestehenden Main-Arbeitsbaum fertig authoriert. **Nicht kompiliert, nicht getestet, nicht live abgenommen.** Kein eigener Commit, Push, Stage, Release oder Sessionneustart. Es wurde ausschließlich der erlaubte gepinnte Formatter angewendet; der letzte Formatierungslauf endete mit Exit 0, nicht mit einem Compiler-/Testbeleg.

### Exakte Sourceidentität

- Gelesener HEAD: `c8e075ea29ab74356cb477f05dcd70cdf7eb0e4c`.
- Finale 73-Dateien-Sourceidentität: `fdd27dc80a4f2803393037c13468952442d7e82b13bef45e48ea31e27151484f`. Beim Snapshot änderte sich keine erfasste Datei zwischen den beiden Byteaufnahmen.
- [Vollständiger Pfad-/Hashmanifest](/tmp/grok-goal-4f7553a1dbe6/implementer/delivery-shell/architect-restart-sourceidentity.json), [Symbolpositionen](/tmp/grok-goal-4f7553a1dbe6/implementer/delivery-shell/architect-restart-symbols.txt), [Sourcearchiv](/tmp/grok-goal-4f7553a1dbe6/implementer/delivery-shell/architect-restart-final-source.tar.gz), [Main-Delta zum aktuellen HEAD](/tmp/grok-goal-4f7553a1dbe6/implementer/delivery-shell/architect-restart-final-head.patch), [Statusaufnahme](/tmp/grok-goal-4f7553a1dbe6/implementer/delivery-shell/architect-restart-final-status.txt).
- Ausgangsbaseline bleibt HEAD `c256ab292b86ad6fb42d76d200c953257d34c862` plus [damalige Source-/Indexsicherung](/tmp/grok-goal-4f7553a1dbe6/implementer/delivery-shell/architect-restart-baseline/). Frühere korrekte Arbeit wurde nicht zurückgesetzt. Der finale Index wurde nur gelesen und als `architect-restart-final-index.patch` gesichert.
- `architect-restart-own.patch` ist Rekonstruktions-/Reviewevidence gegen den damaligen schmutzigen Ausgangsstand, **kein blind anzuwendender Exklusivautorenpatch**. Die Dateien enthalten zuvor richtige gemeinsame Hunks. Parent grenzt Commits mit dem gemeinsamen Publishowner ab; fremde Autorenschaft bleibt erhalten.

Die früheren Aussagen dieses Reports über noch fehlende `returned()`-/`suppress_seen()`-Migration, fehlende AgentTask-Preconditions und einen unformatierten Digestumbau sind überholt und ersetzt.

## Retained capabilities → tatsächliche Owner

Alle Produktpfade unten liegen unter `/home/fg/projects/org/forks/grok-build/`.

### 1. Attemptidentität und Terminalpublikation

- [Tools `task/coordinator.rs`](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator.rs), `handle_terminal_prepared` und `handle_terminal_visible`: der reale Coordinator-Attempt bleibt Identität von Commit, Ergebnisübergabe und späterer Sichtbarkeitsantwort. Fremde/späte Attempts können den aktuellen Zustand nicht übernehmen.
- [Tools `task/coordinator_state.rs`](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator_state.rs), `TerminalOutput::Publishing` und `CompletionBufferPublication`: eine noch unsichtbare Completion bleibt im bereits vorhandenen Terminalpublikationszustand. Erst erfolgreicher History-Publish macht den passiven Buffer sichtbar. Ein früheres echtes Projection-ACK oder Readerteardown retiert denselben Pending-Eintrag; eine spätere Visibilityantwort darf ihn nicht wiederbeleben.
- [Tools `coordinator/completion.rs`](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator/completion.rs), `prepare_buffered_completion`, `publish_buffered_completion`, `purge_completions_for_spawner`: Zielklassifikation und Outputcap bleiben am vorhandenen Bufferowner. Keine zusätzliche Resultablage.
- Der destruktive `SubagentEvent::Completions`-Arm ist gelöscht. Die verbliebenen Produktionsconsumer verwenden Peek plus explizites, parent- und attemptgebundenes `AcknowledgeCompletions`; `suppress_ids` trägt `SubagentCompletionId` statt Taskstrings. Kein Task-ID-only-Reportedledger für Childresults. Das normale Bash-/Workflow-Reporting bleibt bestehen.
- [Shell `agent/subagent/completion_publication.rs`](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/agent/subagent/completion_publication.rs), `prepare_completion_admission` → `publish_committed_completion`: tatsächlicher Producercommit und tatsächliche Veröffentlichung, einschließlich sichtbarem Rejected-/Deferred-Ausgang. Metadata-Reuse bleibt attemptgefenced im vorhandenen `metadata_store`.

### 2. Ein Input-/Receipt-Commitowner

Der reale Consumerpfad bleibt:

`SubagentResultSender` / `SubagentCompletionObservations` → Shell `commit_observed_subagent_tool_result` oder Wake-/Safe-Point-Admission → `ChatStateHandle::append_completion_and_ack` / `append_input_and_ack` → `ChatStateCommand::AppendInputAndAck` → `ChannelChatPersistence` → `PersistenceMsg::AppendInputAndAck` → `StorageAdapter` / `JsonlStorageAdapter`.

- [ChatState Completion-/Inputtypen](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-chat-state/src/input_projection.rs) und [ChatState Actor](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-chat-state/src/actor/mod.rs): vorhandener commit-aware `StrictAppendError`-/ACK-Vertrag wird wiederverwendet; Modellinput und Occurrencereceipt teilen denselben JSONL-Record.
- [Shell `storage/jsonl/completion.rs`](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/session/storage/jsonl/completion.rs), `append_completion_sync`: Retry übernimmt die tatsächlich gespeicherte Occurrence, nicht Prompt-ID oder ParentTask des Retryvorschlags. Ein Commitfehler vor Summarybuchhaltung wird beim nächsten Eintritt aus dem existierenden Chatrecord reconciliiert.
- `read_completion_state_sync` vereinigt dieselben gespeicherten Receipts mit dem vorhandenen Summaryzustand, ohne ein zweites Ledger anzulegen oder Beobachtung zur Ausführung zu machen.
- `InputAdmission` reicht mit ParentTask und CancellationToken bis zum Appendlock. Eine inzwischen geschlossene Task darf dort keine neue Projektion erwerben.
- Historyreplacement behält die tatsächlichen Receipts im bestehenden Summaryowner. Erfolgreiche exakte Parentterminals schließen Obligations; Callerfehler löschen sie nicht fälschlich.
- Eine Recoverycontinuation bindet sämtliche noch offenen Receipts desselben vorherigen ParentTask-Turns gemeinsam an die neue Instanz. Multiresult-/Digestgeschwister werden nicht als getrennte neue Inputs wieder ausgeführt. Die ursprünglichen Projectionreceipts bleiben unverändert.

### 3. Idle, aktive Safe Points, Toolresult und dynamischer Goal

- [Shell `completion_admission.rs`](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/session/acp_session_impl/completion_admission.rs), `commit_subagent_projection`, `drain_subagent_wakes_at_safe_point`, `drain_buffered_subagent_completions`, `commit_observed_subagent_tool_result`: ACK erst nach dem StrictAppend-Ergebnis; verlorene oder unklare Antworten bestätigen keinen ungesehenen Input.
- [Shell `completion_digest.rs`](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/session/acp_session_impl/completion_digest.rs), `project_subagent_wake_turn`: Idle-Digest bleibt erhalten, filtert vorher tatsächliche persistierte Occurrences und committed seine zusätzlichen Receipts zusammen mit dem einen Input. Fehlender Bufferinhalt nutzt weiterhin den echten Producertext.
- [Shell `completion_delivery.rs`](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/session/completion_delivery.rs), `CompletionPayload`, `result_reference`, `completion_reference_text`: Digest und aktive Bufferprojektion behalten die occurrencegebundene Referenz auf den vollständigen durablen Ausgang. Neuer Payload persistiert rohen Text und typisierte Referenz; kombinierter Modelltext wird auf Read abgeleitet. Ältere Payloads ohne Referenz behalten ihren alten Text ohne Heuristik.
- [Shell `interjection.rs`](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/session/acp_session_impl/interjection.rs) bindet Child-/Receiverdrain an die vorhandenen Sampler-/Toolübergänge. Kein laufendes Tool wird dafür unterbrochen und kein paralleler Callerturn gestartet.
- Goal ist kein Childresult-Suppressionsgrund. `goal_enabled` wurde **nicht** als Ersatz für dynamischen Zustand eingeführt. Der neue native Regressionseintritt startet das Child ohne Goal, beendet den gewöhnlichen Callerturn und aktiviert erst danach über `/goal` den wirklichen Parentreceiver; Erfolg und echter Fehler müssen anschließend dessen Sampler erreichen.
- Gewöhnliches Callerturnende beendet kein unabhängiges Child. Humanstop/-cancel, bestehende Wakebarriere, AutoWake-Konfiguration und Held/Auto-Receiptsettlement bleiben getrennte Owner. Scheduler verwendet die kanonische In-flight-Klassifikation auch während Terminalpublikation.

### 4. Native externe Humanreceipt — getrennt vom Childresult

Realer Pfad: ACP-Dispatch `x.ai/session/delivery` → [Extensionowner](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/extensions/session_delivery.rs) → `SessionCommand::ReceiveDelivery` → [Sessionreceiver](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/session/acp_session_impl/delivery_receiver.rs) → Persistence-FIFO → [JSONL-Receiverowner](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/session/storage/jsonl/delivery_receiver.rs).

Der Wirevertrag bleibt exakt [receiver-api.md](/tmp/grok-goal-4f7553a1dbe6/implementer/publish-timeout/receiver-api.md): camelCase-Parameter, `admit`/`observe`, exakte UTF-8-Payloaddigests, sessiongebundene Identität, `absent/admitted/conflict/rejected/indeterminate`, monotone Revision und unveränderte Progresszustände, `delivery_changed` als Hint. Standard-`session/prompt` bleibt turnterminal.

- Observe plant keinen Input und mutiert keine Receiptretention.
- Identischer Admit erwirbt keine zweite Queue-/Inputobligation. Konflikt hat keinen Schedulingeffekt.
- Prozessverlust nach Modellprojektion wird `interrupted`, nicht blind requeuefähig. Exakt gefencete spätere Terminale können die Unklarheit mit höherer Revision auflösen.
- [Receiverexecution](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/session/receiver_execution.rs), `bind_turn` / `finish_turn`, sowie `DeliveryReceiverState::settle_turn` erhalten ParentTask-/Origin-Fencing. Queued-Abschluss braucht exakte Delivery-ID, Digest und Queueturn; ein anderer Ursprung kann keine Humanreceipt abschließen.
- [Cancellation](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/session/acp_session_impl/cancel.rs) captured `InputTurnBinding` vor Taskteardown. `emit_turn_completed` erhält diese Bindung statt nach entferntem Task fälschlich Unknown zu erzeugen.
- Receiveridentität/Digesttombstones werden durch gewöhnliches Historyreplacement oder explizites Historydelete nicht still vergessen. Eine externe Humanreceipt wird niemals zu einem Childresult umklassifiziert.

### 5. Terminalmetadata und Featureconsumer

- Snapshotref-Primitive und Cancellation terminalisieren Metadata nicht vorab. [Reale Metadatafixtures](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/agent/subagent/tests/completion_fixtures.rs), `DurableCompletionFixture` und `completion_data_with_initial_meta`, beginnen mit Running und ohne Abschlusszeit. Cancellation, Producercommit und Publish verwenden dieselbe `ShellCompletionData` und denselben tatsächlichen Sessionpersistenzowner. Kein Fixture setzt den erwarteten Terminalstatus vorweg.
- Die bisherigen Completed-/Cancelled-/Failed-, Cause-/Timestamp-, Snapshotpointer- und Worktreeoracles bleiben nach dem tatsächlichen Commit-/Publishentrypoint erhalten. Zusätzlich bleibt die ursprüngliche Metadata vor Veröffentlichung bytegleich.
- Der zwischenzeitliche Rekonstruktionshelper `commit_result_metadata`, der die echte CompletionData hätte umgehen können, ist entfernt. Es bleibt nur der reale Producer-/Publishpfad.
- [Feature `isolated_spawn_e2e.rs`](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/agent/subagent/isolated_spawn_e2e.rs) importiert keinen nur `cfg(test)`-existierenden Presenter mehr. Sein bestehender Runner verwendet tatsächliche Prepare-/Publish-Acks und einen echten Parentpersistenzowner; keine bedingungslosen Erfolgs-Acks und kein cfg-Widening.
- Der Verbatim-/Formatierungsregressionstest schickt den tatsächlichen `SubagentCompletionWake` durch `run_session`. Ein FIFO-`GetCurrentPromptMode` dient als ereignisgebundene Barriere; anschließend wird das wirklich eingereihte `InputItem.verbatim` geprüft. Originale Text-, Truncation-, Schedulerhint- und Reihenfolgeoracles bleiben erhalten. Kein lokaler Nachbau des Receiverzweigs und kein Polling.

## Tests authored — sämtlich NOT RUN

1. [Strict JSONL-Recovery](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/session/storage/jsonl/completion_recovery_tests.rs): tatsächliche Occurrence bei verlorenem Commit-ACK; Retryprompt darf sie nicht überschreiben; neue Parentinstanz und spätes altes Terminal; geschlossene Admission; Receiverinterruption, Parentfence und kein Replay.
2. [Batchprojection](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/session/storage/jsonl/completion_batch_tests.rs): Multiresult mit zwei Receipts bleibt bei Commitfehler ein Input und bei Recovery eine gemeinsame Continuation; ursprüngliche Receipts bleiben erhalten, alte Parentterminale schließen die neue Instanz nicht.
3. [Coordinator-Projektionsregressionen](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator/completion_projection_tests.rs): gewöhnliche Remindercalls/Reservations und fremde Parent-/Attempt-ACKs konsumieren nichts; Buffer wartet auf Sichtbarkeit; frühes ACK/Teardown plus spätes Publish resurrecten nichts; abgelehnter oder fallengelassener Producer-ACK erzeugt keinen Phantomabschluss; echte Child-ID-Wiederverwendung erhält beide unterschiedlichen Attempts, und spätes altes ACK lässt den neuen bestehen.
4. [Native ACP-/Samplerregressionen](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/tests/test_completion_admission.rs): externe Humanadmission/-observe/-retry/-conflict/-fehler; aktive Sampling-/Toolgrenze; Cancel nach Taskteardown; foreground Childsuccess/-error als genau ein Toolresult; tatsächlicher Backgroundsuccess/-error bei idle, Sampling, Tool und Goal. Die Sampleroracle verlangt neben genau einem Ergebnismarker auch die occurrencegebundene Fullresultreferenz.
5. [Spätere dynamische Goalaktivierung](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/tests/test_completion_goal_activation.rs): Childcontext wird vor dem Goal gebaut und ausgeführt; anschließend aktiviert ein echter neuer Parentprompt `/goal`. Erfolg und echter Runtimefehler müssen dessen nächstes reales Sample erreichen. Kein Capability-/Selbstvergleich als Ersatzoracle.
6. [OS-Prozesscrashes](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/tests/test_admission_process_recovery.rs): verlorener Admission-ACK; Crash nach Receiverprojektion mit unreplayfähigem Interrupted; terminaler Receivercommit; Childsuccess/-error nach Preparation, Publication und Inputprojection. Worker bleibt am tatsächlichen Ereignis aktiv, statt vor der Projection-Crashgrenze zu enden.
7. Migrierte vorhandene Ownerregressionen: Actor-Idle-Digest/Goal/Error, reale Metadata-Commitgrenze, Worktree-Fate, Verbatimreceiver, usage-before-presentation sowie Quoten-/Incarnation-/Root-Replay. Alte manuell gesetzte `terminal_published`-Flags sind durch echte Prepare-/Visible-Callbackübergänge ersetzt. Bestehende Held/Auto-/Cancellation-Receiptdrainregressionen bleiben relevant und unverändert in ihrer Policy.

## Gelieferte Remote-Evidence und übrige Gates

Der bereits rote Run `40003673652` gehört zum früheren HEAD `c8e075ea…`, **nicht** zum obigen schmutzigen finalen Sourcehash. Seine Compilerdiagnosen wurden gelesen, nicht lokal reproduziert. [Die ursprüngliche Evidence](/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-format/failed-ci-readable.log) und [deduplizierte Shell-Rootdiagnosen](/tmp/grok-goal-4f7553a1dbe6/implementer/delivery-shell/architect-restart-c8-diagnostics.txt) bleiben erhalten.

Die dort sichtbaren fehlenden Notification-/Admission-/Taskoutputfixtures wurden aus der ursprünglichen Source rekonstruiert, nicht durch neue Erwartungen verdeckt. Stale CompletionData-/Cancellation-/Snapshotref-/Disposalsignaturen, fehlende Imports, Resultsender-/Observations-/Terminaloutputtypen und Feature-Presenterimport sind am aktuellen Mainowner reconciliiert. Die ungültige freistehende ToolRunResult-Feldzeile im Imagebridge-Test ist entfernt, das echte Initializerfeld bleibt. Korrekte vorhandene Source-/Formatterhunks wurden erhalten; kein Fullfilepatch des isolierten c8-Baums wurde über Main gelegt.

**Lead muss die tatsächliche Integrationsrevision prüfen:**

- Shell-/Tools-/ChatState-/Sampling-Types-/Feature-/Pager-Abhängigkeitskompilierung und die obigen scoped Tests ausschließlich über den koordinierten GitHubweg.
- Echte Crash-/Restart-/Ackverlust-, Held/Auto-/Humanstop-, Caller-Ende- und native Wireabnahme. Autorenschaft und Formatierung ersetzen sie nicht.
- Der separat erfasste ChatState-Pruning-Runtimegate und der PTY-Fokus-/Hintgate sind keine hier behaupteten Fixes. Keine Pageränderung, kein Timeout-/Ignore-/Oracle-Relaxing durch diesen Worker.
- Eventgebundene Receiptbeobachtung liest den vorhandenen Journal-/Summaryowner für reale Completionadmission. Kein Per-Frame-Work, Timer oder neuer Startupscan; zusätzliche Admissionlatenz bei großer History ist nicht gemessen und bleibt ein Runtimegate.
- Consumer nicht aufgrund dieses Reports als live `ReceiverExact` freigeben. Gemeinsame Veröffentlichung/CI/Stage bleibt beim Lead; keine parallele Publikationsfolge gestartet.

## Offene Entscheidung / Observations

Historische Interruptionsrecords ohne typisierte Ursache bleiben ungeklärt. Keine Fehlermeldungsheuristik, keine erfundene Humanintent-Klassifikation und keine neu gemintete historische Attemptidentität. Nur falls eine rückwirkende Migration gewünscht wird, muss Lead/Nutzer zwischen historischen Humanstop- und Runtimeerrorfolgen ausdrücklich entscheiden. Für den Forwardvertrag ist keine solche Entscheidung mehr nötig.

Keine belegte Aussage über Senkus historische Ursache, verlorene Providerantworten oder einen deployten Neustartfix. Laufende Sessions und getrennte Registry-/Origin-/SIG-/Pager-/Capture-/Pushpolicyowner wurden nicht bearbeitet.

Architekturquelle: [ADR 002](/home/fg/projects/org/forks/grok-build/context-modules.d/500_work.d/epics.d/product.d/grok-build/002-attempt-bound-input-admission.adr.md); Fortschritt: [W1-H](/home/fg/projects/org/forks/grok-build/docs/fork/streams/w1h-hold-deliver.md). `wakeup-final.md` neben diesem Report ist auf denselben Current-truth-Stand gebracht.
