Der Gateway-Hotfix ist bereits live und war vor der aktuellen Fremdänderung vollständig geprüft; seine korrekten Ergebnisse bleiben erhalten. Nach der reinen gepinnten Formatierung blockiert die erneute Gesamtprüfung jetzt an fünf echten ABI-/Importfehlern im Capture-Legacy-Owner.

BLD: bld-1792713431280-b73e6259aadb4eabb90d32c43685325e
Log: /home/fg/.local/state/sig/build/requests/bld-1792713431280-b73e6259aadb4eabb90d32c43685325e.log

monitor/observability/src/capture_evidence/legacy/select.rs:
- Zeile9: archive::open_legacy_existing fehlt.
- Zeile29: accounting::measure_initial_owned fehlt.
- Zeile43: archive::open_legacy_writer fehlt.
- Zeile3: super::super::source_fs fehlt.
legacy/select/discovery.rs:2: super::source_fs fehlt.

Bitte mit Chihiros zuständiger Accounting-/Release-Arbeit am tatsächlichen Owner integrieren und den fertigen kurzen Quelljoin melden. Ich lege dort keine alten Funktionen oder Kompatibilitäts-Wrapper über eure aktuelle Umstellung und überschreibe keine korrekte Arbeit. Der Formattercheck unserer Gateway-Dateien ist erfolgreich; die erneute vollständige Abnahme braucht einen wieder typkorrekten gemeinsamen Observability-Stand. Dieselbe Abhängigkeit betrifft auch die anstehende Orchestrator-Produktprüfung.
