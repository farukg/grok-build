# C2a — Runtime-Lifecycle und Beobachtung

## Stand

Begrenzter Autorenstand im gemeinsamen kanonischen Arbeitsbaum; nicht gelandet.
Keine Branchwechsel, Commits oder Pushes. Die vorhandene Branchwahl wurde gemäß
Direktauftrag nicht geändert. `STATUS.md` bleibt unverändert.

Grundlage sind die vollständig gelesenen Vorgängerberichte
`/tmp/grok-child-graph-completed.md`,
`/tmp/grok-child-runtime-capture-authored.md` und
`/tmp/grok-child-inventory-model-authored.md`.

## Geschrieben

- Positives `AttemptEvidence::NotStarted`, getrennt von historischem `Legacy`.
  Historische Quellen dürfen keinen Nichtstart behaupten. Nur ein ausdrücklicher
  Nichtstart-Proof kann ohne das alte `attempt_id` gebunden werden.
- Targetinitialisierung vor lesbarer Queue-/Pending-/Nie-gestartet-Custody;
  rein lesender, tatsächlicher Targetzugriff beim bisherigen Quotenregister.
- `CompletedOrigin` und mitgeführter Committing-Ursprung erhalten einen echten
  Nichtstart auch nach Queueabbruch. Die bisherige Publication-/ACK-Identität
  wird weder als Ausführungsversuch ausgegeben noch geändert.
- Eine geordnete, nach Child-ID adressierbare Queue statt globaler Queuesuche.
- Echter Capture aller fünf Custodyformen für ausdrücklich angefragte IDs;
  einmalige echte Controlabfrage für Running, getrennte Finalisierungsphasen,
  strikte Producer-/Fact-Evidence und typisierte Fehler bei fehlendem Join.
- Benannter, eigener asynchroner Capturefuture mit tatsächlicher Graphrevision;
  keine Actorborrow über das Warten und kein Verlust der natürlichen Send-Eigenschaft.

Die Entscheidung und das quellbasierte Capabilityinventar stehen in
[ADR 003](../../../context-modules.d/500_work.d/epics.d/product.d/grok-build/003-child-runtime-capture.adr.md).
Der genaue API-/Datei-/Test- und C2b-Migrationsplan steht in
`/tmp/grok-child-runtime-lifecycle-authored.md`.

## Prüfung

18 Runtimeowner-/Maporakel und zwei Modellgrenzenorakel sind geschrieben,
sämtlich **UNRUN**. Kein lokaler Build, Check, Test, Formatter, Appstart, Probe,
Netzwerk-, Liveprozess- oder Sessioneingriff. Keine GitLabausführung, kein
Commit und kein Push. Kein Compile-/CI-/Laufzeitabschluss wird behauptet.

## Als Nächstes

C2b verbindet den bereits vorhandenen Graph mit Actor, Resolver und neuen
Respondertypen. Der Actor liefert die vollständige Kandidatenmenge, führt den
Capturefuture neben seinen übrigen Futures, prüft Source-/Targetfences nach der
Antwort und übernimmt Lifecycleevidence sowie den echten Release-Drain.
Query/List/Wait-/Cancelmigration, Backend und native Wiederaufnahme bleiben bei
C2b. Shell, Pager, ChatState, Workflow und Interjection wurden nicht geändert.
