# Pager Global Display — Restart-Follow-up

## Status

Fortsetzung nach unabhängigem Review lokal authoriert; nicht kompiliert oder ausgeführt. Insgesamt sechs Repository-Dateien gezielt geändert. Kein Commit, Push, Stage, Branchwechsel, lokaler Grok-Build/-Test/-Check oder Eingriff in laufende Sessions. Fremde Shell-/Tools-/ChatState-Arbeit und korrekte vorhandene Pagerhunks bleiben erhalten. Ausgangs-HEAD der ersten Recon: `c256ab292b86ad6fb42d76d200c953257d34c862`; umfangreicher fremder Working Tree war bereits vorhanden.

## Kausale Korrekturen

### P1: Unsichtbarer Todo-Fokus

Globale Ausblendung kopierte bislang lediglich Overlay-Flags; `active_pane == Todo` und ein offener Todo-Eingabebalken konnten in einer anderen Session erhalten bleiben. Layout-Unterdrückung konnte ebenfalls die Todo-Fläche entfernen, ohne den Keyboard-Router umzuschalten.

- Globale Übernahme verwendet jetzt `AgentView::adopt_global_work_visibility`: vorhandene `TodoPane::on_state_change`-/Tasks-Kontrakte werden aufgerufen; ausgeblendete Todo-Fokussierung wird über `set_active_pane` zur Scrollback-Fläche übertragen.
- Nach der tatsächlichen Layoutberechnung überträgt `AgentView::draw` Todo-Fokus bei Todo-Höhe 0 ebenfalls über `set_active_pane` zu Scrollback. Die globale Sichtbarkeitspräferenz bleibt bei bloßer Layout-Unterdrückung erhalten; Vergrößern stellt das Todo wieder dar, ohne den Fokus zurückzustehlen.
- Die zuvor ergänzte redundante `is_visible`-Bedingung wurde entfernt: `TodoPane::desired_height` besitzt bereits die Sichtbarkeitsprüfung. Diese Bedingung allein war keine funktionale P1-Reparatur. Die sinnvollen Painting-Regressionen bleiben erhalten.

### P2: Keine All-Views-Verteilung pro Frame

Die Schleife über sämtliche Views ist aus der Draw-Reconciliation entfernt. Alle vorhandenen Views erhalten geänderte Präferenzen ausschließlich im bereits vorhandenen Preferencechange-Inputpfad. `reconcile_work_visibility` übernimmt für eine neu aktivierte View die bestehende Live-Präferenz mit einem einzelnen Lookup und schreibt nur bei Abweichung. Es läuft vor Input-Snapshot und vor Sidebar-Paint, damit auch ein Sessionwechsel mit sofortigem Folgeinput ohne Zwischenframe konsistent bleibt. Kein neuer Speicherowner, keine zusätzliche Datei-I/O und kein Polling.

Das ist eine statisch belegte Beseitigung von per-frame-All-Views-Arbeit, kein gemessener Laufzeit-/Performancebefund.

### P3: Bestehende Subjectdocs geheilt

Nur die ausdrücklich freigegebenen widersprüchlichen Stellen im bestehenden F2/F3-Stream wurden ersetzt: F7-Arbeit bleibt unabhängig verfügbar, Dock steuert die optionale obere Projektion, Queue bleibt unabhängig direkt vor dem Composer. Kein neuer Repository-Report oder Decisionhistory-Abschnitt.

## Exakte geänderte Pfade / Symbole

Alle Pfade relativ zu `/home/fg/projects/org/forks/grok-build/`:

1. `crates/codegen/xai-grok-pager/src/app/agent_view/render.rs` — `AgentView::draw`: Todo-Höhe weiterhin durch Dock und den vorhandenen `desired_height`-Owner bestimmt; Fokusreparatur nach `AgentViewLayout::compute` bei Todo-Höhe 0.
2. `crates/codegen/xai-grok-pager/src/app/agent_view/panes.rs` — neue `AgentView::adopt_global_work_visibility`, verwendet vorhandene Pane-/Overlay-Owner.
3. `crates/codegen/xai-grok-pager/src/app/app_view.rs` — `handle_input_at_with_paste_provenance`, neue `reconcile_work_visibility`, `reconcile_sidebar_chrome`: Eventverteilung und initiale aktive Übernahme ohne All-Views-Schleife im Paintpfad.
4. `crates/codegen/xai-grok-pager/src/app/agent_view/task_icon_mouse_tests.rs` — neue echte Render-/Inputregressionen `upper_todo_render_obeys_dock_and_visibility_without_hiding_f7_or_queue` und `suppressed_todo_releases_keyboard_focus_on_short_and_narrow_terminals`.
5. `crates/codegen/xai-grok-pager/src/app/app_view_tests.rs` — neue `global_todo_hide_releases_another_sessions_focus_and_input`; Erweiterungen an `task_and_todo_preferences_apply_globally_and_survive_new_views_and_restart` und `failed_global_visibility_write_keeps_live_intent_and_reports_the_storage_failure`.
6. `docs/fork/streams/f2-f3-sidebars.md` — ausschließlich die freigegebenen Current-truth-Stellen in Goal/Performance/State.

Keine zweite Einstellungsablage und keine neue Persistenz-/Locklogik.

## Authorierte Regressionen

- **Globale Ausblendung:** echte AppView-Frames und Eingaben. A zeigt/fokussiert Todo und öffnet dessen Suche; B blendet es per Ctrl+T global aus; zurück in A sind Fläche, Such-Eingabebalken und Todo-Fokus weg. Ein `h` fordert den Composer-Fokus an, statt den unsichtbaren Todo-Filter zu verändern.
- **Shrink/Layoutsuppression:** echte AgentView-Frames und Resize-/Tastatureingaben; von sichtbarem fokussiertem Todo zu 16 Zeilen, bei 80, 32 und tatsächlich schmalen 24 Spalten. Danach routet `h` zu `FocusPrompt`, nicht zu Todos; Queue bleibt direkt vor dem Composer. Vergrößern stellt das Todo ohne Fokusdiebstahl wieder dar.
- **Painting-Matrix:** Dock an/aus × F7 offen/geschlossen × Tasks sichtbar/unsichtbar × Todo leer/befüllt; Ctrl+T aus→an→aus, 40/24 Zeilen. Prüft gemalte Marker/Leerplatzhalter und Todo-Geometrie. F7 behält Tasks und per End erreichbare echte Todos; Queue bleibt sichtbar ohne Dockzählung/Hide-Control.
- **Session-/Storekonsistenz:** dritte View wirklich aufnehmen/aktivieren, dort Todo ändern, aus ursprünglicher View Tasks ändern, Kombination aus Store wieder laden; sessioneigene Todo-Daten bleiben erhalten.
- **Neue View ohne erfolgreichen Disk-Write:** nach fehlgeschlagener globaler Tasks-Speicherung neue View anhängen, ohne Zwischenframe zu ihr wechseln und sofort Todo toggeln. Live-Tasks-Präferenz darf dabei nicht durch deren alten Default überschrieben werden.

Alle Fälle sind Source-Authoring, keine bestandenen Tests.

## Verifikation / Lead-CI-Handoff

Ausgeführt:
- Repo-gepinnter `rustfmt +nightly-2026-08-13`, Edition 2024, auf isolierten Kopien. Nur neue/geänderte Blöcke aus dem Formatter-Ergebnis übernommen; keine flächige Neuformatierung vorhandener Hunks.
- Aktuelles Follow-up-Skript: `/tmp/grok-goal-4f7553a1dbe6/implementer/pager-restart-followup-format.py`.
- `git diff --no-ext-diff --check` für alle sechs betroffenen Pfade: erfolgreich.
- Statische API-/Routingprüfung inklusive vorhandener `set_active_pane`, `TodoPane::on_state_change`, ListPane-Suche und Sessionwechselpfad.

Nicht ausgeführt: sämtliche Grok-Builds, Rusttests, Cargo-Checks und Runtime-UI-Akzeptanz. Der frühere grüne isolierte Checkpoint aus dem Handoff verifiziert diesen Working Tree nicht. Kein aktuelles Baseline-Green oder Red-vorher/Green-nachher behauptet.

Lead soll in CI mindestens folgende Filter ausführen:
- `global_todo_hide_releases_another_sessions_focus_and_input`
- `suppressed_todo_releases_keyboard_focus_on_short_and_narrow_terminals`
- `upper_todo_render_obeys_dock_and_visibility_without_hiding_f7_or_queue`
- `task_and_todo_preferences_apply_globally_and_survive_new_views_and_restart`
- `failed_global_visibility_write_keeps_live_intent_and_reports_the_storage_failure`
- `queued_input_is_always_visible_and_owned_once_across_dock_sidebar_and_resize`
- `queued_input_stays_immediately_above_composer_with_optional_surfaces_and_resize`
- `f7_keeps_work_and_todos_when_upper_dock_and_preferences_are_off`
- `task_preference_only_changes_the_additional_upper_projection`
- `both_work_projections_open_the_same_child_with_surface_local_indices`
- `work_visibility_is_global_even_from_session_edits_and_preserves_other_writers`

Außerdem integrierte Pager-Kompilation und gepinnten Formatcheck prüfen. Dies sind auszuführende Prüfungen, keine Erfolgsnachweise.

## Quellen / gelesene Governance

- Zuerst `pager-global-display.md` und `restart-handoff.md`; in der Fortsetzung vollständig `pager-global-restart-review.md` unter demselben `/tmp/.../implementer/`-Owner.
- Repository-Kontext: `CLAUDE.md` laut Auftrag, `docs/fork/STATUS.md`, `docs/fork/streams/f2-f3-sidebars.md` erneut vor Doc-Edit, alle drei `docs/fork/skills/{sigma-review,clean-diff,rebase-able-commits}.md`, `rust-toolchain.toml`. Rekursive Suche unter `crates` fand keine weiteren Agent-Instruktionsdateien.
- Rust-Skill `/home/fg/projects/org/governance/sigma/context-modules.d/300_library.d/skills.d/rust.d/rust.skill.md` und Cargo-Guard gelesen. Kein umfassendes Sigma-Audit beansprucht.
- Sourceowner: `views/todo_pane.rs::{is_visible,desired_height,on_state_change}`, `app/agent_view/input.rs::{handle_input,set_active_pane}`, `app/agent_view/panes.rs::handle_todo_key`, `app/queue_edit.rs::editing_lock_on_pane_switch`, `app/dispatch/ctx.rs::switch_to_agent`, `app/agent_view/panel_persistence.rs`, `app/panel_settings/store.rs`, `app/agent_view/context_sidebar.rs::adopt_global_sidebars`, `views/list_pane/state/methods.rs`.
- Bestehende Render-/Queue-/Sessionregressionsowner wurden wiederverwendet, nicht ersetzt.

## Observations / offene Fragen

- P1 und P2 sind durch Source-/Routinganalyse belegt; tatsächliche Runtime-Reproduktion und CI-Ergebnis bleiben offen.
- Keine weiteren bestätigten Persistenzdefekte im eng untersuchten Flow. Der vorhandene Global-TOML-Owner und selektive Key-Updates bleiben unverändert.
- Nacheinander schreibende Stores sind keine Interprozess-Lockgarantie; keine solche Garantie wurde eingeführt oder behauptet.
- Ratatui-Terminal-Pager, keine Webapp. Browserprüfung ist kein passender Runtime-Nachweis. Kein passendes neu gebautes Artefakt wurde erzeugt/gestartet; kein laufender Prozess wurde verändert. Tatsächliche PTY-Interaktion, schmale-Terminal-Akzeptanz und Light/Dark plus mindestens drei Akzente bleiben unverified. Die nun auch wirklich schmalen Buffer-/Inputregressionen sind ausschließlich authoriert.
- Keine Fork-STATUS-Änderung, da nichts gelandet ist. Kein neuer Repository-Report.
