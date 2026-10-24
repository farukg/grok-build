# Caller completion admission/recovery — finale Authorclosure

Aktueller Ergebnisowner: [architect-restart-continuation.md](/tmp/grok-goal-4f7553a1dbe6/implementer/delivery-shell/architect-restart-continuation.md). Die ursprüngliche Fassung dieses Checkpoints bleibt unter `architect-restart-baseline/wakeup-final.md` erhalten.

## Current truth

Die bestehende Shell-/Tools-/ChatState-Transaktion ist fertig authoriert und am aktuellen Sourceowner reconciliiert, **nicht kompiliert, getestet oder live abgenommen**. HEAD bleibt `c8e075ea29ab74356cb477f05dcd70cdf7eb0e4c`; finale 73-Dateien-Sourceidentität ist `fdd27dc80a4f2803393037c13468952442d7e82b13bef45e48ea31e27151484f`. Vollständige Pfade/Hashes und rückholbarer Source liegen im [Manifest](/tmp/grok-goal-4f7553a1dbe6/implementer/delivery-shell/architect-restart-sourceidentity.json) und zugehörigen Archiv.

- AttemptId folgt Coordinator, Producercommit/-publikation, Completionbuffer und beobachteten Toolresultaten bis zu atomarem Modellinput-/Receiptappend. Retry und Recovery verwenden die tatsächliche gespeicherte Occurrence, nicht die Identität des Retryvorschlags.
- Passive Buffervisibilität wartet auf Historypublication. Frühes Receipt-ACK oder Readerteardown retiert denselben pending Terminalzustand; späte Sichtbarkeit resurrectet ihn nicht. Der alte destruktive Task-ID-only-Drain ist gelöscht.
- Idle-Digest, aktive Sampling-/Toolgrenze und gemeinsame Batchrecovery behalten Erfolg, tatsächlichen Fehler und occurrencegebundene Fullresultreferenz. Keine Goal-Suppression. Eine neue native Oracle aktiviert den Parentgoal erst nach gebautem und laufendem Childcontext; ein Capabilitybool ersetzt keinen dynamischen Receiver.
- ParentTask-/Origin-Fencing reicht bis zu Appendlock, Executionlease, Recovery und Cancellationteardown. Humanstop, Held/Auto und gewöhnliches Callerturnende bleiben getrennte Policyowner.
- Native `x.ai/session/delivery` samt realem ACP-/Session-/Storageconsumer existiert mit dem exakten vereinbarten Wirevertrag. Externe Humanreceipt bleibt vom Childresult getrennt; Observe plant nichts, identischer Admit erzeugt keinen zweiten Input, Interrupted wird nach Prozessverlust nicht blind replayt. Standard-`session/prompt` bleibt terminalgebunden.
- Metadataoracles laufen durch dieselbe tatsächliche CompletionData und den realen Producer-/Publishowner. Initiales Running bleibt bis zur Veröffentlichung unverändert; die ursprünglichen Completed-/Cancelled-/Failed-, Cause-/Timestamp-/Worktree-/Snapshotoracles werden danach geprüft. Kein vorweggenommener Terminalzustand, kein Rekonstruktions-Fakecommit und kein privater Presenter als Featureconsumer.
- Verbatim-/Text-/Hintoracles verwenden den realen `run_session`-Receiver mit FIFO-Barriere. Die bekannten c8-Fixture-/Signaturfehler und die ungültige freistehende ToolRunResult-Feldzeile sind am aktuellen Mainowner reconciliiert; keine Testoracle wurde zur Fehlerumgehung ignoriert oder abgeschwächt.

## Verifikation und Grenzen

Neue und migrierte unabhängige Regressionen für Lost-/Late-ACK, tatsächliche Occurrence, Child-ID-Wiederverwendung, Batchcontinuation, native idle/active/foreground/goal/late-goal Success/Error, Receivercancel und OS-Prozesscrashgrenzen sind authored, sämtlich **NOT RUN**. Bestehende Held/Auto-/Intentionalstop-Regressionen bleiben relevant.

Nur der erlaubte gepinnte Formatter wurde angewendet, letzter Lauf Exit 0. Keine lokalen Grok-Rustbuilds/-checks/-tests, keine eigenen Commits/Push/Stage und keine Session-/Binaryänderung. Der rote Run `40003673652` prüfte den früheren c8-Stand, nicht diese spätere Sourceclosure. Lead muss exakt die integrierte Revision kompilieren und ausführen; Consumer nicht allein aufgrund dieses Reports als live `ReceiverExact` freigeben. Separater ChatState-Pruning- und PTY-Fokusgate bleibt ohne hier behaupteten Fix; Pager-/Registry-/Origin-/SIGowner wurden nicht angefasst.

Historische Unterbrechungen ohne typisierte Ursache bleiben ungeklärt, ohne Textheuristik oder erfundene Humanintent-Klassifikation. Eine rückwirkende Migration braucht eine ausdrückliche Lead-/Nutzerentscheidung. Keine belegte Behauptung über Senkus historische Ursache oder einen deployten Neustartfix.
