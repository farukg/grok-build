1. Der Capture enthält zwei konkrete Assertiondiagnosen, fünf E0451-Stellen, 27 deduplizierte FAILED-Testnamen und einen zusätzlich hängenden Shelltest; Shell-stdout fehlt wegen kontrolliertem Abbruch.
2. Zusammenhängender Slice hier: Producercommit→Terminalmetadaten, typed Wake→Receipt/Hang, Coordinator-Testvisibility, veraltete Takeover-PTY-Boundary; autoriert, nicht ausgeführt.
3. Vor Weiterung separater Ownerbereich A: sechs `session::persistence::cleanup_stale_sessions_tests::*` (Capture 8118–8125), TTL/Dateialter/Livesession, nicht Completionzustellung.
4. Separater Bereich B: `tool_call_telemetry_tests::execute_tool_calls_records_the_product_row_and_execution_span` (6671); ohne beendeten Libtest keine tatsächliche Assertiondiagnose.
5. Drei Config-, drei MCP- und ein Unified-List-Fehler entsprechen den im STATUS genannten Umweltfamilien; keine neue grüne oder abschließende Ursachenbehauptung.
6. Verbleibender echter Policykonflikt: `SubagentCompleted` ist aktuell Drain und hat keine stringbasierte consumed ID (`session/mod.rs:210–217,280–297`); alte Shutdown-/Preemption-/Consumed-Tests verlangen das Gegenteil.
7. Diese Assertions wurden nicht gelockert und die ausgelieferte durable Policy nicht zurückgedreht; getrennte Boundary-/Oraclekorrektur nötig, ohne attemptgebundene Zustellung zu verlieren.
8. Parent bitte getrennte Ownerarbeit für A/B festlegen; hier keine Delegation, Filteränderung oder Ausführung. Ergebnis und Teilpatch: `ci-bc85-runtime/result.md`.
