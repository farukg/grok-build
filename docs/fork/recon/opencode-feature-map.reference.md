---
sig: reference/v1
scope: sigma
slug: GRB
status: in-work
updated: 2026-10-18
---

# OpenCode-Fork-Featurekarte für Grok Build

Die Karte erfasst, was Faruk im OpenCode-Fork (F6/F7-Oberflächen, Display-Formen, Gesten, Kontext-Sichtbarkeit) gebaut oder
geplant hat und in Grok Build übernehmen will. Sie ist die Arbeitsgrundlage von
[GRB-001](../plans.d/grb-001-tui-parity-milestones.plan.md) und ordnet jedes Feature den Items dieses Plans zu; verbindlich ist
der Plan, die Karte ist die Quellenübersicht.

**Gespeiste Items:** M12 (Baumnavigation, Gesten), M13 (Aktionen und Gründe, Kontext-Entfernung), M14 (Farbe und Struktur),
M15 (Persistenz) aus den Abschnitten *F6*, *F7*, *Persistenz*; S-A (Anzeigeformen, Chat-Gesten) aus *Display-Formen* und *Gesten*;
S-B (Sidebar-Gesten, Hover, Aggregat) aus *F7* und *Gesten*; S-N–S-U (Kontext-Inspektion, Prompts, Werkzeuge, LLM-Sicht,
Aufrufe, Ebenen, Kontextmanipulation) aus *Kontext-Sichtbarkeit und Edit* und *Tool-Toggles*; S-V (SIG) aus *SIG-Sessions und
Kontextmodule*.

**Quellenrang bei Widersprüchen:** bindendes Design (Q2) vor Decision-Register (Q3) vor Reference (Q4) vor Archiv (Q8–Q11).
Lieferprotokoll (Q17) und Code (Q29) belegen den gelieferten Stand. Abweichungen von Grok führt GRB-001 §designbasis (G1–G13);
die Tabelle *Widersprüche* löst jeden Fall auf.

**Spalten.** *Status OCF*: `implemented | planned | unknown | superseded | rejected` wie in den Quellen. *Grok-Status*: der
Befund im Grok-Fork zum Stand 2026-10-18, geprüft gegen `docs/fork/STATUS.md` (Q28) und den Code; Werte: `vorhanden`,
`teilweise`, `in Arbeit` (Item M12–M15, umgesetzt, CI des Forks steht aus), `fehlt` (mit Item), `Grok-Abweichung`, `entfällt` (nur in
OpenCode relevant), `offen` (noch nicht bewertet).

## Quellen

Pfade relativ zum Verzeichnis dieser Datei (`grok-build-fork.d/research.d/`).

- Q1 `../../opencode-fork.d/00-index.md` — Bereichsindex OpenCode-Fork
- Q2 `../../opencode-fork.d/plans.d/opencode-tui-display-forms.design.md` — bindendes Design Display-Formen und Gesten
- Q3 `../../opencode-fork.d/decisions.d/decision-register.md` — Decision-Register (OCF-D…)
- Q4 `../../opencode-fork.d/opencode-tui-ux.reference.md` — UX-Referenz (Composer, Subagent, Kontext)
- Q5 `../../opencode-fork.d/plans.d/OCF-30-legacy-first-request-waves.plan.md` — Umsetzungsplan Display-Formen (D-/S-Schritte)
- Q6 `../../opencode-fork.d/plans.d/opencode-consolidation-and-next-waves.plan.md` — Konsolidierung und Restarbeit (HOT-, D-, S-)
- Q7 `../../opencode-fork.d/research.d/oneliner-identity-and-line-geometry-2026-08-24.findings.md` — Befund OneLiner
- Q8 `../../opencode-fork.d/archived.d/rail-basis-select-message-sidebar.plan.md` — archiviert: Sidebar-Fokus und Select-Message-Sidebar
- Q9 `../../opencode-fork.d/archived.d/transcript-interaction-defects.plan.md` — archiviert: Klick-Semantik
- Q10 `../../opencode-fork.d/archived.d/pin-toggle-set-clear.plan.md` — archiviert: Modell-Pin
- Q11 `../../opencode-fork.d/archived.d/decision-register-2026-07-30.md` — archiviert: altes Decision-Register
- Q12 `../../opencode-fork.d/specs.d/sig-navigator-ap-p1-graph-chat-sync.spec.md` — SIG-Navigator Graph/Chat-Sync
- Q13 `../../opencode-fork.d/specs.d/sig-navigator-ap-p4-panels-dna.spec.md` — SIG-Navigator Panels und Theme-DNA
- Q14 `../../opencode-fork.d/specs.d/sig-navigator-ap-p5-actions-fork.spec.md` — SIG-Navigator Aktionen, Fork und Rewind
- Q15 `../../opencode-fork.d/plans.d/sig-session-graph-navigator.plan.md` — SIG-Session-Graph-Navigator
- Q16 `../../opencode-fork.d/specs.d/mcp-tool-registration.spec.md` — MCP-Tool-Registrierung (Vorwärtsanforderung)
- Q17 `../../opencode-fork.d/completed.log.md` — Lieferprotokoll mit Commit-Angaben (Zeilenangaben als `Q17:Zeile`)
- Q18 `../../../epics.d/platform.d/opencode-rescript-migration.d/01-context-transcript.domain.md` — Kontext-Transkript-Domäne
- Q19 `../../../epics.d/platform.d/opencode-rescript-migration.d/02-context-admission-timeline.contract.md` — Kontext-Admission-Vertrag
- Q20 `../../../epics.d/platform.d/opencode-rescript-migration.d/PLT-ORM-06-context-lifecycle.active.story.md` — Kontext-Lebenszyklus (aktiv)
- Q21 `../../../epics.d/platform.d/opencode-rescript-migration.d/PLT-ORM-08-global-entry-actions.draft.story.md` — globale Eintragsaktionen (Entwurf)
- Q22 `../../../epics.d/platform.d/opencode-rescript-migration.d/03-context-transcript-implementation.plan.md` — Umsetzungsplan Kontext-Transkript
- Q23 `../../../epics.d/platform.d/opencode-rescript-migration.d/recon.d/visibility.d/02-chat-visibility-gap.md` — Befund Chat-Sichtbarkeit (überholt)
- Q24 `../plans.d/grb-001-tui-parity-milestones.plan.md` — GRB-001 Meilensteinplan
- Q25 `/home/fg/projects/org/forks/grok-build/docs/fork/design/f6-f7.md` — Grok-Design F6/F7 (Fork-Doku)
- Q26 `/home/fg/projects/org/forks/grok-build/docs/fork/streams/f2-f3-sidebars.md` — Grok-Stream Sidebars
- Q27 `/home/fg/projects/org/forks/grok-build/docs/fork/streams/f1-context-provenance.md` — Grok-Stream Kontext-Provenienz
- Q28 `/home/fg/projects/org/forks/grok-build/docs/fork/STATUS.md` — Grok-Fork-Status
- Q29 OpenCode-Code `packages/res/src/tui/` (u. a. `Tui__MessageClickGesture.res`, `Tui__DisplayForm.res`, `Tui__KindDefault.res`, `Tui__SidebarFocus.res`, `Tui__SidebarTitleGesture.res`) — Code-Befund, Ausschnitt aus dem `opencode-code`-Slice
- Q30 Grok-Code `crates/codegen/xai-grok-pager/` und `crates/codegen/xai-grok-shell/` (u. a. `timeline_panel.rs`, `context_sidebar.rs`, `context_removal.rs`) — Code-Befund, Ausschnitt aus dem `grok-code`-Slice

## Features

### Display-Formen und Karten

| Feature | Beschreibung | Status OCF | Geste/Taste | Persistenz | Quelle | Grok-Status |
| --- | --- | --- | --- | --- | --- | --- |
| Vier Display-Formen | Jede Karte hat genau eine Form: Hidden, OneLiner, Capped, Expanded (Tui__DisplayForm) | implemented | – | pro Zeile und Kind | Q2, Q3 | teilweise: `DisplayForm` hat zwei Formen (Collapsed, Expanded), F7 Message Display setzt sie je Art; Hidden und Capped geplant (S-A) |
| Hidden-Form | Rendert keine Chat-Zeile; Setzen und Zurückholen nur über Sidebar Message Display | implemented | Sidebar-Leaf Alt (Design) | pro Zeile, Session | Q2 | fehlt (S-A geplant) |
| OneLiner-Form | Höchstens eine Terminalzeile, lange Identifier entfallen | implemented | – | – | Q7 | vorhanden als `DisplayForm::Collapsed` (genau eine Zeile; M7 für User, F1-display für alle Arten) |
| Capped-Form | Gleiche Kartenhülle wie Expanded, nur das Body-Budget begrenzt | implemented | Ctrl+Klick (Chat) | – | Q2 | fehlt (S-A geplant) |
| Expanded-Form | Vollständiger Body | unknown | – | stabile Vollform | Q2 | vorhanden (`DisplayForm::Expanded`) |
| Gemeinsame Karten-Shell | Tui__MessageCard und Meta für alle Nachrichtenarten | implemented | – | – | Q5 | fehlt (S-C geplant) |
| Kinds-mal-Formen-Taxonomie | Tui__MessagePresentationKind: Anzeigeart, Default, F6-Facette, Bulk-Eignung | implemented | – | global Default, Session-Override | Q5, Q17:86 | teilweise: `MessageKind` (exhaustiv), `DisplayDefaults` und `EntryForm` vorhanden; F6-Facette und Bulk-Eignung fehlen |
| F6-Facette der Präsentationsart | Taxonomie-Facette; das Design definiert F6 nicht | unknown | – | – | Q3 | offen |
| Edit und Apply Patch getrennt | Eigene Anzeigeidentität und eigener Renderer (Tui__MutationBlock) | implemented | – | – | Q3 | offen |
| Compaction und Internal konfigurierbar | Eigene Anzeige-Settings und Bulk-Teilnehmer | implemented | – | global mit Session-Override | Q5 | offen |
| Tool-Renderer-Dispatch | Tui__ToolPartBlock ist der einzige Renderer-Dispatch | implemented | – | – | Q17:31 | offen |
| Carried-by-sibling-Marker | StepStart, Snapshot und Patch erscheinen als Marker der Nachbar-ToolRow | implemented | – | – | Q6, Q17:32 | offen |
| Reasoning-Settlement | Spinner aus Part-Clock und Turn-Fortschritt; kein endloser Spinner | implemented | – | – | Q6, Q17:91 | offen |
| Stabile Zeilen-Identität | Unveränderte Zeilen behalten ihre Objekt-Identität über Memo-Läufe | implemented | – | – | Q17:184 | offen |
| Tool-Name-Aliase | Einzige typisierte Parse-Grenze für SIG- und Base-Toolnamen | implemented | – | – | Q6 | offen |
| Sticky-Provider-Label | Tui__ProviderLabel ersetzt das hart codierte Σ | implemented | – | – | Q6 | offen |
| Shell-Kommando-Darstellung | Syntax-Highlighting in jeder Form; ab Capped Segmentierung an && und eigener Output-Toggle | implemented | Output-Toggle | – | Q5 | offen |
| Shell-Diff bei Edit-Kommandos | Editierende Befehle zeigen den resultierenden Diff | implemented | – | – | Q5 | offen |
| Thinking-ID nur bei Hover | Lange IDs ruhen im Layout und erscheinen als Hover-Layer ohne Höhenänderung (HOT-3, in-work) | unknown | Hover | – | Q6 | offen |
| Message-Kind-Filter | Sechs Arten; Default user, assistant und compaction an, tool, reasoning und internal per Opt-in | implemented | Chip-Toggle im Picker | per Layer | Q17:83, Q29 | offen |
| Farbpalette je Kind | Feste RGBA-Palette, unabhängig vom Theme | implemented | – | – | Q17:85 | in Arbeit (M14, umgesetzt, CI steht aus: Farbe je Zeilenklasse aus Theme-Feldern, keine feste RGBA-Palette; Grok-Abweichung G9) |
| Form-Gesetz Skill, Thinking, errored Subagent | Errored Subagent lässt Hidden und OneLiner weg; Expanded-Menge für Skill und Thinking strittig | unknown | – | – | Q2, Q4 | offen |
| Entry-Fähigkeitsmatrix | Edit, Dismiss, Steer, Select und Formen je Kind abgeleitet; nicht unterstützte Aktionen fehlen | planned | – | – | Q19 | offen |
| Einzeiliger Titel mit Caret | Dünne Zeile, Titel-Caret kollabiert nur den Body (Titel-Caret unabhängig von Breite) | implemented | ▼/▶ | per Instanz kv | Q17:77 | offen |

### Gesten (Klick und Modifier)

| Feature | Beschreibung | Status OCF | Geste/Taste | Persistenz | Quelle | Grok-Status |
| --- | --- | --- | --- | --- | --- | --- |
| Chat Plain-Klick | Toggle OneLiner ↔ Expanded; kein Doppelklick-Gesetz | implemented | Klick | pro Zeile, Session | Q2, Q3 | teilweise: Doppelklick-Fold vorhanden; Klick-Ziel geplant (S-A) |
| Chat Ctrl+Klick | Öffnet und fokussiert den Select-Message-Picker und wählt den Eintrag | implemented | Ctrl+Klick | – | Q2, Q5 | Grok-Abweichung: Ctrl+Klick bleibt Link-Modifier (E6, GRB-001 §abweichungen) |
| Chat Alt+Klick | Öffnet Message-Aktionen nur für den angeklickten Eintrag | implemented | Alt+Klick | – | Q3, Q29 | Grok-Abweichung: Alt+Klick öffnet das F6-Panel auf dem Eintrag (vorhanden); das Panel ist die Aktionsfläche (GRB-001 §abweichungen) |
| Chat Ctrl+Alt+Klick | Form-Zyklus OneLiner → Capped → Expanded; auf gebundenem Subagent Eintritt in das Child | implemented | Ctrl+Alt+Klick | pro Zeile, Session | Q3 | teilweise: öffnet das verlinkte Child (vorhanden, M3); Form-Zyklus geplant (S-A) |
| Subagent Plain-Klick | Betritt das Child nie | implemented | Klick | – | Q5 | offen |
| Subagent Ctrl+Alt+Klick | Betritt das gebundene Child unabhängig von running, completed oder errored | implemented | Ctrl+Alt+Klick | Child-Bindung über session_task_invocation | Q5 | vorhanden (M3: `is_open_child_click`, öffnet das verlinkte Child unabhängig vom Zustand) |
| Jüngstes direktes Child | session.child.latest für Ctrl+Alt+Down (Design) bzw. Ctrl+Down (OCF-D15) | unknown | Ctrl+Alt+Down (Design) / Ctrl+Down (D15) | – | Q2, Q3 | vorhanden (Ctrl+Alt+↓ = `SessionLatestChild`, Toast „No child session“) |
| Auswahl unterdrückt Klick | Committed Textauswahl unterdrückt den Plain-Toggle; Modifier bleiben aktiv | unknown | Klick bei Auswahl | – | Q2 | offen |
| Chrome-Klick ✕ | ✕ schließt auch bei aktiver Auswahl | implemented | Klick ✕ | – | Q9 | offen |
| Panel-Chrome nicht selektierbar | Chrome wird bedient, nicht gelesen (selectable false) | implemented | – | – | Q17:188 | offen |
| Sidebar-Titel Linksklick | Abschnitt auf- und zuklappen (nur Disclosure) | implemented | Klick auf Titel oder Caret | Disclosure-Status kv | Q5, Q29 | in Arbeit (M12, umgesetzt, CI steht aus: Klick, Enter und Space klappen den Abschnitt in F7) |
| Sidebar-Titel Rechtsklick | Bulk: Leaf-Aktion auf alle berechtigten Kinder | unknown | Rechtsklick Titel | über Kind-Default | Q2 | fehlt (S-B geplant) |
| Sidebar-Leaf Plain-Klick | OneLiner ↔ Expanded | implemented | Klick | Kind-Default, Session | Q2, Q29 | vorhanden: Klick, Space und Enter kippen `1 line` ↔ `full` je Art (F7 Message Display); Kontext-Zeilen schalten ON/OFF |
| Sidebar-Leaf Ctrl, Alt, Ctrl+Alt | Design: Ctrl OneLiner ↔ Capped, Alt Hidden ↔ Capped, Ctrl+Alt zyklisch. Code D-7: Ctrl weniger, Alt mehr, Ctrl+Alt OneLiner ↔ Capped | implemented | Ctrl, Alt, Ctrl+Alt | pro Kind und Session | Q2, Q5, Q17:33 | fehlt (S-A/S-B geplant); Grok-Bulk-Gesten Shift und Ctrl siehe M12 |
| Message Display Titel-Rechtsklick | Kollektiv OneLiner ↔ Expanded für Thinking, Compaction, Internal, Tool Results; User und Assistant unverändert | implemented | Rechtsklick Titel | Tool-Results-Gruppe global oder Session | Q2 | fehlt (S-B geplant); Bulk läuft in M12 über Shift/Ctrl+Enter, Shift/Ctrl+Klick und `+`, `-`, `*` |
| Expand-all und Collapse-all | Kein Gesetz definiert; nächste Mechanismen sind Bulk-Toggles | unknown | – | – | Q2 | in Arbeit (M12, umgesetzt, CI steht aus: Geschwister, Teilbaum, alles in F6 und F7; Tasten `+`, `-`, `*`, Shift/Ctrl+Enter, Shift/Ctrl+Klick; Shift/Ctrl+Enter in Terminals unverifiziert) |
| Hover-Tokenzahlen | Sidebar-Zeilen zeigen Tokens nur bei Hover | unknown | Hover | – | Q2, Q24 | teilweise: F7-Kontextzeilen zeigen geschätzte Tokens dauerhaft; Hover-Variante geplant (S-B) |
| Hover über Art-Indikator | Bewegt Auswahl und Scroll; Hover über Titel oder Vorschau tut nichts | unknown | Hover Indikator | – | Q9 | offen |
| Ctrl+Klick als Link-Modifier | Ctrl+Klick öffnet Link; Ctrl+Alt+Klick öffnet Subagent | planned | Ctrl+Klick, Ctrl+Alt+Klick | – | Q24 | vorhanden (E6) |

### F6 Timeline und Auswahl

| Feature | Beschreibung | Status OCF | Geste/Taste | Persistenz | Quelle | Grok-Status |
| --- | --- | --- | --- | --- | --- | --- |
| F6 Picker-Toggle | F6 öffnet bzw. schließt den Select-Message-Picker; ohne Ziel bei geschlossenem Picker unterdrückt | implemented | F6 | – | Q29, Q17:187 | Grok-Abweichung: F6 = Timeline-Panel (`ToggleTimelinePanel`) |
| F6 Fallback-Ziel | Reihenfolge: Anker, Hervorhebung, neueste Nachricht | implemented | F6 | Anker in-memory | Q29 | offen |
| F6 Sidebar-Fokus-Toggle | Wechselt Normal und Sidebar-Fokus; Sidebar bleibt offen | implemented | F6 | – | Q8, Q17:75 | offen |
| F6 Graph-Rail-Fokus | F6 fokussiert die Graph-Rail; Escape löst den Fokus | implemented | F6, Escape | – | Q29 | offen |
| Timeline-Panel öffnen | F6 erweitert die Timeline-Rail zum Panel (Modus in-memory je AgentView) | implemented | F6 | in-memory | Q30 (timeline_panel.rs) | vorhanden |
| Timeline-Spalte und Breitengate | Zweispaltige Rail; sichtbar bei Setting, Root-Session, Breite ab 60, mindestens 2 Turns | implemented | – | show_timeline (global) | Q30 (sidebars.rs), Q24 | vorhanden |
| Outline Turn, Gruppe, Eintrag | Zeilenmodell, Neuaufbau nur bei Änderung von outline_generation | implemented | – | in-memory | Q30, Q24 | vorhanden |
| Cursor auf und ab | j/k und Pfeile | implemented | ↑ ↓, j, k | – | Q30 | vorhanden; M12 begrenzt ↑/↓ auf sichtbare Zeilen |
| Gruppe einklappen oder zum Elternknoten | Links oder h | implemented | ←, h | Gruppenfaltung Session-View | Q30 | in Arbeit (M12, umgesetzt, CI steht aus): ←/h schließt einen offenen Zweig, sonst Sprung zum Elternknoten |
| Gruppe aufklappen | Rechts oder l | implemented | →, l | Session-View | Q30 | in Arbeit (M12, umgesetzt, CI steht aus): →/l öffnet einen zugeklappten Zweig, sonst erstes Kind |
| F6 Bulk-Gesten | Geschwister, Teilbaum oder jeden Zweig öffnen und schließen | planned | Shift+Enter, Ctrl+Enter, Shift/Ctrl+Klick, `*`, `+`, `-`, Shift+←/→ | – | Q24 | in Arbeit (M12, umgesetzt, CI steht aus; Ctrl+Enter ist global `InterjectPrompt`, wirkt als Bulk nur mit Panel-Fokus) |
| F6 Turn-Standard und Turn-Falten speichern | Turns starten offen oder zu; Ausnahmen je Turn | planned | `t`, `g` | Turn-Standard global oder Session, Ausnahmen Session nach Prompt-Nummer | Q24 | in Arbeit (M15, umgesetzt, CI steht aus; Gruppenfalten nicht gespeichert; Rewind löscht die Falten ab dem Ziel) |
| Sprung zum Eintrag | Enter oder f | implemented | Enter, f | – | Q30 | vorhanden |
| Fokus auf Chat | Tab wechselt vom Panel in den Chat | implemented | Tab | – | Q30 | vorhanden |
| Panel schließen | Esc oder F6 im Panelfokus | implemented | Esc, F6 | – | Q30 | vorhanden |
| Maus im Panel | Linksklick auf Zeile; Mausrad bewegt Cursor | implemented | Klick, Rad | – | Q30 | vorhanden |
| Alt+Klick öffnet Timeline auf Eintrag | Timeline-Panel fokussiert auf den Chat-Eintrag | implemented | Alt+Klick | – | Q30 | vorhanden (Grok: Aktionsfläche = F6-Panel, siehe Widersprüche) |
| Gruppe und Eintrag falten | e | implemented | e | Session-View | Q30 | vorhanden |
| Inhalt und Meta kopieren | y kopiert Inhalt, Y die Meta | implemented | y, Y | nur Zwischenablage | Q30 | vorhanden |
| Gruppen-Kopie und Gruppen-Viewer | y, Y, v auf Gruppenzeile | planned | y, Y, v | – | Q30, Q24 | fehlt (S-K geplant) |
| Block-Viewer | v öffnet den Viewer | implemented | v | – | Q30 | vorhanden |
| Rewind zum Cursor-Turn | R (Großbuchstabe) | implemented | R | bestehende Rewind-Semantik | Q30 | vorhanden |
| Aktionen am gewählten Eintrag wiederverwenden | Falten, Gruppe, Kopie, Viewer, Rewind und Sprung | implemented | bestehende Bindungen | – | Q24 | vorhanden |
| /jump-Vorschau | Slash-Befehl mit Vorschau und Cursor | implemented | /jump | – | Q24 | vorhanden |
| Timeline-Rail-Klick | Klick auf einen Tick springt | implemented | Klick | show_timeline | Q24 | vorhanden |
| Ctrl+P-Eintrag „Timeline“ | Palette-Eintrag für das Panel | planned | Ctrl+P, Timeline | – | Q24 | vorhanden (`PaletteCommand::TimelinePanel`) |
| Scroll-Sync Panel und Chat | Cursor und Chat folgen einander in beide Richtungen | planned | Down, Rad | – | Q24 | vorhanden (M2; Tests `panel_cursor_down_scrolls_scrollback`, `scrollback_scroll_moves_unfocused_panel_cursor`) |
| Gruppenzeilen-Aktionen (E7) | Aktion auf Gruppe gilt für Mitglieder; Gruppe klappt im Panel und im Chat auf | planned | e, Rechts | – | Q24, Q30 | teilweise: Falten, Rewind und Sprung auf Gruppen vorhanden; Kopieren und Viewer auf Gruppen geplant (S-K) |
| Gruppe im Chat ausklappen | Gruppenzeile klappt im Chat auf | planned | – | – | Q24 | vorhanden (Test `group_row_expand_toggles_scrollback_group`) |
| Timeline-Rail bündig links | Rail links bündig, rechte Scrollbar bleibt | planned | – | – | Q25 | teilweise: Panel liegt als linke Spalte (F2/F3); Rail-Ausrichtung ungeprüft |
| F6-Viewport-Hervorhebung | Sichtbereich in der Timeline markiert | planned | – | – | Q25 | fehlt (Fork-Design f6-f7, kein Item) |
| F6-Zeilenstil und Pfadkürzung | Semantische Zeilen, voller Pfad bei Hover | planned | Hover | – | Q25 | in Arbeit (M14, umgesetzt, CI steht aus: Zeilenfarbe und Struktur); Pfadkürzung und Hover-Detail offen |

### F7 Sidebar und Kontext-Schalter

| Feature | Beschreibung | Status OCF | Geste/Taste | Persistenz | Quelle | Grok-Status |
| --- | --- | --- | --- | --- | --- | --- |
| F7 Kontextsidebar umschalten | Öffnet und schließt die Kontextsidebar | implemented | F7 | keine für Open-Zustand | Q30 (context_sidebar.rs) | vorhanden |
| Kontext-Kategorien ON/OFF | Zeilen mit Token-Schätzung je Kategorie | implemented | – | Policy beim Shell | Q30 | vorhanden |
| Kategorie umschalten | Space, Enter oder Klick auf die Zeile | implemented | Space, Enter, Klick | context_policy.json pro Session | Q30 | vorhanden |
| Konsequenztext für OFF | Erklärt, was ausgeschaltet bedeutet | implemented | – | – | Q30 | vorhanden |
| Kontext-Policy pro Session | Persistiert im Session-Verzeichnis, nur bei Änderung | implemented | – | Session-Verzeichnis | Q30 | vorhanden |
| Schaltbare Kategorien | Skills, MCP-Katalog und Memory fehlen in der Schaltliste | planned | – | – | Q30 | teilweise: Kernanweisungen, Projektanweisungen, Umgebung, Tool-Definitionen, User-/Assistant-Runden, Reasoning, Tool-Austausch, Runtime-Hinweise und Kompaktierung schaltbar; Skills, MCP-Katalog und Memory ohne Producer (F1-core „mixed sections“) |
| Message-Display-Zeilen | Je Art 1 Zeile oder voll, per Klick, Space oder Enter | implemented | Klick, Space, Enter | Session-Scrollback, nicht pager.toml | Q30 | vorhanden |
| Globale Message-Display-Defaults | Defaults als globale Pager-Appearance | planned | – | pager.toml | Q25, Q26 | in Arbeit (M15, umgesetzt, CI steht aus: globale Ebene in `$GROK_HOME/panel-settings/global.toml`, nicht pager.toml; Ziel per Taste `g`) |
| F7-Navigation und Fokus | Up/Down, j/k, Esc, Tab, Mausrad, Klick | implemented | Up, Down, j, k, Esc, Tab | – | Q30 | vorhanden; Baumnavigation und Gesten in Arbeit (M12, umgesetzt, CI steht aus) |
| F7 Bulk-Gesten und Speicherziel | Abschnitte und Arten gemeinsam öffnen/schließen; Ziel global oder Session | planned | Shift/Ctrl+Enter, Shift/Ctrl+Klick, `+`, `-`, `*`, `g` | Formen und Abschnittsfalten global oder Session | Q2, Q24 | in Arbeit (M12/M15, umgesetzt, CI steht aus; ändert nie einen Kontext-Schalter, G5/G6) |
| Dock im F7-Körper | Dock-Aktionen unverändert im F7-Körper | implemented | – | – | Q30 (sidebars.rs) | vorhanden |
| Dock-Schmalglyphen und Detail-Popup | Glyphen bei schmalen Zeilen; Detail bei Hover oder Auswahl | planned | Hover, Auswahl | – | Q25 | fehlt (offen im Strang F2/F3) |
| Dreispaltiges Layout 25/50/25 | Bei einem offenen Panel 35/65 | planned | – | – | Q25, Q28 | vorhanden (`screen_columns`: beide offen 25/50/25, eine offen 35/65) |
| Schmal-Overlay-Modus | Esc führt zurück in den Chat | superseded | Esc | – | Q25 | entfällt (superseded) |
| Dock und Queue bei geschlossenem F7 | Ausblendung unklar | unknown | F7 | – | Q30 (f2-f3) | vorhanden (geschlossenes F7: kein Dock, keine Queue über dem Prompt) |
| Projektion der Kategorie-Policy im Request-Bau | Wirkung pro Session | unknown | – | pro Session | Q30 (f1) | vorhanden (`ContextPolicy::project` im Request-Builder, F1-core A2) |
| Sidebar-Breite Icon oder Normal | Breitenring, 42 Spalten im Normalzustand | implemented | Klick ⇔ | kv icon oder normal | Q17:65, Q29 | offen |
| Sidebar-Breite Content-Fit | Doppelklick passt an den breitesten Inhalt an, bis 50 Prozent | implemented | Doppelklick ⇔ | Wert nur im Fit-Schritt | Q29 | offen |
| Sidebar-Breite Icon, Compact, Wide (überholt) | Älterer Dreischritt mit festen Pixelbreiten | superseded | Klick ⇔ | – | Q8 | entfällt (superseded) |
| Sidebar-Fokus-Zustand | Tri-State Closed, Open mit Fokus; Fokus nur bei offener Sidebar | implemented | – | in-memory | Q17:63 | vorhanden (F6/F7-Fokus) |
| Escape löst Sidebar-Fokus | Escape gibt Fokus an Normal und lässt die Sidebar offen | implemented | Esc | – | Q8, Q17:75 | Grok-Abweichung: Esc schließt F6/F7, Tab gibt den Fokus an den Chat |
| Doppel-Escape schließt Sidebar | Zweites Escape binnen 600 ms schließt | unknown | Esc Esc | – | Q11, Q17:75, Q8 | Grok-Abweichung: Esc schließt, kein Doppel-Esc (siehe Widersprüche) |
| Klick außerhalb löst Fokus | Klick auf Chat oder Prompt; kein Modal, kein Scroll-Sprung | implemented | Klick außen | – | Q17:74 | offen |
| Aktionsmenü per Enter | Enter im Sidebar-Fokus öffnet Menü in der Palette-Komponente | implemented | Enter | – | Q8 | offen |
| Klick in Sidebar nur Fokus | Klick fokussiert und wählt, öffnet kein Menü | unknown | Klick | – | Q8 | offen |
| Sidebar kein Modal-Stack | Sidebar greift nicht auf den Modal-Stack zu | unknown | – | – | Q8 | offen |
| Select-Message als Sidebar | Non-modal und rechts angedockt; Auswahl unverändert | implemented | Pfeile, Bild, Pos1, Ende | – | Q8, Q17:69 | offen |
| Vier primäre Aktionsicons | Revert Messages Only, Edit, Delete, Fork klickbar | implemented | Klick | – | Q8 | offen |
| Klickbare Shortcut-Leiste | Klick feuert dieselbe Aktion wie der Shortcut | implemented | Klick | – | Q8 | offen |
| Sidebar-Fokusindikator | Heller Titel statt blauem Rand | implemented | – | – | Q17:73 | offen |
| Gemeinsame Chrome (InspectorPanel) | Message-Sidebar und Session Inspector teilen Chrome | implemented | ▼/▶, ↔, ▐/▭ | per Instanz kv | Q17:67 | offen |
| Sidebar links, Overlay erlaubt | Linke Sidebar; Overlay-Modus zulässig | implemented | – | – | Q17:72 | offen |
| Dreispalten-Layout Ist-Stand | Links, Transkript, rechts; nur Transkript flext | implemented | – | – | Q17:71 | offen |
| Sidebar-Abschnitte Sicht im Hover | Hover-Tokens und Aggregatsummen | unknown | Hover | – | Q2 | fehlt (S-B geplant) |
| Sidebar-Titel-Zusammenfassung | Aggregat aktiv oder gesamt, Tokens ohne Doppelzählung | planned | – | – | Q24 | teilweise: Titel zeigen `N/M ON` bzw. `N/M full` (M12); Token-Aggregat ohne Doppelzählung fehlt (S-B) |
| Dritte Sidebar-Spalte | Zusätzliche Spalte im SIG-Umfeld | planned | – | – | Q24 | vorhanden (Dreispalten-Layout, F7 als rechte Spalte) |
| Graph-Rail in Basis-Sidebar | Graph-Funktionen als neue ReScript-Owner in der linken Sidebar | planned | – | SIG-Backend hält Zustand | Q8 | offen |

### Kontext-Sichtbarkeit und Edit (LLM sieht vs. nur Anzeige)

| Feature | Beschreibung | Status OCF | Geste/Taste | Persistenz | Quelle | Grok-Status |
| --- | --- | --- | --- | --- | --- | --- |
| Kontext-Lebenszyklus request-wirksam | Sichtbare Kontextzustände gehen in die Request-Zusammenstellung ein | implemented | – | Kontext-Evidenz | Q20, Q3 | offen |
| Alle sichtbaren Snapshots konsumiert (R4) | Jeder sichtbare ContextSnapshot und ContextUpdate wird von der Request-Zusammenstellung gelesen; agent=context verboten | planned | – | ContextSnapshot / ContextUpdate | Q18 | offen |
| Kontext auf einer semantischen Zeitleiste | Ein Timeline-Owner für Reihenfolge, Platzierung, Tool-Chronologie und RequestEvidence | planned | – | – | Q3 | offen |
| Kontext-Ordnungsinvarianten | Initialer Kontext vor der ersten UserMessage; Update vor dem betroffenen Schritt | unknown | – | – | Q4 | offen |
| Pending-Entry-Identität | Eine eingereichte Nutzernachricht behält Identität und Position bis zur Beförderung | unknown | – | durable Admission | Q4 | offen |
| Pending Dismiss und STEER | Echte Steuerelemente, kein Text im Body; Dismiss nur für unbefördert, STEER am nächsten sicheren Grenzpunkt | planned | Klick Dismiss, Klick STEER | durable session_input | Q19, Q18 | offen |
| RequestEvidence nur im Inspektor | Pro Provider-Turn, nicht im Dialog, nicht an das LLM gesendet | planned | Inspektor öffnen | RequestEvidence pro Turn | Q18 | fehlt (S-R/S-S geplant) |
| Gesendete Metadaten bleiben | Agent, Provider, Modell, Denkstufe und Zeit bleiben nach Abschluss sichtbar | unknown | – | – | Q4 | offen |
| LLM-Inhalt nie stillschweigend verworfen | Sichtbare Kontextinhalte bleiben im Modell | implemented | – | – | Q29 | offen |
| Kontext-Timeline-Zeilen und Zustände | Abgeleitet aus session.next.context.updated; durable Historie | implemented | über Display-Form | abgeleitet | Q29 | offen |
| Pseudonachricht agent=context | Anzeige-only Kontext-Assistentennachricht | superseded | Klick expandiert | – | Q3 (OCF-D18), Q4 | entfällt (superseded) |
| Edit, Dismiss und Mehr auf Timeline-Einträgen | Oben rechts Edit, Dismiss, Mehr; unten bedingte Steuerung je Eintrag | planned | – | – | Q21 | fehlt (S-N/S-P geplant) |
| ContextDraft vor Session editierbar | Entwurf ist editierbar und verwerfbar, bevor eine Session existiert | planned | – | Entwurf | Q22 | offen |
| Kontext-Änderungsmodus | Ctrl+P-Befehl mit Apply, Queue, Ignore; Home-Default in TUI-KV, effektiv an der Session | planned | Ctrl+P | TUI-KV und Session | Q19 | offen |
| Änderung automatisch anwenden | Geänderte Beobachtung erzeugt ContextUpdate und neue Snapshot vor dem Request | planned | – | ContextUpdate, ContextSnapshot | Q19 | offen |
| Änderung zur Freigabe anstellen | Pending ContextUpdate; alter Snapshot bleibt aktiv | planned | – | pending ContextUpdate | Q18 | offen |
| Änderung ignorieren | Nur Diagnoseeintrag, kein Update | planned | – | Diagnose | Q19 | offen |
| Queued Update bearbeiten, ablehnen, steuern | Bearbeiten in place, ablehnen, STEER für den nächsten sicheren Grenzpunkt | planned | edit, dismiss, steer | pending ContextUpdate | Q19 | offen |
| Beobachtungs-Checkpoints | Re-Observation bei Init, Workspace-Events, Pre-Submit, Pre-Request, Kompaktierung | planned | – | – | Q19 | offen |
| Manuelles Aktualisieren | Befehl „jetzt aktualisieren“ | rejected | – | – | Q19 | entfällt (rejected) |
| Kontextquellen-Inspektion | Bekannte Wrapper als semantische Felder; rohes XML optional im Inspektor | planned | Inspektor | – | Q19 | fehlt (S-N geplant) |
| Token-Schätzung mit ≈ | Schätzungen tragen ≈; Provider-Usage nur auf StepFinished | planned | – | RequestEvidence, StepFinished | Q18 | offen |
| Historische Unveränderlichkeit | Gesendete Einträge nie in place; Korrektur als neuer Eintrag, Ausschluss ab der nächsten Grenze | planned | Edit, Dismiss | ContextExclusion | Q18 | offen |
| Leere Policy ohne versteckte Defaults | Default, Customized oder Empty je Zielgruppe | planned | – | ContextSnapshot-Policy | Q19 | offen |
| Dismiss-All und Titel-Anweisung | Kann Main-Policy und Titel-Generierung abschalten | planned | DismissAll | Entwurf, dann Session | Q19 | offen |
| Atomare Admission beim Submit | Session, Snapshot, Nachricht und Request committen gemeinsam oder gar nicht | planned | – | eine Server-Transaktion | Q19 | offen |
| Batch Dismiss-All und Dismiss-Marked | Ohne Marken Dismiss-All; mit Marken Dismiss-Marked; Bestätigung bei gesendeter Historie | planned | Aktionsleiste | Ausschlüsse für gesendete Einträge | Q19 | offen |
| Mark/Unmark an Warteschlange | Sichtbare Buttons entfernt | superseded | – | – | Q19 | entfällt (superseded) |
| SetModel in Aktionsleiste | Modellwahl wandert in den Prompt-Footer | superseded | – | – | Q19 | entfällt (superseded) |
| Submit-Bestätigung über TUIs | Composer leert erst nach dauerhafter Bestätigung; alle TUIs sehen dieselbe Pending-Wahrheit | planned | – | server-seitig | Q19 | offen |
| Step-Grenzen als dünne Linien | StepStarted und StepFinished minimal, aufklappbar nur mit Inhalt | planned | aufklappen | StepFinishMetadata | Q19 | offen |
| Einheitliche Entry-Schale | Eine Hülle für Akzent, Metadaten, Edit, Dismiss, Mehr, Steer und Marker | planned | oben rechts, Mehr, Steer | – | Q19 | offen |
| Kontext-Entfernung über Rewind | Bisher nur Rewind (ReplaceHistory) | implemented | R | chat_history.jsonl, updates.jsonl | Q24, Q30 | vorhanden |
| Kontext-Entfernung über d | Entfernt Turn oder ToolExchange aus dem Modellkontext, mit Bestätigung | implemented | d, Delete | ContextItemsRemoved im Session-Replay | Q30, Q24 | vorhanden (M5); M13 (umgesetzt, CI steht aus): Verdikt vor dem Senden, sofortiges Senden beim zweiten `d`, Anzeige folgt der Antwort der Shell |
| Refusal für nicht entfernbare Einträge | Meldung bei unbekannter Identität | implemented | d | – | Q30 | in Arbeit (M13, umgesetzt, CI steht aus): benannter Grund je Eintragsart (zehn Gründe) statt der generischen Meldung |
| Entfernungs-Ergebnis als Toast | Toast für Erfolg oder Ablehnung | implemented | – | – | Q30 | vorhanden; M13 (umgesetzt, CI steht aus): das Ergebnis nennt entfernte und übersprungene Einträge mit Grund, `PersistFailed` entfernt die Zeilen und warnt |
| Entfernungs-Guards in der Shell | Guard-Logik und Persistenz über persist_xai_update_only | implemented | – | ContextItemsRemoved | Q30 | vorhanden; M13 (umgesetzt, CI steht aus): Klassifikation je Ref, Schreiben vor Marker, Backend-Aufrufe samt Blob; offen: Rewind-Picker, Prefire, `x_search`-Blob |
| Teilentfernung Text und Reasoning | Feinkörnige Entfernung ohne Eintrag | planned | – | – | Q24 | fehlt (S-I geplant) |
| Eine Quelle für Kontext-Entfernung (E3) | ContextItemRef, RemoveItemsOutcome, eine Wahrheit | planned | – | chat_history.jsonl, updates.jsonl | Q24 | vorhanden (M5: `ContextItemRef`, `x.ai/session/remove_items`, Replay-Marker `context_items_removed`) |
| Nicht gesendet-Markierung je Item | Session-Scope, verfällt bei Löschung oder Kompaktierung | planned | – | Session | Q25 | fehlt (S-R geplant; Zeilenmarker ●/○/⊘ in M14) |
| Kontext-Schalter wirkt auf Request | Wirkt die Sidebar-Kategorie auf die Request-Zusammenstellung | unknown | – | pro Session | Q30 | vorhanden (F1-core A2; Kompaktierung fasst nur Gesendetes zusammen) |

### Tool-Toggles

| Feature | Beschreibung | Status OCF | Geste/Taste | Persistenz | Quelle | Grok-Status |
| --- | --- | --- | --- | --- | --- | --- |
| Tool-Results-Gruppe | Message Display für Tool Results, global oder Session; Tool-Zeilen erben | implemented | Rechtsklick Titel | Global oder Session | Q2, Q29 | fehlt (S-B geplant) |
| Tool-Zeile erbt von Tool-Results | Tool-Zeile folgt der Gruppe, solange kein Override gesetzt ist | implemented | Leaf | tool_form:<key> | Q29 | fehlt (S-B geplant) |
| Per-Tool-Override | Override je Werkzeug | unknown | – | – | Q24 | offen |
| Tool-Toggles und LLM-Sichtbarkeit | Ob Display-Overrides die LLM-Anfrage beeinflussen, ist ungeklärt | unknown | – | – | Q2 | Abgrenzung: Anzeigeform je Tool-Art (S-B) ist nicht die Verfügbarkeit für das LLM (S-Q geplant) |
| Tool-Name-Aliase (Tool-Toggle-Schlüssel) | Typisierte Parse-Grenze für Toolnamen | implemented | – | – | Q6 | offen |

### Persistenz (global und Session)

| Feature | Beschreibung | Status OCF | Geste/Taste | Persistenz | Quelle | Grok-Status |
| --- | --- | --- | --- | --- | --- | --- |
| Globale Default-Ebene mit Session-Override | Session-Override erst nach explizitem Edit; Zurücksetzen auf Global | implemented | – | Global und Session, übersteht Neustart | Q2, Q3 | in Arbeit (M15, umgesetzt, CI steht aus: `global.toml` und `sessions/<id>.toml`; Override erst nach explizitem Edit mit Ziel `g`; nur ausdrückliche Werte) |
| Context- und Message-Display-Settings | Global und Session übersteht Rebuild und Neustart | implemented | – | Global und Session | Q2 | in Arbeit (M15, umgesetzt, CI steht aus; Formen und Abschnittsfalten); die LLM-wirksame Kontext-Policy liegt pro Session in der Shell (`context_policy.json`), Global-Default geplant (S-T) |
| Per-Row-Entscheidungen pro Session | Operator-Entscheidungen je Zeile | implemented | Klick, Ctrl+Alt | kv row_forms:<sessionId> | Q5, Q29 | teilweise: Fold-Zustand in der View; Turn-Ausnahmen pro Session in M15 (Prompt-Ordinal) |
| Kind-Defaults ohne Materialisierung | Lesezeit-Auflösung, global und Session | implemented | – | global kv; session_form:<sessionId>:<key> | Q29 | in Arbeit (M15, umgesetzt, CI steht aus: `DisplayDefaults` hält nur explizite Wahlen, die Ebenenwahl kopiert nichts, jede gesetzte Form wird ausdrücklich gespeichert) |
| Settings-Scope je Domäne | Global oder Session je Bereich | implemented | ContextScopeControls | settings_scope:<domain>:<sessionId> | Q29 | in Arbeit (M15, umgesetzt, CI steht aus: ein Speicherziel für beide Panels, Taste `g`, Anzeige in der Fußzeile) |
| Sidebar-Abschnitt-Disclosure | Auf- oder zuklappen über kv | implemented | Titel-Klick | kv openKey | Q29 | in Arbeit (M12 Bedienung, M15 Persistenz; beide umgesetzt, CI steht aus) |
| Sidebar-Scope und Disclosure überstehen Neustart | Global- und Session-Scope mit Persistenz | unknown | – | Neustart-fest | Q2 | in Arbeit (M15, umgesetzt, CI steht aus: F7-Abschnitte und Formen kommen nach dem Neustart aus den Dateien; F6-Turn-Falten je Session nach Prompt-Nummer, Gruppenfalten nicht gespeichert) |
| Conceal-Modus | Gespeicherter Operator-Modus vor Config-Default | implemented | – | gespeichert, sonst Config | Q6, Q3 | offen |
| Grok-Settings show_thinking_blocks und Appearance | Globale Settings-Schalter | implemented | Settings-UI | global | Q24 | vorhanden |
| Grok-Feature active_agent_messages | Feature-Flag in config.toml, Standard aus | implemented | – | config.toml [features] | Q24 | vorhanden |

### Navigation (Baum, Fokus)

| Feature | Beschreibung | Status OCF | Geste/Taste | Persistenz | Quelle | Grok-Status |
| --- | --- | --- | --- | --- | --- | --- |
| Sitzungswechsel-Parität | Direkt geöffnete Sessions zeigen ‹ i/n › und klickbare Pfeile wie Dashboard-Sessions | planned | Links, Rechts, Ctrl+Alt+Links/Rechts | – | Q24 | vorhanden (M1; Test `direct_open_session_renders_cycle_switcher_like_dashboard_open`) |
| Ctrl+[ und Ctrl+] Sitzungswechsel | Bereits vorhanden, aber unsichtbar | implemented | Ctrl+[, Ctrl+] | – | Q24 | vorhanden (Ctrl+[ = Esc ohne Kitty-Protokoll) |
| Geschwister-Wechsel in der Child-Ansicht | Ctrl+Alt+Links/Rechts wechselt zu Geschwister-Subagent | implemented | Ctrl+Alt+Links, Ctrl+Alt+Rechts | – | Q24 | vorhanden (Ctrl+Alt+←/→ in einer Child-Ansicht wechselt in Startreihenfolge, `children_of`) |
| Einzige Sitzungswechsel-Reihenfolge | Eine Funktion für Dispatch und Render, gecacht | planned | – | in-memory | Q24 | vorhanden (M1: `cycle_order` und `dispatch_session_cycle`, Test `session_cycle_order_matches_dispatch_with_workspace_rows`) |
| Subagent-Recency-Owner | Ein Owner für Start und Resume; Alt-Klick und Ctrl+Down zielen auf das jüngste Child | unknown | Alt+Klick, Ctrl+Down | Recency-Store nicht spezifiziert | Q3 | teilweise: `children_of` ordnet nach Startzeit; Resume-Recency ungeprüft |
| Subagent-Block stabil | Ein Eintrag in place über running, backgrounded, completed, failed | unknown | – | – | Q4 | offen |
| Subagent-Hintergrund | Deaktiviert, bis Abschluss den Startblock in place aktualisiert | planned | – | – | Q4 | offen |
| Subagent-Aufruf-Normalisierung | Nicht unterstützte Denkstufe auf nächste Stufe; ungültiges Modell entfällt; Tiefe maximal 3 | planned | – | – | Q3 | offen |
| Subagent-Preview-Paging | Älterer Verlauf wird bei erreichter Kante nachgeladen | implemented | Rad | lokal bis zur ältesten Kante | Q6 | offen |
| session.child.latest | Ein Owner: jüngstes direktes Child für Ctrl+Alt-Pfade | implemented | Ctrl+Alt+Down (Design) | Child-Bindung über session_task_invocation | Q6 | offen |
| Legacy Message-Action-Registry | Geschlossene Schlüsselmenge für Dialog und Picker | implemented | Shortcut-Bindings | delegiert an SDK | Q14 | offen |
| Legacy Revert löscht Folgenachrichten | Löscht alles nach dem Klickziel; widerspricht dem Ziel ohne Löschung | implemented | revert | Server-Löschung | Q14 | offen |
| Legacy Fork dupliziert Session | Erzeugt neue Session per session.fork und navigiert dorthin | implemented | fork | neue Session auf Server | Q14 | offen |
| Rebase-Aktion | Kein Rebase-Reduzer im Kernel; v1-Integrationspfad ist Merge, Rebase = V4-Roadmap | planned | – | – | Q14 | offen |
| ReParent als eigene Aktion | Offene Produktentscheidung (C5) | unknown | r (vorgeschlagen) | – | Q14 | offen |
| Panel-Modus Sidebar/Overlay | Ein Knopf mit Glyphe; Ausblenden nur über ✕ | planned | Klick auf erste Kopfglyphe | Panel-Modell | Q13 | offen |
| Inspektor-Transparenz-Zyklus | Vier Stufen auf dem Hintergrund (HEAD: drei) | planned | Klick ● | – | Q13 | offen |
| Breiten-Zyklus mit verzögertem Klick | Icon, Compact, Expanded; Klick um 250 ms verzögert | planned | Klick ⇔ | Breitenachse | Q13 | offen |
| AutoFit mit exakter Wiederherstellung | Doppelklick passt an; zweiter Doppelklick stellt Modus und Breite her | planned | Doppelklick ⇔ | gespeicherter Vormodus | Q13 | offen |
| TitleOnly-Form | Caret klappt das ganze Panel auf die Titelzeile | planned | Caret ▼/▶ | Form persistent | Q13 | offen |
| Einzelne Kopfzeile | ▼ Titel … ⇔ ▐ ● ✕, Kürzung zuerst Hinweis, dann Titel | planned | – | – | Q13 | offen |
| Ein PanelChrome für Stack, Fleet, Inspektor | Parametrischer Chrome für drei Panels | planned | – | – | Q13 | offen |
| Zweiachsige Rail | Modus und Breite getrennt; Migration der kv-Schlüssel; blockiert durch Entscheidung C2 | planned | – | sig_<rail>_mode, sig_<rail>_width | Q13 | offen |
| Einachsige Rail mit Toggle | Zyklus hidden, icon, compact, expanded | implemented | Ctrl+Alt+S (Baum), Ctrl+Alt+I (Fleet) | kv sig_*_rail | Q13 | offen |
| Theme-DNA mit sieben Rollen | Feste Farbrollen für SigTheme | planned | – | – | Q13 | offen |
| Tasks und Todos mit einem Statusmodell | Ein Glyphen- und Zählmodell | planned | – | – | Q13 | offen |
| Gemeinsame InspectorRow | Muted Label links, getönter Wert rechts | planned | – | – | Q13 | offen |
| Vollständige Kopier-Quelle | Eine Autorität für CopySource, Selektion, Geste; ehrliche Clipboard-Meldung | planned | Kopieren | – | Q3 | offen |

### SIG-Sessions und Kontextmodule (nur geplant)

| Feature | Beschreibung | Status OCF | Geste/Taste | Persistenz | Quelle | Grok-Status |
| --- | --- | --- | --- | --- | --- | --- |
| Sessions als Graph-Knoten ohne sichtbare Wurzel | Sessions sind Top-Level-Knoten | planned | – | – | Q15 | offen |
| Turns in Provider-Chronologie | Turn-Knoten in chronologischer Reihenfolge | planned | – | – | Q15 | offen |
| Subagent-Zweige unter dem Spawn-Turn | Verschachtelbar, mehrere Ergebnisse als Kinder | planned | – | – | Q15 | offen |
| Typisierte Bibliotheks- und Modulknoten | Gruppiert je Bibliothek, sortiert nach logischer Modulchronologie | planned | – | – | Q15 | offen |
| Modellierte Fokuszustände | Graph-Cursor, Chat-Fokus, externer Route-Fokus, Sprunghistorie | planned | – | – | Q15 | offen |
| Session-Follow über route.sessionID | Ein Effekt für fünf Wechselpfade | planned | Palette, Ctrl+Alt+Links/Rechts, Enter | – | Q12 | offen |
| Session-Follow ohne Echo und ohne Jumplist | Graph-Cursor setzen ohne Quellereignis | planned | – | – | Q12 | offen |
| Pager „N von M“ gleich Graph-Cursor | Hauptsession-Zähler; Subagents ausgenommen | planned | – | – | Q12 | offen |
| Graph führt Chat bei j und k | Chat scrollt zur Turn-Nachricht des Cursors | planned | j, k | – | Q12 | offen |
| Chat führt Graph bei Scroll | Nutzerscroll bewegt Graph-Cursor zur sichtbarsten Nachricht | planned | Rad | – | Q12 | offen |
| Sync-Fokusbalken | Akzentbalken an der synchronisierten Nachricht | planned | – | – | Q12 | offen |
| Programmatische Scrolls mit expectedTop | Ersetzt den 600-ms-Timer des Mockups | planned | – | – | Q12 | offen |
| Aktionsstreifen unter Cursor | a öffnet kindabhängige Aktionen; Esc oder Bewegung schließt | planned | a | – | Q14 | offen |
| Kindabhängige Knotenaktionen | Turn: Rewind, Fork, Löschen, Umschreiben, Kind ändern; Branch: Merge, Freeze, Checkout | planned | Tasten offen | – | Q14 | offen |
| Batch-Aktionen | Schnittmenge der Aktionen über mehrere Knoten | planned | Batch-Taste nicht belegt | – | Q14 | offen |
| Rewind als Fork am Turn plus Checkout | Alte Turns bleiben, werden gedimmt | planned | Rewind | Git-artige Branch-Records | Q14 | offen |
| Fork als Geschwister-Branch | Ohne Checkout; Entscheidung Q2 offen | planned | Fork | Kernel-Branch-Record | Q14 | offen |
| Merge Branch in Parent | Kernel-Reducer existiert | planned | Merge | Kernel-Merge-Commit | Q14 | offen |
| ForkRewind-Backend-Naht | Wählt Legacy- oder Git-Pfad | planned | – | – | Q14 | offen |
| Kernel-TUI-Keybind Branch und Deliver | Noch nicht verdrahtet | planned | keine | – | Q29 (Sig__KernelReducers.res) | offen |
| Session-TUI-Keybind-Verdrahtung | Noch nicht verdrahtet | planned | keine | – | Q29 (Sig__Session.res) | offen |
| SIG-Stack-Baum-Sidebar und Descend | Ungeklärt | unknown | unbekannt | unbekannt | Q29 (Sig__Dto.res) | offen |
| Cockpit-Sidebar-Collapse und Auswahl | Persistenz beim Aufrufer, nicht verifiziert | unknown | unbekannt | Aufrufer-Owner | Q29 (Cockpit__Sidebar.res) | offen |

### Sonstiges (Pin, Modell, Composer, Performance; nur TUI-relevant)

| Feature | Beschreibung | Status OCF | Geste/Taste | Persistenz | Quelle | Grok-Status |
| --- | --- | --- | --- | --- | --- | --- |
| Pin setzen oder lösen am Modellknopf | Ungepinnt setzt auf aktives Modell; gepinnt löst, Modell bleibt | implemented | Klick Modellknopf | session.model | Q10, Q17:111 | offen |
| Pin-Farbe | Modellname in Fehlerfarbe bei gepinntem Modell | implemented | – | abgeleitet aus session.model | Q10, Q17:457 | offen |
| ctrl+o Einmal-Override | Gilt für eine Nachricht; ändert den Pin nicht | implemented | ctrl+o | keine | Q10 | offen |
| Null-Sentinel zum Lösen | Fehlendes Modell = keine Änderung, null = lösen | implemented | – | session.model | Q10 | offen |
| Admission schreibt Pin nicht mehr | Senden überschreibt den Pin nicht | implemented | – | Pin getrennt von Historie | Q10 | offen |
| Modelllose Sendung | Folgt Pin, sonst letztem Modell, sonst Provider-Default | implemented | – | liest session.model | Q10 | offen |
| Fork übernimmt Pin | Agent und Modell in die neue Session | implemented | fork | neue Session-Zeile | Q10, Q17:113 | offen |
| Toast bei Pin setzen oder lösen | Meldung „Session gepinnt auf …“ bzw. „Pin gelöst …“ | planned | – | – | Q10 | offen |
| V2-Clear-Pfad | Außerhalb des Umfangs | planned | – | – | Q10 | offen |
| Session-Pin-Reconcile | Ausblenden bei fehlendem Modell via vollem Ersatz | implemented | – | TUI-Sync-Store | Q10 | offen |
| HOT-10 Sidebar-Flackern | Behoben in 550aec348e | implemented | – | – | Q6 | offen |
| HOT-7 Performance-Modus | Ctrl+P schaltet ästhetisches Rendering ab (Highlighting, Animationen) | unknown | Ctrl+P | persistenter Schalter | Q6 | offen |
| HOT-7 sofortiger Interrupt | Doppel-Escape unterbricht idempotent | unknown | Doppel-Esc | – | Q6 | offen |
| HOT-8 verlustfreies Senden | Senden sofort, Composer leert erst nach dauerhafter Aufnahme | unknown | Enter | durable session_input | Q6 | offen |
| HOT-2 Pending-Eingabe | Eine serverseitige Wahrheit; Dismiss und STEER als Steuerelemente | unknown | Klick | server-seitig | Q6 | offen |
| HOT-3 stabiles Streaming | Keine Remounts zwischen Deltas | unknown | – | – | Q6 | offen |
| HOT-4 Terminal-Metadaten | Agent, Provider, Modell, Denkstufe bleiben sichtbar | unknown | – | unveränderliches Profil | Q6 | offen |
| HOT-5 sicherer Composer-Clear | Leert erst nach Ack der exakten Attempt-ID (P2) | unknown | – | Entwurf bis Ack | Q6 | offen |
| HOT-9 EditBuffer und Ziel-Timer | Fehler „EditBuffer destroyed“ und Timer-Reset beheben | unknown | – | – | Q6 | offen |
| Filter-bewusste Abdeckung | Restarbeit: Display-Formen und Nachweise nach Filter | unknown | – | – | Q6 | offen |
| Aktionsvollständigkeit | „Keine Aktion“ nur als typisierte Abwesenheit mit Grund | unknown | – | – | Q6 | offen |
| Nicht-Erfolg-Benachrichtigungs-Modal | Klick auf Spinner öffnet Fehler-Modal ohne Transkripttext | unknown | Klick Spinner | – | Q6 | offen |
| Aktuelle TUI-Reihenfolge | HOT-0 bis HOT-10, dann S-5 und T-0 bis T-5 | planned | – | – | Q6 | offen |
| Composer-Entwurf bis Ack wiederherstellbar | Text bleibt bis zur Bestätigung erhalten | unknown | – | Entwurf vor Submit | Q4 | offen |
| Gesperrtes Senden sichtbar | Enter führt nie stumm ins Leere; Force und Steer geben Rückmeldung | unknown | Enter, Force-Send, Steer | – | Q4 | offen |
| Prompt-Historie | Reicht bis zur ältesten Eingabe; übersteht Neustart und Fork | unknown | – | persistent | Q4 | offen |
| Kanonisches Highlighting konsumieren | Nicht selbst besitzen oder duplizieren | unknown | – | – | Q4 | offen |
| Ersetzte Gesten aus der Referenz | Anzeige- und Gestenstichpunkte bis 21.08. überholt | superseded | – | – | Q4 | entfällt (superseded) |
| Ersetzte Vier-Form-Gesten (D-098) | Ctrl+Alt schaltete OneLiner/Capped; ersetzt durch OCF-D11 | superseded | Ctrl+Alt | – | Q11 | entfällt (superseded) |
| Ersetzte Klick-Semantik | Einfachklick schaltet Collapse, Doppelklick öffnet Menü; 280 ms und 350 ms | superseded | Klick, Doppelklick, Ctrl+Klick | – | Q9 | entfällt (superseded) |
| Zweiter Doppelklick-Detektor im Select-Dialog | 350 ms; entfernt zugunsten eines Gesten-Owners | superseded | Doppelklick | – | Q9 | entfällt (superseded) |
| Auswahl gewinnt über Klick | Committed Auswahl unterdrückt Plain, Ctrl, Alt und Titel-Gesten | implemented | Klick bei Auswahl | – | Q9 | offen |
| Hover markiert nur den Art-Indikator | Hover auf Titel oder Vorschau tut nichts | unknown | Hover | – | Q9 | offen |
| F6 schließt offenen Picker | Auch bei leerem Transkript und ohne Anker | implemented | F6 | – | Q9, Q17:187 | offen |
| Sidebar-Fokus gelöst durch Außenklick | Siehe oben | implemented | Klick außen | – | Q17:74 | offen |
| Transkript bleibt nach Submit sichtbar | Neue Nachricht wird ergänzt, Verlauf bleibt | implemented | Submit | Message-Store | Q9 | offen |
| Grok: Subagent-Steer- und Resume-Composer (M6) | Eingabe und Senden im Takeover | planned | Eingabe, Senden | Feature-Flag | Q24 | vorhanden über W1-S/W2/W3 (Child-Ansicht mit Composer, Prompts per Session-ID; `active_agent_messages` über die Config-Vorlage an) |
| Grok: Subagent-Token-Anzeige (M8) | Tokens in Aufgabenbereich, Dock und Takeover-Zeilen | planned | – | – | Q24 | vorhanden (`1a8ac47b`) |
| Grok: Session-Switcher-Header ‹ i/n › | Siehe Navigation | planned | Klick Pfeile | – | Q24 | vorhanden (M1) |
| Grok: Subagent-Ctrl+Alt+Down (M3) | Siehe Navigation | planned | Ctrl+Alt+Down | – | Q24 | vorhanden (`SessionLatestChild`) |
| Grok: Ctrl+Alt+Klick öffnet Child (M3) | Öffnet das verlinkte Child der angeklickten Karte | planned | Ctrl+Alt+Klick | – | Q24 | vorhanden (M3) |
| MCP-Beschreibung normalisieren | Einmal bei Registrierung, nicht pro Turn | planned | – | Registry-Wert; Overrides in opencode.json | Q16 | offen |

## Widersprüche

Auflösungsregel für OpenCode-Aussagen: bindendes Design (Q2) vor Decision-Register (Q3) vor Reference (Q4) vor Archiv (Q8–Q11);
Lieferprotokoll (Q17) und Code (Q29) belegen den gelieferten Stand und überstimmen veraltete Pläne und Archivaussagen, nicht
das bindende Design. Für Grok-Aussagen gilt der Code, dann `STATUS.md` (Q28), dann die Fork-Designs, dann GRB-001. Weicht
Grok bewusst vom OpenCode-Design ab, führt GRB-001 §designbasis die Abweichung als Grok-Entscheidung (G1–G13); die Spalte
„Grok-Folge“ nennt sie.

| Aussage A | Aussage B | Quelle A | Quelle B | Auflösung | Grok-Folge |
| --- | --- | --- | --- | --- | --- |
| Sidebar-Leaf: Ctrl OneLiner ↔ Capped, Alt Hidden ↔ Capped, Ctrl+Alt zyklisch | Sidebar-Leaf in D-7: Ctrl weniger Inhalt, Alt mehr Inhalt, Ctrl+Alt OneLiner ↔ Capped; Hidden-Gesten entfernt | Q2, Q3 | Q5, Q17:33 | Design und Register (Rang 1–2) gelten; die D-7-Umsetzung ist die OpenCode-seitig zu prüfende Abweichung | S-B folgt dem Design; Bulk-Gesten in M12 sind eine eigene Grok-Entscheidung (G5) |
| Jüngstes Child per Ctrl+Alt+Down | Jüngstes Child per Ctrl+Down (D15) | Q2 | Q3 | Design gilt | Ctrl+Alt+↓ (vorhanden, M3) |
| Sidebar-Titel-Klick nur Disclosure | Sidebar-Titel-Klick schreibt Content-Zustand für alle Einträge (onToggleAll) | Q2 | Q5 | Design gilt | Titelklick = nur Disclosure (M12) |
| Message-Display-Bulk: Thinking, Compaction, Internal, Tool Results | Bulk nur Thinking und Tools | Q2 | Q5 | Design gilt | G7: M12-Bulk erfasst alle Arten, Angleichung offen (S-B) |
| Chat Ctrl+Klick öffnet Picker | Ctrl+Klick ist Link-Modifier in Grok (E6) | Q3 | Q24 | Register gilt für OpenCode; Grok-Abweichung G2 | Ctrl+Klick bleibt Link-Modifier |
| Chat Alt+Klick öffnet Message-Aktionen | Grok: Alt+Klick öffnet das F6-Panel; A22/A23 | Q3 | Q30, Q24 | Register gilt für OpenCode; Grok-Abweichung G3 | Das Panel ist die Aktionsfläche; ein Karten-Menü bleibt S-I |
| F6 als Facette der Präsentationsart | F6 als Picker-Toggle, Sidebar-Fokus-Toggle, Graph-Rail-Fokus und Grok-Timeline-Taste | Q3 | Q29, Q8, Q30 | Das Register definiert F6 nicht als Taste; je Oberfläche eigene Belegung | F6 = Timeline-Panel (vorhanden) |
| Ctrl+Alt schaltet alle vier Formen (OCF-D11) | Ctrl+Alt schaltet OneLiner/Capped (D-098) | Q3 | Q11 | Register gilt; Archiv überholt | keine |
| Chat-Plain-Klick schaltet OneLiner ↔ Expanded (OCF-D12) | Plain-Klick schaltet nur Section-Collapse (D-049) | Q3 | Q11, Q9 | Register gilt; Archiv überholt | S-A: Klick-Ziel `OneLiner ↔ Expanded` (G4) |
| Kein verzögertes Doppelklick-Gesetz (OCF-D12) | Doppelklick öffnet Sidebar (D-020); Breiten-Content-Fit per Doppelklick; 350-ms-Timer (D-040) | Q3 | Q11, Q9, Q29 | Register gilt für den Chat; Doppelklick nur für die Breitensteuerung der OpenCode-Sidebar | Doppelklick faltet im Chat weiter, bis S-A das Klick-Ziel liefert; Breitenring entfällt (G12) |
| Auswahl unterdrückt nur den Plain-Toggle (OCF-D12) | Auswahl unterdrückt alle Gesten, CommittedSelection einziger Klassifikator | Q3, Q2 | Q9 | Register und Design gelten | S-A |
| Doppel-Escape schließt Sidebar binnen 600 ms (D-017) | Escape schließt nie; einziger Schließweg ist ✕ | Q11, Q17:75 | Q8 | Das Lieferprotokoll gilt (einfaches Esc verlässt den Fokus, Doppel-Esc binnen 600 ms schließt); die Q8-Aussage „nie“ meint das einfache Esc. Design und Register schweigen | Esc schließt F6/F7, Tab gibt den Fokus an den Chat (G11) |
| FocusableSidebar als eigene Komponente geplant | FocusableSidebar gelöscht, Endzustand InspectorPanel | Q8 | Q17:68 | Gelieferter Stand gilt; Archivziel überholt | keine |
| Breitenring Icon, Compact, Wide (archiviert) | Breitenring Icon oder Normal mit 42 Spalten (D-038) | Q8 | Q11, Q17:65 | Neuerer Stand gilt; Q8 überholt | feste Spalten (G12) |
| Hover-Tokenzahlen nur bei Hover (Design) | Grok M8: Tokens dauerhaft in Aufgaben und Dock | Q2 | Q24 | Design gilt für Sidebar-Zeilen; M8 betrifft andere Flächen | F7-Kontextzeilen zeigen Tokens dauerhaft (G8), Hover-Variante S-B |
| Bulk-Sektionen Instruction Files und Environment (Design) | Grok-Kategorien `ProjectInstructions` und `Environment` schaltbar, Skills/MCP/Memory ohne Producer | Q2 | Q30 | Design gilt; Grok hängt an der Shell-Inklusionssteuerung | Bulk ändert nie einen Kontext-Schalter (E12, G6) |
| Hidden nur über Sidebar Message Display (Design) | Hidden über Per-Kind-Defaults (S-A) | Q2 | Q24 | Design gilt; der Per-Kind-Default in F7 Message Display ist diese Fläche | S-A: Hidden nur über F7 Message Display |
| Ctrl+Alt+Klick auf Child in jedem Zustand (Design) | früher Grok M3 nur bei completed | Q2, Q5 | Q24 | Design gilt | vorhanden (`is_open_child_click`, jeder Zustand) |
| Alle Kategorien schaltbar (Grok-Design f6-f7) | Skills, MCP-Katalog und Memory nicht schaltbar (Code) | Q25 | Q30 | Code gilt | `docs/fork/design/f6-f7.md` im Fork ist zu korrigieren (siehe Ergebnisbericht) |
| Message-Display-Defaults global über pager.toml (f6-f7) | Je Session, nicht global persistiert (STATUS, f2-f3, Code) | Q25 | Q28, Q26, Q30 | Code und STATUS gelten | M15 liefert die globale Ebene in eigenen Dateien (E10) |
| Context-Policy über x.ai/session/context_policy (f6-f7, Code) | x.ai/context/policy (f1) | Q25, Q30 | Q27 | Code gilt | `streams/f1-context-provenance.md` im Fork ist zu korrigieren |
| F6 unbelegt (früher GRB-001) | F6 öffnet das Timeline-Panel (Code) | Q24 | Q30 | Code gilt; GRB-001 ist angeglichen | F6 = Timeline-Panel |
| Kontext-Entfernung nur per Rewind (früher GRB-001) | Entfernung per d (Code) | Q24 | Q30 | Code gilt; GRB-001 ist angeglichen | M5 auf main, M13 umgesetzt (CI steht aus) |
| Sidebar-Ctrl-Klick bewirkt Compact-Wechsel (DisplayForm-Kommentar) | Ctrl+Links bindet LessContent (SidebarTitleGesture) | Q29 | Q29 | Code gilt; Kommentar im OpenCode-Code | keine |
| OCF-D10 im Register ist verstümmelt (Pipes im Zellinhalt) | Die vier Formen stehen vollständig im Design | Q3 | Q2 | Design gilt; die Registerzeile ist in OpenCode zu reparieren | keine |
| OCF-D17 steerContext ausgeliefert | Archivierte Liste nennt steer als nächsten offenen Slice | Q3 | Q17:124 | Gelieferter Stand gilt; Restarbeit eigener Eintrag | keine |
| HOT-5 und HOT-8 besitzen beide den Composer-Clear | Master benennt keinen Owner | Q6 | Q6 | HOT-8 besitzt Clear nach Ack, HOT-5 ist Teilmenge | keine |
| Index: HOT-0 bis HOT-6 als aktuelle Reihenfolge | Master: HOT-7 bis HOT-10 vor HOT-6 | Q1 | Q6 | Master gilt | keine |
| Residual message-kind taxonomy offen | D-1 Presentation-Taxonomie accepted | Q6 | Q17:31 | D-1 gilt; Restposten schließen | G1: Grok hat die Taxonomie als `MessageKind` |
| Layout-Status implementiert | Layout-Entscheidung 25/50/25 planned | Q28 | Q25 | Code gilt: `screen_columns` liefert 25/50/25 und 35/65 | vorhanden |

Zeilenangaben `Q17:n` beziehen sich auf `completed.log.md`.
