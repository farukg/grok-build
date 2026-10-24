# CI-c8-Fixtures — Korrektur der unabhängigen Folgereviews

## Aktueller Stand

**Reviewbefunde im Code adressiert; fachliche Freigabe, Kompilierung und Runtime weiterhin offen.** 23 Sourcepfade: 18 vorhandene Dateien geändert, fünf neue Test-/Supportdateien. Keine Builds, Tests, Checks, cargo-Aufrufe, Commits, Publikation, Stage-/Releaseoperationen, Subagents oder Änderungen laufender Registry/Session. Nur erlaubte gepinnte Formatierung eigener Source und Metadaten-/Patchausgabe.

- Worktree: `/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format`.
- Branch: `sokrates/ci-c8-format`.
- Inputbaseline: `c8e075ea29ab74356cb477f05dcd70cdf7eb0e4c`.
- Unverändert erhaltener HEAD: `bb36f7bd58f057e0909759f0f10c54cf13ccc7ba`.
- Erhaltene Syntaxkorrektur: `4302b167a92acd77b76aeac16dbaa2a382ee3691`.
- Erhaltene Formatierung: `04121ef55075f91e6685b9ba5757c0615a3f4ab8`.
- Die vorherige 18-Datei-Reviewbasis samt damaligem Ergebnis, Patch und Hashes ist unter [review-baseline/](review-baseline/) unverändert gesichert. Ihre mechanischen grünen Format-/Hashchecks gelten **nicht** für diese neue Source.
- Der erste 12-Datei-Patch bleibt unter [continuation-baseline/prior-twelve-files.patch](continuation-baseline/prior-twelve-files.patch) erhalten.
- Aktive Main-Zustellungsquellen nicht übernommen oder editiert. Ausgelieferte Coordinator-, Producer-, Goal- und Lifecycle-Consumerfunktionen unverändert. Neue Module/Reexports sind ausschließlich Test-/`test-support`-Owners; keine Produktionsguards, zusätzliche Ledger oder Ersatzklassifizierer.

Eingang der Korrektur: [review-findings.md](review-findings.md). Alle unten genannten Oracles sind **autoriert, nicht ausgeführt oder unabhängig erneut freigegeben**.

## Befund → Rootcause → Owner → korrigierte Oracle

### 1. Replay: sichtbares Exactly-once nach dem gesamten tatsächlichen Replay

**Rootcause des früheren Nachweises:** `startup_admission_timeout_is_failed_not_cancelled` zählte rohe Gateway-Finishes vor dem zweiten Publish. Das war weder vollständige Replaybeobachtung noch die tatsächliche sichtbare Exactly-once-Wirkung.

Der bestehende Publisher ist auf Rohtransportebene at-least-once: `shell/agent/subagent/spawn.rs:503–509` sendet bei erneuter Publikation erneut; `storage/jsonl/completion.rs` verwendet denselben durablen Lifecycleevent und dessen EventId wieder. Die reale Pager-Lifecycle-/Attemptreduktion ist der idempotente Empfänger (`pager/app/acp_handler/subagent_lifecycle.rs`, `session_notification.rs`). Ein weiterer Producer-Ledger oder ein Sendeguard würde diese Zustellungs-/Recoverysemantik verändern und wurde nicht eingeführt.

**Neue zusammenhängende echte Strecke:**

1. Der bestehende Fixtureowner wurde unter `shell/agent/subagent/completion_test_support.rs` für Unit- und `test-support`-Konsumenten gemeinsam verbunden. Die bisherigen `tests/completion_fixture.rs` und `tests/coordinator_fixture.rs` sind weiterhin die einzigen Konstruktor-/Adapterimplementierungen, keine duplizierten Klassifizierer.
2. `completion_replay_for_e2e` treibt `ChannelBackend::spawn` → wirklicher `SubagentCoordinator` → dessen echte AttemptId/Reportingentscheidung → ausgelieferte Prepare-/Publishfunktionen → echter `SessionPersistence`-/Storageowner. Der Initialzustand ist Running, niemals ein vorab erwartetes Terminalmetadatum. Der Storageowner wird über seine vorhandene Explicit-Dir-API in einem Tempverzeichnis konstruiert; keine test-only Persistence-Implementierung wird in den Featurebuild hineingezogen.
3. Nach dem ersten bestätigten Publish und dem nativen Terminalresultat wird **wirklich erneut publiziert**. Erst danach wird ein Marker auf denselben Gateway-FIFO gelegt. Die zurückgegebene Nachrichtenspur enthält sämtliche vorherigen tatsächlichen Wiremeldungen, einschließlich der Wiederübertragung; kein Ende vor dem zweiten Publish.
4. Neu: `pager/src/app/acp_handler/tests/completion_replay.rs::native_completion_replay_has_one_visible_terminal_effect`. Diese Spur wird vollständig durch den realen `AppView`-ACP-Eingang `handle_ext_notification` geführt. Geprüft werden Completed, echte Startupdeadline-Fehlermeldung sowie ExplicitStop(Human). Die wiederholten Finishes müssen den Empfänger tatsächlich erreichen und dieselbe persistierte EventId tragen. **Nach sämtlichen Meldungen bleibt genau ein sichtbarer terminaler Subagentblock**, mit stabiler tatsächlicher EntryId, richtigem Completed/Failed/Cancelled-Kind, Error und beendetem Running-Flag. Ein eventuell separat vorhandener Background-Startedblock wird nicht fälschlich als weiterer Finish gezählt.
5. Der ursprüngliche Shell-Deadlinetest beobachtet nun ebenfalls beide Publikationen bis zum Gateway-FIFO-Marker (`tests/mod.rs:2627–2650`). Jeder rohe Finish behält Failed, die exakte Deadlineursache und dieselbe EventId. Mehr als eine rohe Meldung ist nur eine **Replay-Coveragevorbedingung**; die fachliche Erwartung wurde nicht blind auf zwei geändert. Das bisherige sichtbare „eins“ steht jetzt am tatsächlichen Pagerempfänger. Der Shelltest prüft unverändert genau einen gespeicherten Finish sowie unveränderte Metadaten nach dem vollständigen Replay.

Zusätzlich erhalten: `snapshot_ref_write_promotes_nonterminal_status_to_terminal`, `unproven_thread_exit_preserves_fresh_worktree`, `cancel_pending_shell_child_presents_one_cancelled_finish`, Failed statt Cancelled bei Startupdeadline, genaue Cause-/Timestamp-/Worktree-/Deduplizierungsoracles. Primitive Pointereffekte bleiben getrennt von der durablen Terminaltransaktion.

**Kein belegter Produktionsfehler erfordert hier einen Hunk.** Der anfängliche ExplicitStop-Verdacht bleibt ausdrücklich widerlegt: der wirkliche Coordinator klassifiziert ExplicitStop bereits zu Silent (`tools/.../task/coordinator.rs:1575–1582`). Die vierfache Wakeoracle bleibt true/true/true/false und verwendet die wirkliche Klassifikation und den wirklichen Attempt, nicht eine pauschale Reportable-Fixture. Details: [production-gap-explicit-stop.md](production-gap-explicit-stop.md).

### 2. Polling entfernt: tatsächliche Ereignisse, Receipts und FIFO-Acks

Owner: `shell/src/session/acp_session_tests/completion_persistence.rs`, `support.rs`, `agent/mvp_agent/tests/subagent_spawn_context_tests.rs` und `agent/subagent/tests/mod.rs`.

- Der Verbatimtest sendet nach dem typed Wake einen vorhandenen `GetToolOverrides`-Befehl auf denselben Parent-FIFO, wartet auf dessen echten Receiverreply und liest den tatsächlichen Queueeintrag **einmal**. `assert!(verbatim)` und alle Body-/Reihenfolge-/Schedulerhint-Oracles bleiben erhalten. Kein `yield_now`-/State-Waitloop.
- Der Goaltest verwendet denselben echten FIFO-Ack nach der gewöhnlichen Notification. Der berichtete Taskzustand wird danach einmal gelesen.
- Der vorhandene durabel delegierende Chatfixtureowner stellt jetzt typed **Beobachtungsereignisse** bereit: `MessageQueued` für den tatsächlichen `ChatPersistence::persist_message`-Aufruf und `InputAcknowledged` mit der ursprünglichen `InputProjection` und dem wirklichen `CompletionProjectionAck`. Er delegiert weiterhin an `ChannelChatPersistence`; er berechnet keine Geschäftsentscheidung und erfindet keinen Ack. Er sendet den originalen Ack zuerst an den wirklichen ChatStatecaller zurück und veröffentlicht erst dann die Beobachtung. Die anschließende ChatState-Query ist ein FIFO hinter diesem laufenden Handler.
- Goal-/Childtests warten mit `recv().await` auf die passenden tatsächlichen Write-/Receipt-Ereignisse statt mit `yield_now` plus wiederholten Zustandsqueries. Frische Childcompletion verlangt weiterhin Appended und genau einen tatsächlichen synthetischen Input sowie einen durablen attemptgebundenen Receipt.

Die Goal-Geschäftsinvariante bleibt: Context vor späterer Goalaktivierung gebaut, danach echte Active-/UserPaused-Übergänge am Parenttracker; aktuelle Active-Goals unterdrücken gewöhnliche Tasks, fehlende/pausierte Goals erlauben sie, echte Childresults bleiben auch im inzwischen aktiven Goal zulässig. Keine neue Arc-/Bool-Snapshot-API. Unberührte ältere Testhilfen außerhalb der beanstandeten Schleifen wurden nicht drive-by umgebaut.

### 3. Pruning: diskriminierender Retained-/Providerkontrast

Owner: `chat-state/src/actor/tests.rs::apply_turn_request_pruning_soft_trims_old_results_over_half_window`.

Die bisherige Over-Half-Oracle läuft nun für Providerusage **6001 und 4000**, jeweils mit fünf tatsächlich gespeicherten 8000-Byte-Toolresultaten und Window 10000. Die Retained-Estimate muss >5000 sein; die Providerusage wird über den echten Actor gespeichert und zurückgelesen.

Der zusätzliche 4000er-Fall ist der kohärente Unterschied zwischen einem vom Provider gesehenen bereits verkleinerten Request und weiter vollständig behaltener History: **Provider-Gate würde nicht prunen; Retained-Gate muss prunen.** Ein falscher Rückfall auf Providerusage scheitert damit am unveränderten verlangten Trimmarker und der kleineren Requestdarstellung. Ältestes gespeichertes Resultat bleibt 8000 Bytes, jüngstes Requestresultat ebenfalls 8000; zusätzliche Queries belegen, dass die gespeicherte History nicht verändert wurde. Keine Kopie von `should_prune` oder der Trimlogik.

Die frühere No-op-Fixture bleibt separat tatsächlich unter der halben Retained-Grenze (Window 32000, 5×8000 Payload, Provider 4000), mit unverändertem erwarteten Ergebnis 8000. Der neue Kontrast ergänzt diese Oracle, statt ihre Assertions oder Grenzwerte zu lockern.

### 4. PTY-Klick: erst echte Deselektion, dann neue Selektion

Owner: `pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs:149–236` und `send_subagent_message_selection.rs`.

- Tab und die bereits positiv reviewte neue strukturelle Rahmenoracle bleiben erhalten; begrenztes Home erreicht weiterhin den tatsächlichen go-Prompt.
- Vor dem go-Klick wird dessen **konkreter** Selektionsrahmen gesichert. Ein Mausdruck/-release auf den tatsächlichen Composer nutzt den vorhandenen Fokus-only-Pfad (`pager/src/app/mouse.rs:555–560`), nicht Space oder eine Draft-Prefill-Abkürzung.
- Abgewartet werden gleichzeitig: derselbe tatsächliche go-Prompt bleibt an der ermittelten Stelle sichtbar, sein konkreter Rahmen ist verschwunden, er besitzt keine Selektionsrahmung mehr. Ein ungültiger/leer decodierter Screen kann diese Bedingung nicht erfüllen. Der Composertext muss gegenüber vorher identisch bleiben: kein unbeabsichtigter Draft.
- **Erst danach** wird go angeklickt. Geprüft wird nun ein gegenüber diesem deselegierten Bild **neu erschienener** go-Rahmen. Die alte schon-vor-dem-Klick-wahre Bedingung wurde ersetzt. Composertext bleibt auch danach identisch.
- Right, der echte sichtbare Message-sent-Childlink, die unveränderte Caret-/Enter-Auswahl sowie sämtliche CHILD_DONE-/[↑]-/exakte CHILD_ID-/Session-Info-/Reopenoracles bleiben erhalten. `finished_child_manual_compact_cancel.rs` bleibt unverändert.
- Der Decoder unterstützt zusätzlich genau den belegten zweizeiligen Sticky-Pushfall **Prompt + unteres Padding, ohne nachfolgenden Gap** (`scrollback/sticky.rs:298–317`): Prefix zwei Zeilen vor dem Content und die tatsächlich leere untere Paddingzeile. Keine hypothetische allgemeine Headerausweitung.
- Ein gemeinsames bestehendes ROW_TIMEOUT-Budget von 60 Sekunden umfasst weiterhin den gesamten Fokushelfer. Keine Timeoutsteigerung und kein zusätzlicher Ignore.

## Bestehende Compilerbaseline und Scopegrenzen

Quelle bleibt ausschließlich der bereits beendete rote Run `40003673652`; keine neuen Logs oder Rawlogdumps. [diagnostics.md](diagnostics.md) / [diagnostics.json](diagnostics.json) enthalten die vollständige primäre deduplizierte Baseline: **69** Fundstellen = Shell 53 und Tools 16, einschließlich bereits separat reparierter Syntax. Wiederholte CI-Steps werden nicht erneut gezählt. Die übrigen 68 erfassten Fundstellen sind autoriert adressiert, **nicht neu kompiliert bestätigt**.

| Fehlerklasse → Rootcause | Owner → erhaltene Geschäftsoracles |
|---|---|
| E0425 → fehlende Notification-/Admission-/Taskoutput-Konstruktoren | Shell `acp_session_tests/support.rs` konstruiert reale Notifications, `TaskWakeAdmission`/`TaskWakeFallback`, typed Taskoutput-Fälle. ID aus `NotificationSource`. Suppression, Priorität, verlorener Ack/Fallback, Subagent-Ausnahme und konsumierte/nicht konsumierte IDs bleiben. Gatewayfall nutzt wirklichen Coordinator-/Gatewayfixturepfad. |
| E0061 → neue CompletionData-/Cancellation-/Snapshotref-/Disposal-/Backendsignaturen | Shell `subagent/tests/{mod,rest}.rs`, Tools-Backend-/Coordinatortests. Tatsächliche CompletionData/Initialartefakte, geteilte Cancellation-/Publishdaten, echte Resultsender/Observations. Usage-Ack, Outcomes, Worktree-/Snapshot- und Registration-oracles erhalten. |
| E0560 → veraltete Background-/Attemptfelder | Vorhandene AutoWakeInputs; tatsächlicher Completion-Attempt statt ShellCompletionData-Fakefeld. Panicoracle weiterhin exakt at1.face. |
| E0432 → privater cfg(test)-Presenter im Featurepfad | Bestehender `isolated_spawn_e2e` ruft echten Prepare-/Publishowner mit echten Acks. Gemeinsamer Completion-Testowner ist explizit cfg(test oder test-support), nicht ein cfg-Widening privater Presenterlogik. |
| E0308 → Resultsender/Terminaloutputs nicht mehr untypisiert | Tools ActiveMessage-/RootTargetsfixtures verwenden SubagentResultSender, CompletionObservations und TerminalOutput::Finishing. Queue-/Panic-/Fencing-/Root-Sentineloracles erhalten. |
| E0609/E0599 → Publikationsbool/-methoden ersetzt | Tools AgentTargets: existierender Before-Visibility-Checkpoint und wirkliche Commit-/Visibilitycallbacks. Koaleszieren, Sichtbarkeitsbarriere, Incarnation und genaue Pair-/Attemptquoten erhalten. |
| E0609 zweimal → entferntes Goal-Contextfeld | Tatsächlicher Parentreceiver mit späteren Goalübergängen und nun ereignisgetriebener Synchronisation; kein Capability-/Snapshotersatz. |

Pager hatte im alten Log keine zusätzliche eigene Source-Compilerdiagnose; sein Step exponierte den Shell-Featurefehler. Chat-State hatte einen Runtime-, keinen Compilerfehler; dessen Vorbedingung und zusätzliche diskriminierende Oracle stehen oben. Sampling-Types hatte im alten Log 302 bestandene Tests; keine Sampling-Types-Änderung und keine neue Verifikation.

Normale Produktionsfunktionen, laufende Registry/Session und aktives Hauptrepo bleiben unverändert. Besonders Coordinator/Stateowner, `completion_publication.rs`, `spawn.rs`, `metadata_store.rs`, JSONL-Completionowner, Goalowner sowie `session/completion_delivery.rs` und `agent/mvp_agent/session_setup.rs` wurden nicht symptomatisch geändert. Der Manifestabschnitt `protected_sources` hält ihre HEAD-/aktuellen Hashes fest. Kein zusätzlich belegter Produktions-/Policyblocker; der frühere ExplicitStop-Zwischenverdacht bleibt durch den echten Coordinator widerlegt.

## Getrennte Patches und Snapshotzuordnung

Nicht alle Pakete gleichzeitig anwenden: Gesamtpatch oder passende inkrementelle Basiskette wählen. Aktive Main-Dateien nur hunkweise integrieren, nie als ganze alte Dateien überschreiben.

| Artefakt | Basis / Inhalt | SHA256 |
|---|---|---|
| [review-baseline/fixtures.patch](review-baseline/fixtures.patch) | Der unabhängig reviewte alte 18-Datei-Snapshot, unverändert erhalten | `1e8186a49fd3f3caab253fb7a8085b6d03bfdf4be906459761c34d6e6382f89f` |
| [review-event-sync.patch](review-event-sync.patch) | Gegen Reviewbasis: Goal-/Receipt-/Write-Ereignissynchronisation | `60063d757b65d4f8772c0ee235164abd0ac7de06397fb01c69a772f185b0a8c8` |
| [review-replay.patch](review-replay.patch) | Gegen Reviewbasis: gemeinsamer Fixtureowner, vollständiges Replay, Verbatim-FIFO, echter AppView-Empfänger | `8b770eb67dd6e5d4c2c1acf893963abaa689b5e900cd7909de659254df763676` |
| [review-pruning.patch](review-pruning.patch) | Gegen Reviewbasis: Retained-/Providerkontrast | `eaa09b1b5c9bf73d0074990541c5fe0474eee8278becdcd346559c3a180b7bd4` |
| [review-pty.patch](review-pty.patch) | Gegen Reviewbasis: Deselektion/Klick/neuer Rahmen und zweizeiliger Pushheader | `5e0d000d2d963ae0e139213f2358968822cd03a694312c7abeafc44fdb6425cf` |
| [review-production.patch](review-production.patch) | Leer: keine ausgelieferten Produktionsfunktionen geändert | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| [review-corrections.patch](review-corrections.patch) | Gesamter Korrekturslice gegen Reviewbasis | `12860e30d54c778ce46cbd56232768fb45e4efefeeae6270b4e162deba485537` |
| [fixtures.patch](fixtures.patch) | Vollständige 23-Datei-Lieferung gegen erhaltenen HEAD bb36f7bd | `2f0e6e5fd15dad93a03906b0c03e3327ede0cfcc821042505251bf9e4a9627d1` |

Erster 12-Datei-Patch weiterhin SHA256 `33d0c8d252f18da03046aff401b3c9805d9ac90cbc330b718e4930f66c7cc090`. Aktualisierte `continuation-*`-Patches gegen diese frühere Basis, vollständige Input-/Artefakthashes und alle Sourcepfade stehen in [paths-and-hashes.md](paths-and-hashes.md). [source-manifest.json](source-manifest.json) enthält zusätzlich absolute Pfade, HEAD-, erste Folgeslice- und Reviewbasishashes. Generator [manifest.py](manifest.py) schreibt nur Scratchartefakte; keine Patchanwendungsprüfung ausgeführt.

## Unverified Gates — weiterhin Lead/CI

**Keine neue fachliche oder mechanische Freigabe behauptet.** Neue Source wurde nur mit nightly-2026-08-13 rustfmt ohne Check formatiert; für unberührte Nachbarbereiche wurden file-lines und vorhandene Skipattribute respektiert. Nicht ausgeführt: Builds, Checks, Tests, clippy, Patchanwendungscheck, UI-/PTY-Läufe, neuer GitHub-Run, Commit oder Publikation. Die alten mechanischen grünen Ergebnisse dürfen nicht auf diese Hashes übertragen werden.

Erforderliche unabhängige Folgeabnahme:

1. Review der vier `review-*`-Sachpatches, insbesondere der nachgewiesenen **at-least-once-Transport / exactly-once-sichtbare-Wirkung**-Grenze und des ack-erhaltenden Beobachteradapters.
2. Vorhandene GitHub-Format-/Compilegates einschließlich Shell ohne und mit test-support, Pager-Libtests und `native_completion_replay_has_one_visible_terminal_effect`.
3. Shell: Goal-Late-Activation/-Pause, Verbatimreceiver, vierfache Wakeoracle, alle Completed-/Cancelled-/Failed-/Snapshot-/Worktree-/Cause-/Timestamp-/Replayed-Finishoracles sowie bisherige Panic-/Usage-/Admissionregressionen. Keine neuen Pollingfences als Ersatz akzeptieren.
4. Chat-State: beide Pruningtests einschließlich **provider=4000 / retained>5000 / window=10000**; Tools-Backend-/Coordinator-/ActiveMessage-/AgentTargets-/RootTargetsgates und vorhandener Sampling-Types-Gate.
5. Die bestehenden PTY-Gates `send_subagent_message_row` und `finished_child_manual_compact_cancel_returns_to_reopenable_child` über das owning `pty_e2e_shell_tools`-Target. Konkretes Rahmenverschwinden/-neuerscheinen, unveränderter Draft und sämtliche Childidentitäts-/Session-Info-/Compact-/Cancel-/Reopenoracles müssen tatsächlich bestehen.

## Alle aktuellen Sourcehashes

Relative Pfade beziehen sich ausschließlich auf den oben genannten isolierten Worktree.

| Source | SHA256 |
|---|---|
| `crates/codegen/xai-chat-state/src/actor/tests.rs` | `71487ca07f98949304b6cd337349cc8f7a33b0eb104b9d4120703762b10bb3e4` |
| `crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs` | `9d9442df8e5ba10cef9c5e7360075236c3ce01f9762f177a802aaccd5339ad0a` |
| `crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_selection.rs` | `f61e9566504c751440f3dc18ac7555fb6bf1cf247ff43404521454380793319d` |
| `crates/codegen/xai-grok-pager/src/app/acp_handler/tests/completion_replay.rs` | `b70ef9b494f5d34c3134a12bbe0a3c8bd7f5c49cb4cf1be03b668afd85dab777` |
| `crates/codegen/xai-grok-pager/src/app/acp_handler/tests/mod.rs` | `7b9dbc28dca34c3ded08d290dfe0603191e4fab582b57fcebee7be348f00aec9` |
| `crates/codegen/xai-grok-shell/src/agent/mvp_agent/tests/subagent_spawn_context_tests.rs` | `9e65c937a93509a4ce1945f4feff8ea9fecff23d876c69d95c742d4537ea42e1` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/completion_test_support.rs` | `7c6b3b8007f62e9712feccc89b1a922a021d65bea498ea641e73c52d0f309ff3` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/isolated_spawn_e2e.rs` | `6a7dccfc370d258d4ffcfe9dc88ae96cd874ff0a458997550b0b9b521ad314b6` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/mod.rs` | `95601aa68ebef9030ed7f228ec95d53b57e518426f50d3a3b9037b9e7e953687` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/tests/completion_fixture.rs` | `a688233b34bd78b5b762f83be14039c8508637fb0b027493bd6dc4d8d1a85e75` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/tests/coordinator_fixture.rs` | `b969d5d2a40487d5e6fd11975752438b47d973df992faab79093a9686408599b` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/tests/mod.rs` | `5a9c98c2bce204f388d5795217cb2431ac656ec0c06ea4e5a2644cd19c3f78b0` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/tests/rest.rs` | `4322470710c7bc820a505541ff3c880f3bc499cfd4e5436ce42fc7eafba7fc90` |
| `crates/codegen/xai-grok-shell/src/agent/testkit/mod.rs` | `23619502f0c07cda7eed01d8efc03ac79b2298e0c394df0fffa693ebc0ba9c0f` |
| `crates/codegen/xai-grok-shell/src/session/acp_session.rs` | `1a8078722c230629658342365205b9420a410aee35d65a8edb2365546d539764` |
| `crates/codegen/xai-grok-shell/src/session/acp_session_tests/auto_wake_suppression_tests.rs` | `87be37ae0829d823401c09fc9d4496848416604f988f1f1089548922009b8507` |
| `crates/codegen/xai-grok-shell/src/session/acp_session_tests/completion_persistence.rs` | `350143d1eee4627a0f6c9ee9085150097b325f6447d578817e845b4924155e0c` |
| `crates/codegen/xai-grok-shell/src/session/acp_session_tests/support.rs` | `61a583a0accce3bdde84aef4caf62accf67c4c0048e62bbcf448356f7ea99fc8` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/backend_tests.rs` | `e44b19f78f0a6f6fb8404653a6c609f23e2ecc80ee6afae98867d67c3a658500` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator/active_message_tests.rs` | `f71f985adb155c9b6cd506974cbcfaf081f1e9f68da2066b2862afae602af04f` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator/agent_targets_tests.rs` | `be2464f541ca5d91eeaade8b01346c189e323fe2b7c57fde8dadad8224c9107c` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator/root_targets_tests.rs` | `60b70ff1dc9866522ff945e2d7c0403d947d90239bd54755d78cbf2bf50e97b2` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator_tests.rs` | `d702f6479012f4dbe61ff356c4298463b11ad90aba7469485a8717566388f8e4` |

Vollständige sieben Inputhashes stehen im Manifest und in der lesbaren Liste. Die beiden alten erfassten Logs bleiben bytegleich, SHA256 `947825dd02848b285644f571263f6bcd63b8a394960f8bf645e758de53baf523`.
