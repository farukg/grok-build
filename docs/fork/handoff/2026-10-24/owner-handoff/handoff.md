# Grok-Übergabe an den neuen verantwortlichen Agenten

## Auftrag und Freigabe

Grok-Implementierungsarbeit dieser Session ist angehalten. Der neue Owner übernimmt die offenen Grok-Aufgaben in einer frischen Session. Sokrates behält Sigma-/Gateway-Arbeit; notwendige Änderungen gemeinsamer Verträge werden zwischen beiden Ownern abgestimmt.

Die beiden folgenden Checkpoints sind lokal vorbereitet. Ihre Veröffentlichung bleibt gesperrt, bis Faruk ausschließlich mit dem Namen des neuen Grok-Owners antwortet. Diese Übergabe behauptet keine erfolgreiche neue CI oder Stage.

Aktuelle Scope- und Freigabereferenz: [current-user-scope.md](../current-user-scope.md).

## Zwei getrennte Sourcecheckpoint-Branchen

| Sourcebestand | Lokaler Commit | Zur Veröffentlichung vorbereitete Branch |
| --- | --- | --- |
| Kanonischer Checkout, 27 bisher uncommittete Pfade | `074f54b599d2bdd46c150124de4dc5f117641565` | `handoff/grok-canonical-20261024` |
| Isolierter Runtimefix, 14 Pfade | `14a5f56c12f423f58540f649360fc10a115c92f0` | `handoff/grok-runtime-20261024` |

Der kanonische Checkpoint basiert auf `c8e075ea29ab74356cb477f05dcd70cdf7eb0e4c`; der Runtimecheckpoint auf `bc85ceb95ee122ed436cfbe516c41bd6208d4fb2`. Diese Bestände haben unterschiedliche Basen und überlappende Pfade. Ihre fachlichen Hunks müssen gezielt zusammengeführt werden; ein Wholefileaustausch zwischen den Checkouts würde Fortschritt verlieren. Die Checkpoints wurden ohne Änderung ihrer Sourcebytes erstellt. Beide Arbeitsbäume waren anschließend sauber.

Kanonischer Checkout: `/home/fg/projects/org/forks/grok-build`.

Runtimeworktree: `/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format`.

Exakte Pfade, Bytegrößen und SHA-256: [canonical/source.json](./canonical/source.json) und [runtime/source.json](./runtime/source.json). Gitidentitäten und Erhaltungsprüfung: [checkpoints.json](./checkpoints.json). Zusätzlich liegen ursprüngliche Patches und vollständige Sourcearchive neben den Manifesten.

## Verantwortlicher nächster Schritt

Zuerst die zwei Checkpoints und die unten referenzierten Ergebnisowner lesen. Die letzte Authorfortsetzung war bei den drei Metadaten-Phasenoracles unterbrochen. Die zwei betreffenden Testdateien sind im Runtimecheckpoint enthalten; ihr aktueller Zustand ist vollständig erhalten, jedoch nach der Unterbrechung noch nicht erneut abgenommen.

Danach die verbleibende Compiler-/Runtimeclosure auf einem kohärenten Snapshot abschließen, committen und über GitHub Actions prüfen. Für die finale Auslieferung müssen Build und relevante Tests für denselben endgültigen SHA erfolgreich sein. Erst dieser konkrete erfolgreiche Build darf über Glue heruntergeladen und gestaged werden.

## Aktueller Runtimefix und bekannter Reviewbefund

Der erhaltene Runtimefix umfasst:

- terminale Childmetadaten nach durablen Producercommit, bevor dessen Ack zurückkehrt;
- typisierte Child-Wakes über die tatsächlichen Queue-, Receipt- und Safe-point-Pfade;
- Shutdown und Crashrecovery für noch unprojizierte Childresultate;
- gültige Summary-Fixtures für die bestehende TTL-Bereinigung samt Schutzoracle für malformed Summary;
- den begrenzten Taskfixture-Sichtbarkeitsjoin;
- PTY-Navigation in die tatsächliche Childsession mit konkreter Session-ID, Rückweg, Compaction und Cancel.

Die vollständige fachliche Erklärung des ursprünglichen 12-Dateistands steht in [source-report-before-stop.md](./source-report-before-stop.md). Dessen Hashlisten beschreiben diesen vorherigen Stand; für den finalen Checkpoint gelten ausschließlich die oben verlinkten Übergabemanifeste.

Der unabhängige P1-Befund und die erforderlichen Phasenoracles stehen in [review.md](../../ci-bc85-runtime/review.md). Eine erneute Prüfung des finalen 14-Dateistands steht aus.

## Tatsächliche bisherige CI-Evidence

Veröffentlichter Ausgangsstand: `bc85ceb95ee122ed436cfbe516c41bd6208d4fb2`.

- GitHub-Build: Run `40058325363`.
- GitHub-Test: Run `40058633091`, fehlgeschlagen; der bereits fehlgeschlagene Testlauf wurde nach mehrstündigem Shellhang beendet.
- Format, Sampler und Shellrouting bestanden am Ausgangsstand. Der Runtimecheckpoint hat daraus keine eigene Abnahme.
- Der vollständige Capture enthält 27 unterschiedliche FAILED-Testnamen sowie den zusätzlichen hängenden Shelltest. Die deduplizierte Liste und Capturezeilen stehen in [diagnostics.json](../../ci-bc85-runtime/diagnostics.json).
- Echter Vollcapture: `/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures/bc85-failed-ci.log`.
- Laufmetadaten: `/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures/ci-bc85-results/terminal-summary.json`.

Der Telemetrietest `execute_tool_calls_records_the_product_row_and_execution_span` besitzt im beendeten Capture keinen individuellen Assertionstdout. Weitere Config-, MCP- und Unified-List-Failures sind ebenfalls nicht einzeln ursächlich erklärt. Diese Fälle brauchen echte neue CI-Evidence; sie gelten nicht als behoben.

Die bekannte Hangursache wurde als veraltete, untypisierte Wakefixture adressiert. Eine regulär beendete vollständige Shellsuite muss nun die offenen Einzelfehler einschließlich ihrer tatsächlichen Assertiondiagnosen liefern.

## Weitere Grok-Aufträge, die mit übergehen

Die bisherigen fachlichen Anforderungen bleiben erhalten:

- dauerhafte Completion-/Deliverytransaktion mit restartfesten Receipts, korrelierter Child-/Attemptidentität und genau einmal sichtbarer Projektion;
- dynamische Parent-Goal-Auswertung am tatsächlichen Consumer und echte AppView-Replayprüfung;
- globale Task-/Todo-Anzeigepräferenzen über Sessions und Neustart;
- Arbeitsinhalte dauerhaft in F7, optional zusätzlich oben, Queued unmittelbar über dem Chatinput;
- Mid-run-Steer und vollständige Tool-/Completion-Wireverträge;
- finale echte Terminal-/PTY-Abnahme und Auslieferung desselben erfolgreichen SHA.

Die jeweiligen vorhandenen Detailowner werden über den Sessionplan und dessen Ergebnisreferenzen erschlossen:

`/home/fg/.grok/sessions/%2Fhome%2Ffg%2Fprojects/01a15e70-cc42-70d2-8b7b-a20615d0abf4/goal/plan.md`

Weitere Einstiegspunkte:

- `delivery-shell/architect-restart-continuation.md`
- `ci-c8-fixtures/result.md` und `ci-c8-fixtures/review-unblock.md`
- `pager-global-display.md`, `pager-global-restart-fix.md` und die vorhandenen Pagerreviews
- `missing-model/` und `compaction-pty/`

Alle relativen Einstiegspunkte in dieser Liste liegen unter `/tmp/grok-goal-4f7553a1dbe6/implementer/`.

## Grenzen und spätere Auslieferung

Grok-Rustbuilds und -Tests erfolgen ausschließlich über Commit/Push und GitHub Actions. Lokal liefen für diesen abschließenden Checkpoint weder Compiler noch Tests oder PTY. Der vorherige 12-Dateistand bestand die gepinnten Formatprüfungen; die anschließende Erweiterung ist davon getrennt.

`codex-rs` bleibt lesbare Referenz. Die vorgegebene Modellbelegung verwendet GPT-6 Luna und GPT-6.1 Sol. Bestehende Sourcearbeit und konkrete Oracles bleiben erhalten.

Der vorhandene Glue-Stageowner unterstützt `--run-id` und `--commit`; Ausgangsbeleg ist Glue-Commit `7fe173e`. Für diesen Übergabecheckpoint wurde kein neuer Build gestaged. Aus älteren Releasepfaden darf weder der aktuell ausgewählte noch der tatsächlich laufende Grok-Stand abgeleitet werden; beides ist bei der finalen Abnahme explizit zu prüfen.

## Sigma-/Gateway-Koordination

Kontakt für notwendige Gateway- oder Orchestratoränderungen: `Sokrates`, tatsächliche native Session `01a15e70-cc42-70d2-8b7b-a20615d0abf4`, Directchannel `orc.direct.Sokrates`.

Der neue Agent registriert seine eigene reale Sessionidentität vor lokaler SIG-Koordination. Die bisherige Registrybindung wird dabei nicht übernommen oder umgebunden.

Für Vertragsänderungen zuerst konkreten Wire-/Request-/Receiptbeleg, benötigte Producer-/Consumeränderung und kompatible Versions-/Rolloutgrenze austauschen. Grok- und Gateway-Freigaben bleiben anhand ihrer jeweils tatsächlichen Artefakte und Runtimebelege getrennt nachvollziehbar.
