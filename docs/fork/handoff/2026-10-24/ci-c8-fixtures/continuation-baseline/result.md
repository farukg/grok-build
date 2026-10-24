# CI-c8-Fixtures — Author-only-Handoff

## Ergebnis und Grenzen

**Autoriert, nicht vollständig entblockt, nicht kompiliert oder getestet.** 12 Dateien sind uncommittet geändert. 66 deduplizierte Compilerfundstellen sind durch Fixture-/Consumer-Migration adressiert; zwei E0609-Fundstellen bleiben absichtlich unverändert, weil der vorhandene Produktvertrag die historische Goal-Sharing-Oracle nicht ausdrücken kann. Dazu bestehen drei statisch sichtbare Konflikte historischer Metadaten-Oracles mit der neuen Producer-Commitgrenze. Keine Assertion wurde gelöscht, abgeschwächt oder ignoriert.

- Worktree: `/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format`.
- Branch: `sokrates/ci-c8-format`.
- Inputbaseline: `c8e075ea29ab74356cb477f05dcd70cdf7eb0e4c`.
- Gelesener und unverändert erhaltener HEAD: `bb36f7bd58f057e0909759f0f10c54cf13ccc7ba`.
- Erhaltene Syntaxkorrektur: `4302b167a92acd77b76aeac16dbaa2a382ee3691`.
- Erhaltene Formatierung: `04121ef55075f91e6685b9ba5757c0615a3f4ab8`.
- Keine Commits, Rebases, Resets, Pushes, Staging-/Releaseoperationen, Builds, Checks oder Tests. Kein cargo. Keine Operation an laufender Registry oder Session. Das aktive Hauptrepo wurde weder editiert noch als Quelle für Arbeitsdateien verwendet.
- Normale Produktivfunktionen bleiben unverändert. `session/acp_session.rs` ändert ausschließlich die Sichtbarkeit des bereits `cfg(test)`-gebundenen Supportmoduls. `isolated_spawn_e2e.rs` ist der bestehende ausschließlich über `test-support` eingebundene Fixtureowner.

## Vollständige erfasste Compilerbaseline

Quelle ist ausschließlich der bereits abgeschlossene rote Run `40003673652`, nicht ein neuer Build. Der file-first Sanitizer entfernt echte ANSI-Sequenzen und literales `^[[...`; der anschließende Index dedupliziert nach Fehlercode, Meldung und primärer Sourceposition. Wiederholte CI-Steps zählen nicht als neue Fehler.

**69 eindeutige primäre Source-Diagnosen:** Shell 53 = 52 libtest-Diagnosen einschließlich des bereits separat reparierten Syntaxfehlers, plus eine Feature-/Pager-Abhängigkeitsdiagnose; Tools 16. Codes: E0425 36, E0061 15, E0609 9, E0560 2, E0308 2, E0599 3, E0432 1, Syntax 1. Davon: Syntax bereits in 4302 repariert; 66 Fundstellen hier statisch adressiert; E0609 am Goalvertrag zweimal offen. Das ist keine Kompilierbestätigung.

Vollständige Fundstellen mit ursprünglichen Zeilen und Wiederholungszahl: [diagnostics.md](diagnostics.md), maschinenlesbar [diagnostics.json](diagnostics.json). Parser: [sanitize.py](sanitize.py), [inventory.py](inventory.py). Keine neuen CI-Logs abgerufen und keine neuen Rawlogdumps erzeugt oder an den Lead weitergegeben.

| Fehlerklasse / Rootcause | Bestehender Owner / Migration | Erhaltene Geschäftsoracles |
|---|---|---|
| E0425: Gateway-/Notification-/Admission-/Taskoutput-Konstruktoren fehlen | Shell `agent/subagent/tests/mod.rs`: vorhandenen `test_gateway` importiert. `session/acp_session_tests/support.rs:1054–1140`: echte Fixturekonstruktion für Bash-/Monitor-Notifications, `TaskWakeAdmission`/`TaskWakeFallback` und Taskoutput wieder verbunden. Task-ID wird aus `NotificationSource` abgeleitet. | Unterdrückung nach Stop, Goal-Unterdrückung normaler Tasks, Monitor-Priorität, Fallback vor Ablehnung bei verlorenem Ack, Subagent-Ausnahme vom Task-Wake-Barrier, genau konsumierte Completion-IDs; laufende Ergebnisse und unterbrochene Wait-Platzhalter bleiben unkonsumiert. Alle bestehenden Assertions bleiben. |
| E0061: CompletionData, Cancellation, Snapshotref und Worktree-Disposal haben neue Signaturen | Shell `tests/mod.rs` und `tests/rest.rs`: gültige Signaturen. Cancellation und anschließende Präsentation teilen dasselbe `ShellCompletionData`. `pending_shell_completion_fixture` konstruiert initiale Running-Metadaten und `TerminalArtifacts::StartPublished` mit echter `SubagentCompletionId`/`AttemptId`; keine vorweggenommene Terminalisierung. | Usage-Ack vor Präsentation; genau ein Cancelled-/Failed-Finish; frischen Worktree entfernen, resumed Worktree erhalten; unbelegten Thread-Exit konservativ behandeln; Timeout bleibt Failed statt Cancelled; Snapshotpointer bleibt vor Directory-Removal persistiert; fehlende Metadaten bleiben Fehler. Die unten genannten drei Konflikt-Oracles bleiben unverändert. |
| E0560: `AutoWakeInputs.run_in_background` und `ShellCompletionData.attempt_id` existieren nicht mehr | Shell `tests/mod.rs`: Wakegate-Inputs entsprechen vorhandenem Owner; Attempt liegt ausschließlich auf `ChildCompletion`. Panicfixture übergibt dort `AttemptId::mint(0xface)`. | Gesamte Wake-Suppressionstabelle unverändert. Panic-Finish erwartet weiterhin exakt `at1.face`; nicht auf den zuvor falschen Consumerwert `at1.1` geändert. |
| Statisch mitbetroffene Completion-Consumer verwenden noch generische Prompt-/Lifecycle-Kommandos | Shell `tests/mod.rs:66,688,795,863,2851`: Match auf `SubagentCompletionPublished` und `SubagentCompletionWake`. Für die Verbatim-Oracle wird der echte `run_session`-Receiver über einen reinen Harnesskonstruktor gestartet, und das tatsächlich eingereihte `InputItem.verbatim` geprüft. | Alle bisherigen Text-/Reihenfolge-/Schedulerhint-/Truncation-Oracles sowie `assert!(verbatim)` erhalten; Erfolg und Runtimefehler wachen, expliziter Stop bleibt still; exakt ein Finish und ein Wake. Keine lokale Nachbildung des Receiverzweigs. |
| E0432: Featurefixture importiert einen nur unter cfg(test) existierenden Presenter | Shell `agent/subagent/isolated_spawn_e2e.rs:207–365`: ruft die ausgelieferten `prepare_completion_admission`/`publish_committed_completion` auf. Der bestehende Fixtureowner konstruiert einen echten Persistenceowner; der Parenttransport leitet Publish-Nachrichten an dessen bestehendes Protokoll weiter. | Echte Commit-/Visibility-Ergebnisse statt bedingungsloser Erfolgs-Acks; isolierter Spawn behält Erfolg/Error/Worktree-Ergebnis. Kein cfg-Widening des privaten Testpresenters, kein zusätzliches Produktionsdefault und keine kopierte Terminalisierungslogik. |
| E0425 / E0061 / E0308 in Tools: fehlende Imports, zusätzliche CompletionObservations, typisierter Resultsender und Terminaloutput | `task/backend_tests.rs`, `task/coordinator_tests.rs`, `coordinator/active_message_tests.rs`, `coordinator/root_targets_tests.rs`: `ChildCompletion`-/Checkpointimport, optionale Observations explizit, echter `SubagentResultSender`, `TerminalOutput::Finishing` für den bestehenden Sentinel. | Registrierung vor Resultat; Panic-/Finalisierungsfencing; Queue-Promotion und Ablehnungen; Root-Routing darf fremde Terminaloutputs nicht entfernen. |
| E0609 / E0599 in Tools: Bool-Publikation durch Zweiphasenvertrag ersetzt | `coordinator/agent_targets_tests.rs`: vorhandenen `finish_child_before_visibility` wiederverwendet; reale Commit-/Visibility-Callbacks werden aus `terminal_published_rx` an `handle_terminal_publication` gegeben. | Zwei Wakes koaleszieren, warten auf Sichtbarkeit; Root-/Granted-Replay behält Incarnation; Pair-/Attemptquoten und sämtliche erwarteten QuotaExceeded-/NotActive-Ergebnisse bleiben exakt. Keine manuell auf Published gesetzten Flags. |
| E0609 zweimal: Goal-Sharing-Vertrag fehlt | Unverändert gelassen; siehe Produktionsblocker unten. | Die historische Shared-Goal-Oracle wird nicht durch einen Capability-Bool, Fakefeld oder Selbstvergleich ersetzt. |

### Weitere Steps aus demselben Log

- **Pager:** keine zusätzliche eigenständige Pager-Source-Compilerdiagnose. Der Pager-Lib-Step exponiert E0432 in der Shell-Featurefixture; diese Fundstelle steht genau einmal im Index und ist oben adressiert.
- **Chat-State:** kein Compilerfehler, sondern ein echter erfasster Runtimefehler: `actor::tests::apply_turn_request_pruning_is_noop_under_half_window`, vorher 3019 statt 8000 Bytes. Rootcause der Fixture: fünf ungekürzte 8000-Byte-Toolresultate liegen mit ihrer Retained-Estimate bereits über der halben 10000er-Windowgrenze. Providerusage 4000 setzt diese Schätzung nicht auf 4000; der vorhandene Vertrag in `actor/request_builder.rs:122–145` verwendet ausdrücklich die Retained-Estimate, nicht den Providerwert. Fixturewindow jetzt 32000; zusätzliche Vorbedingung Retained-Estimate <16000. Die Ergebnisoracle bleibt exakt **8000 Bytes**, Pruning bleibt eingeschaltet, Payload und Turnzahl bleiben identisch. Nicht ausgeführt.
- **Sampling-Types:** keine Compiler- oder Testfehler in diesem erfassten Step; historisch 302 bestanden. Keine Sampling-Types-Datei geändert. Keine Aussage über einen neuen Run.
- **PTY:** echter erfasster Timeout in `finished_child_manual_compact_cancel_returns_to_reopenable_child`, vor der eigentlichen Compact-/Cancelprüfung. Der Helper `send_subagent_message_row.rs:148–150` erwartet nach Tab immer `Space:prompt`. Produktquelle `pager/src/views/agent.rs:931–973` bietet den unpinned Prompt-Hint abhängig von der Selektion an; bei einem kollabierten Userprompt fehlt er bewusst. Das erfasste Bild zeigt Expand-/Thinking-Hints. Kein Compilerfehler; kein Beleg dafür, dass Tab den Fokus nicht wechselte. Helper/Oracle unverändert, kein Timeout erhöht und keine schwächere Ersatz-Erwartung eingeführt. Dieser PTY-Gate bleibt offen und benötigt eine separate, unabhängige Fokus-Oracleentscheidung.

## Minimale Produktions-/Contractblocker an den Lead

### 1. Fehlende semantische Goal-Beobachtung blockiert weiterhin die Shell-libtest-Kompilierung

`agent/mvp_agent/tests/subagent_spawn_context_tests.rs:162–175` verlangt, dass ein bereits gebauter Spawncontext die spätere Aktivierung desselben Parent-Goal-Gates beobachtet; die Ausdrücke in Zeile 170 und 173 greifen auf das entfernte `ctx.goal_loop_active` zu. `SubagentSpawnContext` bietet nur `goal_enabled` als Capability (`agent/subagent/mod.rs:258`) sowie Parentkanäle (`:266–271`). `try_build_subagent_spawn_context` setzt `goal_enabled` aus der Konfiguration (`agent/mvp_agent/subagent_spawn.rs:377`), nicht aus dem laufenden Goal-Lifecycle. Ein gleichbedeutender geteilter typisierter Goalzustand ist in diesem Contextvertrag nicht vorhanden.

**Benötigt:** eine Leadentscheidung bzw. reale semantische Goal-Lifecycle-Beobachtungs-API am bestehenden Owner, falls die historische Shared-State-Invariante weiter gelten soll. `goal_enabled` ist kein Ersatz. Kein Produktionsfeld ergänzt, keine Testassertion entfernt. Diese zwei bekannten E0609 bleiben sichtbar; der Patch ist daher ausdrücklich nicht buildfähig bestätigt oder vollständig entblockt.

### 2. Historische Terminal-Metadaten-Oracles widersprechen der neuen Producer-Commitgrenze

Diese Konflikte sind statisch sichtbar, **nicht neu ausgeführt**:

1. `agent/subagent/tests/rest.rs:511–526`, `snapshot_ref_write_promotes_nonterminal_status_to_terminal`: startet mit Running, erwartet nach Snapshotref-Write weiterhin Completed. Der vorhandene Writer `agent/subagent/mod.rs:2561` delegiert an `metadata_store.rs:48–53`, das ausschließlich `snapshot_ref` ändert. Statuspromotion vor Producercommit ist dort nicht vorgesehen.
2. `agent/subagent/tests/mod.rs:2472–2523`, `unproven_thread_exit_preserves_fresh_worktree`: erwartet bereits unmittelbar nach Cancellation die Cancelled-Metadaten, Timestamp und Cause (`:2519–2521`). `cancel_pending_shell_child` defert bei `:2117` lediglich Terminalartifacts; bei unbelegtem Thread-Exit liefert es ohne Metadatenwrite zurück.
3. `agent/subagent/tests/mod.rs:2525–2571`, `startup_admission_timeout_is_failed_not_cancelled`: erwartet ebenso Failed-Metadaten vor der späteren Präsentation. Derselbe vorhandene Cancellationvertrag hat bis dahin nur die initialen Running-Artefakte behalten.

Die Fixtures initialisieren jetzt korrekt Running-Artefakte und teilen die CompletionData; sie schreiben nicht heimlich das erwartete Endergebnis. **Benötigt:** die verbindliche Grenze der historischen Metadaten-Invariante gegenüber dem neuen durablen Producervertrag festlegen. Eine frühzeitige Terminalisierung wäre ein Produktvertragseingriff; eine erwartete Running- statt Terminalstatus-Assertion wäre eine verbotene Umdefinition der Oracle. Beides wurde unterlassen. Ein Folgeslice kann nach dieser Entscheidung die Oracles über den realen Commitowner treiben; ein Fakecommit oder der private Presenter darf diese Entscheidung nicht ersetzen.

## Unverified Gates — ausschließlich Lead/CI

Nicht ausgeführt: rustfmt `--check`, Compiler, Tests, Clippy, UI-/PTY-Läufe und neuer GitHub-Run. Der einzige ausgeführte Rustprozess war der ausdrücklich erlaubte gepinnte **Formatter ohne Check** (`rustup run nightly-2026-08-13 rustfmt`, Edition 2024, vorhandene Config, `skip_children=true`). Für Chat-State wurde abschließend der Bereich 4255–4290 mit `--unstable-features --file-lines` formatiert; außerhalb dieses Bereichs bleibt der vorherige Text unverändert. Ein erster Range-Aufruf ohne den erforderlichen Flag wurde vom Formatter abgelehnt; der korrigierte Aufruf endete mit Exit 0. Bestehende `cfg_attr(rustfmt, rustfmt::skip)`-Vorgaben bleiben erhalten.

Nach Auflösung des Goalvertrags sind beim zuständigen CI-Owner mindestens erforderlich:

- Die erfassten `linux-test.yml`-Formatierungs- und Kompiliergates auf der tatsächlichen Integrationsrevision erneut ausführen; nicht den alten roten c8-Run als Verifikation verwenden.
- Shell-Libtests: `agent::subagent::tests`, `subagent_spawn_context_tests`, `auto_wake_suppression_tests`; dabei die drei Metadaten-Konflikt-Oracles explizit beobachten. Verbatimtest verwendet nun den realen Receiver und ist ebenfalls unverified.
- Tools-Libtests: `backend_tests`, `coordinator_tests` einschließlich `active_message_tests`, `agent_targets_tests` und `root_targets_tests`.
- Shell-Feature-/Pager-Kompilierung mit dem vorhandenen `test-support`-Abhängigkeitspfad; die isolierte Spawnfixture muss echte Producer-Commit-/Publish-Acks erreichen.
- Chat-State `apply_turn_request_pruning_is_noop_under_half_window` und vollständiger vorhandener Sampling-Types-Gate.
- Den bereits erfassten PTY-Compact-/Cancel-Gate nach unabhängiger Fokus-Oracleklärung erneut ausführen; kein Timeout-/Ignore-/Assertionpatch in dieser Lieferung.

## Übertragung und Artefakte

Patch ist gegen den **erhaltenen HEAD bb36f7bd** erstellt, nicht gegen den inzwischen aktiv weiterbearbeiteten Hauptrepo-Arbeitsstand. Nur gezielte Hunks integrieren; keine ganzen Dateien über aktive Registrierungs-/Zustellungsautoren kopieren. Insbesondere die produktiven Dateien `session/completion_delivery.rs` und `agent/mvp_agent/session_setup.rs` wurden nicht geändert.

- Patch: [fixtures.patch](fixtures.patch), SHA256 `33d0c8d252f18da03046aff401b3c9805d9ac90cbc330b718e4930f66c7cc090`.
- Vollständige absolute Sourcepfade, ursprüngliche HEAD-Hashes, aktuelle Hashes, Inputhashes und erhaltene Revisionshashes: [source-manifest.json](source-manifest.json).
- Lesbare Pfad-/Hashliste einschließlich aller vier Inputs: [paths-and-hashes.md](paths-and-hashes.md).
- Reine Metadaten-/Artefakterzeugung: [manifest.py](manifest.py).

### Alle geänderten Pfade und aktuellen SHA256

Relativ zum oben genannten isolierten Worktree:

| Source | SHA256 |
|---|---|
| `crates/codegen/xai-chat-state/src/actor/tests.rs` | `2eeebf8823c2b57868fe7ea5a0b8460c80a0be554d696f27abd09e4bc7bb7f22` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/isolated_spawn_e2e.rs` | `cf4dd5fddf7d15925ebba0a48de6c5107a572e26f2ffc910bb017738524dfcb0` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/tests/mod.rs` | `e064e0f9412a5905734d3bb2c3dae7e622baf5bd9c530c6a4faa44b647c6d3dc` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/tests/rest.rs` | `22eeeb827b5c4af3391808aa5e1542b806c1ec03bde1ba8a341241f828694e06` |
| `crates/codegen/xai-grok-shell/src/session/acp_session.rs` | `1a8078722c230629658342365205b9420a410aee35d65a8edb2365546d539764` |
| `crates/codegen/xai-grok-shell/src/session/acp_session_tests/auto_wake_suppression_tests.rs` | `87be37ae0829d823401c09fc9d4496848416604f988f1f1089548922009b8507` |
| `crates/codegen/xai-grok-shell/src/session/acp_session_tests/support.rs` | `55a6a91aae0921da49c4c132e8fdc7e7a912b12b6617f9c1b082c11d5de93fe5` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/backend_tests.rs` | `e44b19f78f0a6f6fb8404653a6c609f23e2ecc80ee6afae98867d67c3a658500` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator/active_message_tests.rs` | `f71f985adb155c9b6cd506974cbcfaf081f1e9f68da2066b2862afae602af04f` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator/agent_targets_tests.rs` | `be2464f541ca5d91eeaade8b01346c189e323fe2b7c57fde8dadad8224c9107c` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator/root_targets_tests.rs` | `60b70ff1dc9866522ff945e2d7c0403d947d90239bd54755d78cbf2bf50e97b2` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator_tests.rs` | `d702f6479012f4dbe61ff356c4298463b11ad90aba7469485a8717566388f8e4` |

Inputhashes stehen vollständig in der verlinkten Liste und im Manifest. Die beiden erfassten Logdateien sind bytegleich, SHA256 `947825dd02848b285644f571263f6bcd63b8a394960f8bf645e758de53baf523`.
