1. Aktueller alleiniger Ergebnis-/Ownershipstand: `ci-bc85-runtime/result.md`; früherer Split als `resume-baseline-split.md` archiviert.
2. Die angenommene große TTL-Domain ist durch konkrete Source widerlegt: Fixture schrieb ungültiges Summary, der Produktdecoder verweigert bewusst Löschung bei unbekannten Admissions.
3. Bestehenden Summarykonstruktor am Fixtureowner verwendet; ursprüngliche Stats-/Dateialteroracles unverändert und Fail-closed-Gegenoracle ergänzt.
4. Die drei Completionboundaryfälle sind jetzt an echte Toolresultatreceipts, Between-turn-Zustellung und wirklichen Shutdown/Persistencerecoverypfad angebunden; keine Produktpolicy zurückgedreht.
5. Einziger weiterhin ungeklärter neuer FAILED-Test: `execute_tool_calls_records_the_product_row_and_execution_span`; im cancelled Capture fehlt dessen Assertionstdout.
6. Parent/CI benötigt dafür den tatsächlichen Assertiontext; keine spekulative Telemetriereparatur und keine Abschwächung vorgenommen.
7. Sieben weitere FAILED-Namen entsprechen dokumentierten Umweltfamilien, ohne neue Einzelfehlerfreigabe. Keine Delegation oder Ausführung durch diesen Leaf.
