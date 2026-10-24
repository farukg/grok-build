# Grok-Fixtures: unabhängiger Review des Folgeslices

## Goal-/Commit-Review

Reviewer `01a16aef-4fa3-7e23-9a70-13d72d74325e`, read-only.

- P1, `agent/subagent/tests/mod.rs:2667–2686`: Live-Zählung endet vor dem zweiten Publish. `spawn.rs:503–509` sendet dabei erneut einen Finish; die aktuelle Exactly-once-Oracle erfasst diesen Replayeffekt nicht. Beobachtung und FIFO-Barriere müssen den gesamten Replay einschließen. Der echte unterstützte Producer-/Coordinator-/Consumerpfad muss die behauptete einmalige sichtbare Wirkung tragen.
- P2, `mvp_agent/tests/subagent_spawn_context_tests.rs:210–266` und `subagent/tests/mod.rs:931–938`: `yield_now` plus Zustandsabfragen ist Polling; vorhandene Ereignis-/Receipt-/FIFO-Synchronisation verwenden.
- Evidenzlücke, `xai-chat-state/src/actor/tests.rs:4267–4286`: Providerusage und Retained-Estimate liegen beide unter der Halbfenstergrenze. Ein falscher Rückfall auf den Provider-Gate würde hier ebenfalls bestehen. Zusätzlicher kontrastierender kohärenter History-/Retainedfall muss die tatsächliche Gateentscheidung unterscheiden.
- Positiv: echter Coordinator klassifiziert ExplicitStop zu Silent; vierfacher Wakevertrag und FIFO-Nullbarriere, spätere Goalaktivierung/-pause und echte Persistence-Acks werden grundsätzlich getrieben. Terminalerwartungen sind erhalten.

## PTY-Folgereview

Reviewer `01a16a96-1016-7170-b835-a11f37023203`, read-only.

- P1, `send_subagent_message_row.rs:187–193`: Die Klickbestätigung ist schon vor dem Klick wahr. `wait_until` kann zurückkehren, bevor Mouseevents verarbeitet sind. Vorher den Fokus auf echtem Nutzerweg vom Prompt abziehen und das Verschwinden genau seines Auswahlrahmens abwarten; danach Klick und neu erschienenen Rahmen beobachten. Einen durch den Fokuswechsel erzeugten Draft oder andere Nebeneffekte im festen Ablauf berücksichtigen.
- Tab-Oracle prüft tatsächlich einen neuen strukturellen Rahmen. Decodergeometrie und Unicode-Spalten sind für die feste 120×50-Fixture statisch plausibel.
- Offene Clippinggrenze: Decoder:95–108 erfasst zweizeiligen Pushheader „Prompt + unteres Padding“ nicht; `scrollback/sticky.rs:298–317` erlaubt ihn. Erreichbarkeit an der konkreten Fixturestelle ist nicht belegt; Source-/Fixturebeleg liefern oder unterstützten tatsächlichen Fall ergänzen.
- Unverändert erhalten: Caret-/Enter-Childpfad und sämtliche Child-/Session-Info-Assertions. Kein belegter Produktfokusbug.

Beide Reviews blockieren die Freigabe. Reine Format-/Hashchecks ersetzen diese fachlichen Oracles und die spätere GitHub-Abnahme nicht.
