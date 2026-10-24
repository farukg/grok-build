# CI-c8-Fixtures — aktueller Author-only-Handoff

## Stand

**Goal-, Producer- und PTY-Fixturemigration autoriert; weiterhin vollständig unverified durch Compiler und Runtime.** Insgesamt 18 Sourcepfade: 15 vorhandene Dateien geändert, drei neue Testsupportdateien. Keine fachliche Erwartung abgeschwächt, kein Test gelöscht oder zusätzlich ignoriert. Produktivfunktionen bleiben unverändert; kein zusätzlicher Produktionsguard, Default, Goal-Snapshot oder privater Ersatzklassifizierer.

- Ausschließlicher Worktree: `/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format`.
- Branch: `sokrates/ci-c8-format`.
- Baseline: `c8e075ea29ab74356cb477f05dcd70cdf7eb0e4c`.
- Erhaltener HEAD: `bb36f7bd58f057e0909759f0f10c54cf13ccc7ba`.
- Erhaltene Syntaxkorrektur: `4302b167a92acd77b76aeac16dbaa2a382ee3691`.
- Erhaltene Formatierung: `04121ef55075f91e6685b9ba5757c0615a3f4ab8`.
- Keine Builds, Tests, Checks, cargo-, Commit-, Rebase-, Reset-, Push-, Stage- oder Releaseoperationen. Keine Änderung an laufender Registry/Session; aktive Hauptrepo-Quellen weder editiert noch kopiert.
- Einziger Rustprozess: ausdrücklich erlaubter gepinnter rustfmt ohne `--check`, Edition 2024, vorhandene Config, `skip_children=true`. Für die Goal- und PTY-Owners wurden nur die editierten Bereiche mit `--unstable-features --file-lines` formatiert; bestehende rustfmt-Skipattribute bleiben erhalten.

Der vorherige 12-Datei-Handoff ist bytegetreu unter [continuation-baseline/result.md](continuation-baseline/result.md) erhalten. Seine damals offenen Goal-/Metadaten-/PTY-Contractfragen werden durch die unten beschriebenen realen Einstiegspfade ersetzt, nicht durch geänderte Erwartungswerte.

## Neue Folgeslices und unveränderte Oracles

### Goal: tatsächlicher Parentreceiver statt entferntem Contextfeld

Owner: `shell/src/agent/mvp_agent/tests/subagent_spawn_context_tests.rs:162–271`; bestehender Receiver-Fixtureowner `session/acp_session_tests/support.rs:1023`, erweitert über `completion_persistence.rs:61`.

Die bestehende `subagent_spawn_context_shares_parent_goal_loop_gate`-Oracle baut weiterhin **zuerst** den echten Mvp-Spawncontext. Dessen übernommener Parentkanal ist mit einem realen `run_session` und echtem Chat-/Persistenceowner verbunden. Erst anschließend entstehen bzw. pausieren Goals durch `GoalTracker::create_goal` und `GoalTracker::pause(GoalPauseReason::User)`. Die Fälle verwenden die wirklichen `GoalStatus`-Werte, keine neuen Bool-/Stringzustände.

Beobachtete Geschäftsoracles:

- Ohne Goal und nach echter Pause wird dieselbe gewöhnliche Taskcompletion genau einmal als Input aufgenommen.
- Bei **nach dem Contextbau** aktiviertem Parentgoal wird die gewöhnliche Taskcompletion vom wirklichen Notificationdrain abgearbeitet/als berichtet markiert, ohne Input oder neuen Turn zu erzeugen.
- Über denselben vorher gebauten Childpfad bleibt eine typisierte Subagentcompletion unter dem nun aktiven Goal zulässig: exakt ein tatsächlicher Synthetic-Userinput und exakt ein durabler attemptgebundener Receipt.

Sourcegrundlage: `session/acp_session_impl/run_loop.rs:1036–1100,725–733`, `notification_drain.rs:516–600`, `goal_support.rs:1037–1042`. Der vorhandene Receiver liest den aktuellen Parenttracker beim Drain; Childresults haben den beauftragten separaten Completionpfad. Eine neue oder unbenutzte Arc-API und ein statischer `goal_enabled`-Ersatz sind nicht nötig. Die beiden erfassten E0609-Zugriffe auf das entfernte private Feld sind migriert. Kein Goal-Produktionshunk.

### Producer: Primitiveffekt und öffentliche Terminaltransaktion getrennt

Neue reine Konstruktorowner:

- `agent/subagent/tests/completion_fixture.rs`: initiale **Running**-Artefakte, wirklicher `SessionPersistence`-Actor, echter StorageAdapter und Parenttransport. Publish-Nachrichten samt originalem Replysender gehen unverändert an diesen Owner; keine erfundenen Acks. Andere Parentkommandos werden zur Beobachtung weitergereicht.
- `agent/subagent/tests/coordinator_fixture.rs`: offizieller `ChannelBackend::spawn`/`SubagentCoordinator`-Pfad. Der Fixture-Runner liefert nur den typisierten ChildRunOutput. AttemptId, CompletionReporting und CompletionPreparation stammen vom echten Coordinator. Offizielle Hostcallbacks rufen die ausgelieferten Prepare-/Publishfunktionen auf.

Die Geschäftsoracles laufen nun durch `prepare_completion_admission` → tatsächliche durabel bestätigte Producertransaktion → `publish_committed_completion` → echte Metadaten-/Lifecyclepublikation:

| Bestehende Oracle | Jetzt geprüfte tatsächliche Grenze; erhaltene Erwartungen |
|---|---|
| `snapshot_ref_write_promotes_nonterminal_status_to_terminal` (`tests/rest.rs:510–590`) | Der Snapshotref-Write wird separat auf exakt diesen Pointereffekt geprüft: sämtliche übrigen initialen Felder unverändert. Nach Prepare sind die externen Metadaten ebenfalls unverändert. Erst nach wirklichem Publish weiterhin **Completed**, derselbe Snapshotref und Worktreepointer, Timestamp, kein Error, richtige Counts. Zweites Publish erhält sämtliche Metadaten und genau einen persistierten Finish. Die alte Completed-Assertion bleibt; sie wurde nicht auf Running umgeschrieben. |
| `unproven_thread_exit_preserves_fresh_worktree` (`tests/mod.rs:2497 ff.`) | Cancel-/Shutdownkommandos, Interrupted-Ergebnis und Worktreeerhalt bleiben unmittelbar geprüft. Initiale Metadaten bleiben bis Publish unverändert. Danach weiterhin **Cancelled**, vorhandener Timestamp und exakt `Subagent was cancelled`. Worktree bleibt auch nach Commit/Publish erhalten. Replay ändert weder Metadaten noch den einmaligen persistierten Finish. |
| `startup_admission_timeout_is_failed_not_cancelled` (`tests/mod.rs:2587 ff.`) | Weiterhin nicht Interrupted, nicht Success, Status **failed**, exakt `Subagent initial prompt was not admitted before the deadline`. Nach der realen Transaktion **Failed**-Metadaten, derselbe Error, Timestamp, genau ein wirklicher persistierter und ein Live-Finish. Wiederpublikation erhält alle Metadaten und einen Historyfinish. |
| `cancel_pending_shell_child_presents_one_cancelled_finish` (`tests/mod.rs:2321 ff.`) | Die frühere private Presenterabkürzung ist auch hier entfernt. Native Cancellation bleibt Interrupted/unsuccessful; wirklicher Producer-/Publishowner erzeugt weiterhin genau einen persistierten und einen Live-Cancelled-Finish. |
| `completion_errors_admit_parent_wake_but_explicit_stop_stays_silent` (`tests/mod.rs:694 ff.`) | Unveränderte vier Zustände und Wake-Wahrheitstabelle **true / true / true / false**. Der wirkliche Coordinator klassifiziert und mintet den Attempt; der wirkliche Commit-/Publishowner materialisiert Status, Cause und Timestamp. Die persistierte typisierte Completionfact behält exakt Zustand und wirklichen Attempt. Parent-Wakes werden bis zu einer FIFO-Beobachtungsbarriere gezählt, sodass null Wakes nicht durch eine ungepumpt asynchrone Queue vorgetäuscht werden können. |

Keine erwarteten Terminalmetadaten werden vorab in eine Fixture geschrieben. Der Initialartefaktkonstruktor verlangt Running; die Terminaldaten entstehen ausschließlich durch den vorhandenen Producerowner.

#### Zwischenverdacht ExplicitStop ausdrücklich zurückgezogen

Beim Durchverbinden des realen Publishpfads fiel die alte handkonstruierte Kombination `ExplicitStop + Reportable` auf. Die zuerst gemeldete Produktionslücke war ein **Fixturedefekt**, kein belegter Produktfehler: der vollständige Coordinator-Aufrufervertrag (`tools/.../task/coordinator.rs:1575–1582`) gibt für ExplicitStop bereits `Silent`, über `result.permits_completion_wake()` aus dem vorhandenen typisierten Stateowner. Die Fixture darf nicht erneut für alle Zustände Reportable festsetzen oder diesen Klassifizierer privat nachbauen.

Daher nimmt die Matrix ihre Disposition und AttemptId jetzt tatsächlich vom Coordinator. Null Wakes für ExplicitStop bleiben verpflichtend, nicht umdefiniert. Kein Produktionsguard wurde geschrieben. Die vollständige Korrektur des Zwischenbefunds steht in [production-gap-explicit-stop.md](production-gap-explicit-stop.md); die frühere Zwischenmeldung ist zurückgezogen. **Aktuell kein konkreter unauflösbarer Produktions-/Policyblocker aus diesem Folgeslice.** Das ist keine Laufzeit- oder Buildfreigabe.

### PTY: struktureller Fokus und reale Navigation

Eingang: [pty-focus-review.md](pty-focus-review.md), rein statischer unabhängiger Review. Implementierung ausschließlich im Testhelper `pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs:149–203` und neuem `send_subagent_message_selection.rs`.

- Tab bleibt erhalten. Das Bild davor wird mit dem danach verglichen; gefordert ist **genau ein neu hinzugekommener vollständiger struktureller Selektionsrahmen im Nachrichtenbereich**, kein beliebiges Rahmenzeichen und kein Ersatzhint.
- Der Decoder arbeitet mit Terminalspalten einschließlich Unicode-Breiten, den festen Fixturemaßen 120×50, den tatsächlichen Default-Selektionsspalten und der strukturell identifizierten Composergrenze. Er prüft zusammengehörige obere/untere Ecken, durchgehende gepaarte Seiten, nichtleeren Nachrichteninhalt sowie die dargestellten Clippingkanten (`┆` nur an den zulässigen Randzeilen, mit passenden Viewport-/Stickyheadergrenzen). Composerrahmen oder einzelne dekorative Glyphen reichen nicht.
- Drei begrenzte Home-Eingaben erreichen im festen Elternskript den tatsächlichen ursprünglichen `go`-Prompt. Gefordert sind dessen exakter Prefix/Text und ein ihn geometrisch einschließender Selektionsrahmen. Dann wird genau dieser Prompt angeklickt und der Rahmen erneut geprüft. Right öffnet dessen Turn; erst der wirkliche sichtbare `Message sent …`-Childlink beendet diese Vorbedingung.
- Die vorhandenen `select_row`-/Caret-/Enter-Operationen bleiben unverändert. **CHILD_DONE, `[↑]`, exakte CHILD_ID sowie Session-Info vor/nach Wiederöffnung bleiben unverändert**; `finished_child_manual_compact_cancel.rs` ist bytegleich zum erhaltenen HEAD.
- Kein neuer Ignore, keine Timeoutsteigerung. Der gesamte neue Fokushilfsablauf teilt **einen** vorhandenen `ROW_TIMEOUT`-Deadlinebudget von 60 Sekunden.

Sourcebezüge: `pager/src/scrollback/layout.rs:75–85`, `scrollback/selection.rs:171–239`, `scrollback/scrollback_pane.rs:1032–1108`, `app/agent_view/render.rs:1597–1607,1741–1745`, `views/agent.rs`/`pager-render/src/appearance/config.rs` für die feste Defaultgeometrie; die Home-/Right-/Childlinkbindings stehen im eingegangenen Review. Der Decoder ist an diese feste, isolierte Fixture gebunden; andere Terminalgrößen werden nicht still als Erfolg akzeptiert. Kein Produktionsfokusbug angenommen oder repariert. **PTY-Runtimeabnahme weiterhin offen.**

## Gesamte deduplizierte Compilerbaseline und bisheriger Patch

Nur bestehende Evidence: abgeschlossener roter Run `40003673652`. Keine neuen Logs/Tests oder Rawlogdumps. File-first Sanitizer verarbeitet echte ANSI-Escapes und literales `^[[...`; dedupliziert wird nach Code/Meldung/primärer Sourceposition, nicht nach erneutem CI-Step.

**69 eindeutige Source-Diagnosen**: Shell 53 (52 libtest einschließlich Syntax, eine Feature-/Pager-Abhängigkeitsdiagnose), Tools 16. E0425 36, E0061 15, E0609 9, E0560 2, E0308 2, E0599 3, E0432 1, Syntax 1. Syntax bereits separat in 4302 repariert; die übrigen 68 erfassten Fundstellen sind durch die autorierte Migration adressiert. Keine bewusst unadressierte erfasste Compilerfundstelle bleibt. **Kein Compilerlauf und keine Behauptung, dass die aktuelle Source bereits kompiliert.** Vollständiger Index: [diagnostics.md](diagnostics.md), [diagnostics.json](diagnostics.json).

| Fehlerklasse → Rootcause | Bestehender Owner / erhaltene Oracle |
|---|---|
| E0425 → fehlende Notification-/Admission-/Taskoutput-Fixtures | `acp_session_tests/support.rs` rekonstruiert tatsächliche Notifications, `TaskWakeAdmission`/`TaskWakeFallback` und typisierte Taskoutput-Fälle. Task-ID stammt aus `NotificationSource`. Suppression, Priorität, Fallback vor Ablehnung bei verlorenem Ack, Subagent-Ausnahme sowie konsumierte/nicht konsumierte Completion-IDs bleiben geprüft. Der frühere fehlende `test_gateway`-Aufruf ist durch den realen Coordinator-/Gatewayfixturepfad ersetzt. |
| E0061 → neue CompletionData-/Cancellation-/Snapshotref-/Disposal-/Backendsignaturen | `agent/subagent/tests/{mod,rest}.rs`, Tools-Backend-/Coordinatortests. Actual CompletionData wird über Cancellation und Publikation geteilt; Initialartefakte besitzen echte Completion-/Attempttypen. Usage-Ack, fachliche Outcomes, Worktree-/Snapshotinvarianten und Registration vor Resultat bleiben. |
| E0560 → veraltete Background-/Attemptfelder | AutoWakeInputs entsprechen dem vorhandenen Gateowner; Attempt gehört zur echten Completion, nicht zur ShellCompletionData. Panicoracle erwartet weiterhin exakt `at1.face`. |
| E0432 → nur cfg(test)-Presenter im Featurefixture | Bestehender `isolated_spawn_e2e.rs` verwendet weiterhin echte Producer-/Publishfunktionen mit echtem Persistenceowner und tatsächlichen Commit-/Visibility-Ergebnissen. Kein cfg-Widening eines privaten Presenters und kein gefälschter Erfolgsack. |
| E0308 → untypisierte Resultsender/Terminaloutputs | Tools `active_message_tests.rs` und `root_targets_tests.rs`: wirklicher `SubagentResultSender`, explizite CompletionObservations, `TerminalOutput::Finishing` für den vorhandenen Sentinel. Queue-/Panic-/Fencing-/Root-Routingoracles unverändert. |
| E0609/E0599 → Bool-Publikation durch Zweiphasenvertrag ersetzt | Tools `agent_targets_tests.rs`: bestehender Before-Visibility-Fixturecheckpoint und wirkliche Commit-/Visibility-Callbacks statt manuell gesetzter Published-Flags. Koaleszierte Wakes, Sichtbarkeitsbarriere, Incarnation und exakte Pair-/Attemptquoten bleiben erhalten. |
| E0609 zweimal → entfernter Goal-Structzugriff | Jetzt echter Parentreceiver/Lifecyclepfad wie oben; kein Architektur-Fakefeld. |

Weitere tatsächliche Logergebnisse bleiben getrennt von Compilerfehlern:

- Pager hatte keine zusätzliche eigene Source-Compilerdiagnose; dessen Step exponierte die Shell-Featurefixture E0432 und wird nicht doppelt gezählt.
- Chat-State hatte den echten Runtimefehler `apply_turn_request_pruning_is_noop_under_half_window` (3019 statt 8000). Die unveränderte erste Lieferung setzt die Fixture auf einen tatsächlich unterhalb der halben Retained-Estimate-Windowgrenze liegenden Fall: Window 32000 statt 10000, zusätzliche Vorbedingung Estimate <16000. Payload 5×8000 Bytes, Turns, eingeschaltetes Pruning und erwartete 8000 unverändert. Providerusage 4000 war unter dem vorhandenen `request_builder.rs:122–145`-Vertrag keine passende Retained-Estimate-Vorbedingung.
- Sampling-Types hatte im erfassten Log 302 bestandene Tests und keine Source-Compilerfehler. Keine Sampling-Types-Datei geändert; das ist historische Evidence, keine neue Verifikation.
- Der erfasste PTY-Timeout an `Space:prompt` wird jetzt durch die oben beschriebene tatsächliche Fokus-/Navigationsoracle migriert, nicht als Kompilierfehler gezählt.

## Reviewbare Lieferteile

Alle neuen Folgeslice-Patches sind gegen die separat gesicherte erste 12-Datei-Lieferung erstellt; unverändert vorliegende Dateien benutzen den erhaltenen HEAD als Basis. Keine ganzen Altdateien über aktive Main-Autoren kopieren, sondern gezielte Hunks integrieren.

| Artefakt | Bedeutung | SHA256 |
|---|---|---|
| [continuation-baseline/prior-twelve-files.patch](continuation-baseline/prior-twelve-files.patch) | Erste Lieferung unverändert erhalten | `33d0c8d252f18da03046aff401b3c9805d9ac90cbc330b718e4930f66c7cc090` |
| [continuation-goal.patch](continuation-goal.patch) | Goal-Oracles und realer Receiver-/Persistencekonstruktor | `34a2e8e09f1ccd4e957160020a84466d187a93ca6b3eb1878905810a7940ffeb` |
| [continuation-producer-fixtures.patch](continuation-producer-fixtures.patch) | Reale Producer-/Coordinatorfixtures und Metadaten-Oracles | `0c24797e5f34188c92b6ab59e20918a8f02849c7ec7fa637e50a42c5849c77be` |
| [continuation-pty.patch](continuation-pty.patch) | Nur PTY-Testhelper und strukturelle Bildschirmoracle | `b94961a16903e27473dbad166f5e8021b93b68cf5a209efa7b7660ebc3653204` |
| [continuation-production.patch](continuation-production.patch) | Absichtlich leer: keine Produktionshunks erforderlich | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| [continuation-fixtures.patch](continuation-fixtures.patch) | Kombinierter Folgeslice über neun Sourcepfade | `fc67966a3687d26e9fdf43c939a36ba6e1ef904b9d0d758d2b5f404b9f357238` |
| [fixtures.patch](fixtures.patch) | Gesamte Lieferung über 18 Pfade gegen HEAD bb36f7bd | `1e8186a49fd3f3caab253fb7a8085b6d03bfdf4be906459761c34d6e6382f89f` |

[source-manifest.json](source-manifest.json) enthält sämtliche absoluten Pfade, HEAD-Hashes, Folgeslice-Basishashes, aktuellen Sourcehashes, Inputhashes, erhaltenen Revisionshashes und die unveränderten geschützten Produktivowner sowie den unveränderten Child-Identitätstest. [paths-and-hashes.md](paths-and-hashes.md) ist die lesbare vollständige Liste. Artefakte wurden allein durch den file-first Metadatengenerator [manifest.py](manifest.py) geschrieben; kein Patchanwendungstest ausgeführt.

## Unverified Gates — Lead/CI

Nicht ausgeführt: Formatcheck, Compiler, Tests, Clippy, Patchanwendungscheck, unabhängiger Code-Review der neuen Hunks, Visual-/PTY-Lauf, neuer GitHub-Run oder Publikation.

Erforderlich auf der tatsächlichen Integrationsrevision:

1. Vorhandene `linux-test.yml`-Format-/Compilegates, Shell-Libtest-Kompilierung und Shell-Feature-/Pager-Kompilierung einschließlich `isolated_spawn_e2e`.
2. Shell-Filter `subagent_spawn_context_shares_parent_goal_loop_gate`, `completion_errors_admit_parent_wake_but_explicit_stop_stays_silent`, `snapshot_ref_write_promotes_nonterminal_status_to_terminal`, `unproven_thread_exit_preserves_fresh_worktree`, `startup_admission_timeout_is_failed_not_cancelled`, `cancel_pending_shell_child_presents_one_cancelled_finish`; zusätzlich die bisherigen Subagent-/AutoWake-/Verbatim-/Panicregressionen.
3. Tools-Backend-/Coordinatortests einschließlich ActiveMessage-, AgentTargets- und RootTargetsoracles.
4. Chat-State `apply_turn_request_pruning_is_noop_under_half_window` sowie vorhandener Sampling-Types-Gate.
5. Unabhängiger statischer Review des streng auf die 120×50-Fixture gebundenen PTY-Decoders; danach beide vorhandenen PTY-Gates `send_subagent_message_row` und `finished_child_manual_compact_cancel_returns_to_reopenable_child` im owning `pty_e2e_shell_tools`-Target mit ihren bestehenden Ignore-/Explizitlaufkonventionen. Vollständige Childidentität und Compact-/Cancel-/Reopenoracles bleiben Pflicht.

Kein Abschluss-/Grün-/Stage- oder Releaseclaim vor diesen Ergebnissen.

## Alle Sourcepfade und aktuellen SHA256

Relativ zum oben genannten isolierten Worktree; vollständige absolute Pfade und Basishashes stehen im Manifest.

| Source | SHA256 |
|---|---|
| `crates/codegen/xai-chat-state/src/actor/tests.rs` | `2eeebf8823c2b57868fe7ea5a0b8460c80a0be554d696f27abd09e4bc7bb7f22` |
| `crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs` | `b0d24d6ada734a05410c9cddce797db99cd287305a0ac0864743064a7251bf4b` |
| `crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_selection.rs` | `a611a88ad392c1bac47017bb415b6dc49d5f434a36bfdc9276db80f5bf3ba1cb` |
| `crates/codegen/xai-grok-shell/src/agent/mvp_agent/tests/subagent_spawn_context_tests.rs` | `d61a9460dca004bf62b0c0ae376ed1ac7812a4726c91a0344b3143c902b1aea2` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/isolated_spawn_e2e.rs` | `cf4dd5fddf7d15925ebba0a48de6c5107a572e26f2ffc910bb017738524dfcb0` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/tests/completion_fixture.rs` | `f9adbb05f623d9048a908266e5743194a85311ad2e9f153c368c239a8f1b62df` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/tests/coordinator_fixture.rs` | `96ca1acb9ec708cc64b731b2f08951c4ba11a4889b8128fbb69b7e05f7817c81` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/tests/mod.rs` | `cc88f6d89735f73df1986eabbe42f32f8ee31dab030fc7c724cd79ebfa3453bb` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/tests/rest.rs` | `4322470710c7bc820a505541ff3c880f3bc499cfd4e5436ce42fc7eafba7fc90` |
| `crates/codegen/xai-grok-shell/src/session/acp_session.rs` | `1a8078722c230629658342365205b9420a410aee35d65a8edb2365546d539764` |
| `crates/codegen/xai-grok-shell/src/session/acp_session_tests/auto_wake_suppression_tests.rs` | `87be37ae0829d823401c09fc9d4496848416604f988f1f1089548922009b8507` |
| `crates/codegen/xai-grok-shell/src/session/acp_session_tests/completion_persistence.rs` | `012be7da8e8ce903e29e579238b9e98def46c2ded7cdd151d8ca9eddc0afb449` |
| `crates/codegen/xai-grok-shell/src/session/acp_session_tests/support.rs` | `f4fa5eda9ad365009a3c8845022434b725e1d797312875720de6d235abe344b4` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/backend_tests.rs` | `e44b19f78f0a6f6fb8404653a6c609f23e2ecc80ee6afae98867d67c3a658500` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator/active_message_tests.rs` | `f71f985adb155c9b6cd506974cbcfaf081f1e9f68da2066b2862afae602af04f` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator/agent_targets_tests.rs` | `be2464f541ca5d91eeaade8b01346c189e323fe2b7c57fde8dadad8224c9107c` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator/root_targets_tests.rs` | `60b70ff1dc9866522ff945e2d7c0403d947d90239bd54755d78cbf2bf50e97b2` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator_tests.rs` | `d702f6479012f4dbe61ff356c4298463b11ad90aba7469485a8717566388f8e4` |

Alle sechs Inputhashes und alle Artefakthashes stehen zusätzlich vollständig in [paths-and-hashes.md](paths-and-hashes.md). Die beiden erfassten alten CI-Logs bleiben bytegleich: SHA256 `947825dd02848b285644f571263f6bcd63b8a394960f8bf645e758de53baf523`.
