# Grok: Instructions-Rollen und Prompt-Caching

## Implementierungsauftrag und Handoff-Ursprung

Ursprung 2026-10-28: Faruks Auftrag „für alles was im grok fork passieren muss erstelle eine handoff-datei, das werde ich an eine andere session übergeben“.
Der anschließende Auftrag, die Übergabe ins Grok-Repository zu verschieben, repo-konform zu benennen und in die bestehende Planung zu integrieren, ist dokumentarisch abgeschlossen; keine Implementierung ist damit erfolgt.
Diese Übergabe bündelt ausschließlich die geplanten Grok-seitigen Arbeiten; Gateway und Sigma-Konfigurationsgenerator sind externe Abhängigkeiten.
Planungsowner: [GRB-001, S-O/S-T-Verfeinerung](/home/fg/projects/org/governance/sigma/context-modules.d/500_work.d/areas.d/grok-build-fork.d/plans.d/grb-001-tui-parity-milestones.plan.md#instructions-cache) und [aktive Fortsetzung, geplanter Folgeumfang](/home/fg/projects/org/governance/sigma/context-modules.d/500_work.d/areas.d/grok-build-fork.d/plans.d/grb-f7-request-display.plan.md#planned-instructions-and-cache-follow-up). Die aktuelle Feature-Pause und die Actual-request-Abgrenzung bleiben bestehen.
Faruk aktivierte `/smart-leader-lead`. Technische Implementierungsdetails unten sind Forschungsempfehlungen; der folgende Inhaltsauftrag ist ausdrücklich von Faruk entschieden.
**Verbindlicher Nachtrag:** Die zentral verwalteten §MOD-Coding- und Authoring-Regeln sollen als echte `developer`-Anweisungen beim Modell ankommen, mit möglichst vollständiger fachlich relevanter Abdeckung. Höchste Output-Qualität hat Vorrang: Violations und Code Smells erkennen und innerhalb des zuständigen Umfangs ursächlich beheben; korrektes vorhandenes Design erkennen und erhalten. Pauschales Umschreiben guten Codes und stilles Akzeptieren bekannter Mängel verletzen das Ziel. Umfangsüberschreitende Defekte mit konkretem Befund und Remediation benennen.
**Weiterer verbindlicher Nachtrag:** Faruk verlangt fallabhängige Kontrolle darüber, welche Regelteile in welchem Umfang und in welcher Rolle ankommen. Die gemessenen ~136 KiB sind der Umfang der fünf Quelldateien, keine vorgeschriebene Vollbeladung jeder Anfrage. Aufnahme/Auswahl, Zielrolle und Größenbudget sind getrennt steuerbar zu machen; bestehende Profil-/Agent-/Sitzungskonfiguration wiederverwenden und die effektive Auswahl mit Größen und Herkunft anzeigen. Das §MOD-Developer-Profil bleibt sein Qualitäts-Schwerpunkt; die konkrete Zusammenstellung kann je Aufgabe variieren.
Ziel der Folgesession: Diesen §MOD-Auftrag einschließlich fallabhängiger Inhaltsauswahl, herkunftsautorisierter Instructions-Platzierung, vollständigem Cache-/Usage-Vertrag und sicherer effektiver Request-Inspektion umsetzen. §MOD-Inhalte kommen aus den kanonischen verwalteten Quellen, nicht aus beliebigen Repository- oder Tool-Inhalten.
Keine pauschale Hochstufung sämtlicher AGENTS-/Workspace-/Personal-Inhalte und keine behauptete Kosten- oder Kontingentersparnis. Gleiche §MOD-Regeln möglichst genau einmal ausliefern; fachlich relevante Inhalte bei Budgetproblemen nie still abschneiden, sondern Größe/Quelle/Grenze sichtbar machen und die vollständige, gezielt geladene Abdeckung planen.

## Current state/next steps

### Befund und Verifikation

- Read-only-Recherche abgeschlossen; keine Implementierung, Builds, Tests oder Inferenzanfragen für diesen Auftrag.
- Öffentliche Dokumentation wurde laut Rechercheübergabe am 2026-10-28 direkt abgerufen.
- Gateway `/v1/models` wurde dabei live geprüft: Luna, Sol und Astra liefern `api_backend=responses`; ihr generierter Katalog nutzt derzeit `use_responses_lite=true`.
- Aktueller Grok-HEAD read-only bestätigt: `75f5ff0ad4dc70177d9e632451abc926188a5c23`.
- Externe HEADs bestätigt: sigma-crates `3554a93887b3a7c0ab49b441f2eead5140141e06`, Sigma `3360024fe6ba3be4b178edde3db23a6e14ca54b9`.
- Grok enthält umfangreiche vorbestehende Änderungen einschließlich Manifest/Lockfile, Sampling-Typen, Shell, Agent, Chat-State und Fork-Dokumentation; Sigma ist ebenfalls nicht sauber.
- `docs/fork/STATUS.md` und der einschlägige Abschnitt in `docs/fork/design/f6-f7.md` wurden gelesen. Letzterer besitzt bereits Request-Projektion/Exklusion; diese Übergabe ersetzt ihn nicht.
- STATUS beschreibt einen älteren, exakt gebundenen Release-Artefaktstand, nicht die Abnahme des heutigen HEAD. Keine neue Runtime-/Test-Abnahme ableiten.
- Die Repo-Dokumente beschreiben CI-basierte Release-Verifikation und lokale Ausführungsbeschränkungen. Das sind repo-lokale Prozesshinweise; maßgeblich bleiben die übergeordneten Workspace-/Faruk-Gates und die tatsächliche Runtime. Beschränkungen eines früheren gejailten Agenten werden durch die Aufgabenübergabe nicht vererbt.

### Rollen, Größen und Doppelungen — ergänzende Untersuchung

- Responses unterstützt `system`, `developer` und `user`. Leeres Top-Level-`instructions` heißt nicht leerer Developer-Kontext: Grok sendet input/system, unser Codex-Adapter projiziert diesen vorhandenen Text nach developer; Lite-Tools sind ein eigener additional_tools/developer-Eintrag. §MOD ist aktuell User-Kontext; sein Developer-Transport ist umzusetzen.
- Im geprüften öffentlichen Create-Schema und den geprüften lokalen Rollen-Schnittstellen ist kein separates numerisches Developer-/User-Textmaximum ausgewiesen. Das gemeinsame Modell-/Requestbudget bleibt maßgeblich; daraus keine unbegrenzten Nachrichten ableiten.
- Öffentliche GPT-6-Luna/Sol/Astra-Modellseiten nennen je 1.050.000 Kontext-, 922.000 maximale Input- und 128.000 Output-Tokens. Das sind API-Werte, keine Zusage für Codex-Abokonten. Lokal enthält `openai_body.rs:73–78,116–118` einen experimentellen Codex-Kontextwert 872.000; effektive Route/Entitlement vor Budgetierung prüfen.
- Grok `xai-grok-sampling-types/src/types.rs:1176–1182` definiert 50 MiB Requestgrößen-Vorgabe für Responses/ChatCompletions; modellaufgelöstes `max_request_bytes` kann abweichen. Der geprüfte Image-Budgetpfad beweist keine vollständige Text-Hardcap-Prüfung und kein eigenes Rollenlimit.
- Codex-RS `project_doc_max_bytes` ist standardmäßig 32.768 Bytes kumuliert über Projekt-AGENTS-Dateien (`config/src/config_toml.rs:334–336`, `core/src/agents_md.rs:68–94,142–178`). Dieses Loaderbudget gilt weder allgemein für Developer noch automatisch für Grok; für die §MOD-Sammlung darf es nicht still wiederverwendet werden.
- Standard-/Legacy-Prefix und separater AGENTS-Reminder sind bei synchroner Definition über `include_browser_verification` gegenseitig exklusiv (`prompt_build.rs:556–600`, `prompt/context.rs:211–215`); zusätzliche Reminder werden durch `conversation_has_project_instructions` verhindert.
- **Offener konkreter Doppelungsfall:** Der erfolgreiche Custom-Template-Pfad sammelt Regeln unabhängig vom Flag (`prompt_build.rs:565–580,650–700`); bei Flag=false und Template-Rendering der Rules-Placeholders kann zusätzlich der separate Reminder auftreten. Vor Änderung tatsächliche Request-Fixtures nachweisen; diese Sitzung wurde nicht per effektivem Request-Capture ausgezählt.
- Der AGENTS-Loader liest Bodies direkt und dedupliziert kanonische Pfade (`agents_md.rs:127–135,313–346,389–407`); er expandiert den literalen `@/home/fg/projects/AGENTS.md`-Verweis in CLAUDE.md nicht. Verweis plus separat geladener Body ist keine zweite Vollkopie. Inhaltliche Überlappungen bleiben gesondert zu prüfen.

### 1. Herkunft und tatsächliche Rollen vor dem Flattening

- Typisierte, herkunftsautorisierte Fragmente für Produkt-, Agent-/Rollen-, Workspace- und persönliche Guidance an bestehenden Produzenten modellieren.
- Vor String-Flattening klassifizieren; vorhandene Provenienz, ContextPolicy und SourceOccurrence erweitern statt paralleler Policy-Engine.
- Verbindlich: zentral verwaltete, fachlich relevante §MOD-Coding-/Authoring-Regeln als echte developer-Nachrichten liefern; ihr bisheriger User-Transport erfüllt Faruks Nachtrag nicht. Quellen und wirksame Abdeckung sichtbar machen. Produkt-/Rollenbasis bleibt effektiv developer; sonstige Repo-/Personal-Regeln standardmäßig user.
- Bestehende SIG-Kontext-Komposition für die §MOD-Sammlung wiederverwenden. Herkunft, Vollständigkeit, Reihenfolge und Aktualisierung am verwalteten Produzenten festlegen; keine handkopierte zweite Regel-Sammlung im Grok-Template erzeugen. Falls dafür Sigma-Kompositions-/Generatorarbeit nötig ist, als externe Abhängigkeit koordinieren.
- §MOD-Fragmente anhand strukturierter Quellen genau einmal platzieren; gemischte historische AGENTS-/Custom-Inhalte bleiben ohne verlässliche Fragmentgrenzen opaque. Semantische Dopplungen und stilles Kürzen ausdrücklich prüfen; keine Text-Heuristik zur Entfernung einsetzen.
- Ordnervertrauen erlaubt Lesen, erteilt aber keine automatische Developer-Autorität. Repo-Inhalt, Frontmatter oder Tags dürfen sich nicht selbst hochstufen.
- Bestehende vertrauenswürdige Config-/Provider-/Model-/Session-Schichten nutzen; konkrete Policy-Auflösung zuerst festlegen.
- Echte User-Turns, Tool-Ausgaben, abgerufene Dokumente, Memory und Umgebungsdaten bleiben von einer solchen Hochstufung ausgenommen.
- Request-time-Rollenprojektion bevorzugen, nicht ungeprüft eine persistierte Developer-Variante durch alle historischen Formate ziehen.
- Provider-default und exakt angeforderte Rolle unterscheiden; tatsächlich wirksame Codex-Abbildung system → developer sichtbar machen.
- Opaque/alte Custom-Templates erhalten; keine XML-/Tag-Auswertung zur Autoritätsableitung.
- Beide produktiven Regelnpfade erhalten und deduplizieren: Shell `prompt_build.rs:580–600,609–700` und Agent `prompt/context.rs:202–215` mit `include_browser_verification`.
- Exkludierte Inhalte nicht wieder einschleusen; Regeln genau einmal. Späte Regeln nicht in vergangene Turns hochziehen, Historie nicht still umschreiben.
- Default-Verhalten, Bytes und Reihenfolge bis zur expliziten Policy-Auswahl bzw. bewussten Rekonstruktion erhalten.

### 2. Typisierten Cache-Vertrag und SDK-Unterstützung ergänzen

- Aktuelle Grok-Konstruktoren/Serializer senden weder `prompt_cache_options` noch `prompt_cache_breakpoint`.
- Gepinnter SDK-Stand: `InputTextContent` enthält nur `text`; zuständigen gepflegten SDK-Fork/Schema-Owner bestimmen, dort ergänzen und Grok-Pin aktualisieren.
- Niemals Cargo-Checkout/Cache patchen. Root-Manifest enthält den Git-Pin `95b52ebdedf42143083cf3d6f0e0be7c84e9c808` und Workspace-Abhängigkeit `async-openai` 0.33.0.
- Typisierte Cache-Konfiguration und Content-Marker über Request-Vertrag bis zum Responses-Serializer transportieren.
- Tatsächlicher ChatGPT-/Codex-Endpunkt-Support ist unbewiesen: Kompatibilität explizit behandeln, nicht aus öffentlicher API-Dokumentation ableiten.
- Empfehlung als Ausgangspunkt: automatisches/implizites 30m-Caching; statischen Block-Breakpoint nur bei nachgewiesenem Nutzen ergänzen.
- Der erste Developer-Block-Endpunkt ist bereits implizit; Top-Level-`instructions` kann keinen Content-Breakpoint tragen.
- Tools, Namen, Schemas und Reihenfolge stabil halten; Historie chronologisch append-only, dynamisches Datum/Memory/cwd hinter stabile Instructions.

### 3. Usage vollständig weiterreichen

- SDK `shared/response_usage.rs:18–29` verliert laut Recherche `cache_write_tokens` und TTL-Aufschlüsselung; gepflegte Abhängigkeit samt Pin korrigieren.
- Vorhandene Grok-Usage-/Session-/UI-Verbraucher erweitern; gewöhnlichen Input, Cache-Reads, Cache-Writes, Output und Reasoning nicht doppelt zählen.
- Input-Kategorien sind disjunkt; GPT-6-30m/12h-Writes nicht als Anthropic-5m/1h umetikettieren.
- Gateway-Raw-Responses können Felder bewahren, normalisierte Gateway-Abrechnung verliert Writes: externe Voraussetzung für End-to-End-Nachweis.

### 4. F7 als Einstieg für Inspektion, Kontrolle und Bearbeitung erweitern

Detaillierter UX-/Autoritäts-/Abnahmeowner: [Planned F7 instruction controls and content editing](../../../design/f6-f7.md#planned-f7-instruction-controls-and-content-editing). Geplant auf Faruks Auftrag „Dann mach du die Planung und update die docs“; keine Implementierungsfreigabe für die vollständige neue Lane und keine neue Release-Abnahme. Aktuelle Actual-request-Grenze und Feature-Pause bleiben bestehen.

- **F7 → Context** erschließt alle unterstützten Instruction-Kontrollen: Aufnahme, bekannte Regelteile, angeforderte Rolle/provider-default, Größen-/Tokenbudget, unterstützte Cache-Policy/-Grenzen und bestehenden Scope unabhängig steuern. Kategorie → Quelle/Block → Abschnitt nur mit Konstruktionsprovenienz; opaque Blöcke bleiben ganz auswählbar und vollständig einsehbar. Keine neue Locked-Kategorie, Hochstufung untrusted Quellen, Tag-/Regex-Chirurgie oder stille Kürzung. Aktueller User-Input und gültige Tool-Paarung bleiben erhalten.
- Bestehenden Baum, Aktionssystem und Center-Viewer verwenden: Context-Klick selektiert nur, Enter inspiziert; Space/Disclosure und Message-Display-Scope behalten ihre Bedeutung. Effektive Policy-Quelle, Fragmentart, Ursprung/Layer/Overrides, angeforderte/wirksame Rolle samt Adaptionsnachweis, konfigurierte/gemessene Größen, Aufnahmeentscheidung/Grund, Key-Herkunft und Cache-Grenzen zeigen. §MOD als verwaltetes Developer-Qualitätsprofil, nicht als pauschale ~136-KiB-Pflichtladung oder Developer=Projekt-Gleichsetzung.
- Vollständigen tatsächlichen fachlichen Text im bestehenden Viewer und dieselbe gewählte Quelle in Copy anbieten, nicht nur Zusammenfassung oder Metadaten. **Quelle/aufgelöste aktuelle Instructions**, spätere seiteneffektfreie **PREVIEW** und unveränderlicher **SENT**-Capture mit Session/Request/Attempt/Modell/Provider klar trennen. Effektiven Request aus Chat-State verwenden, nicht Pager-Scrollback oder ungefilterte persistierte Historie. Caller-Capture beweist keinen post-Gateway-Payload oder Modellkonsum; fehlende Daten Unknown, bekannte Abbildung nur als abgeleitet kennzeichnen. Versteckter OpenAI-Systemprompt ist nicht zugänglich behauptet.
- Sichere Längen/Hashes statt routinemäßiger Rohprompt-/Secret-Protokollierung; verfügbare Wire-Ansicht mit ihrer tatsächlichen Capture-Grenze und Schutzgrenzen kennzeichnen. Inspektionsinhalt nie in Modellhistorie einspeisen. PREVIEW benutzt spätere echte Preparation ohne Inferenz, Admission, Queue-Verbrauch oder History-Mutation; es erweitert nicht rückwirkend den aktuellen Actual-request-Viewer.
- Bearbeiten ab F7 über den normalen Composer (S-P), mit benanntem Ziel, Quelle und Scope; Entwurf/Anhänge bewahren. Apply sendet genau eine typisierte Shell-Operation, keinen User-Turn/Modellaufruf. Cancel stellt den Entwurf wieder her und lässt das Original bestehen; Fehler halten Edit und bestätigten wirksamen Zustand samt Ursache. Managed/generated Quellen über richtigen Owner oder ausdrückliches scoped Override bearbeiten; Reset entfernt nur das gewählte Override zum Source/Default.
- Shell bestätigt Revision, Scope, Source/Ziel und Session-Identität; stale Auswahl/Revision/Sessionwechsel ablehnen, Konkurrenz autoritativ abgleichen, Pending nicht als Applied darstellen. Bestätigte Änderungen gelten an der nächsten Preparation-Grenze, nie für laufende/gesendete Requests. Bestehende Warnung mit Cancel/Session-Alternative und Profil-/Agent-/Provider-Auflösung wiederverwenden, keine neue redundante Merge- oder Bestätigungslogik.
- Spätere Abnahme nach der Designmatrix: Keyboard/Maus, schmale Breiten/Unicode, Root/Child, große Texte/Copy, Entwürfe/Fehler/Konkurrenz, Resume/Fork/Compaction mit vollständiger aktiver Policy und unveränderlichen historischen Captures; keine Wiederbelebung ausgeschlossener/ersetzter Regeln. Genau einmalige bekannte Fragmente über User-/Developer-Platzierung, beide Regelpfade und Custom-Template/Reminder nachweisen. Cache-Felder/Usage/Provider-Effekte end-to-end belegen, nicht garantieren; keine per-frame Reads/Hashes/Tokenisierung. §MOD-Verhaltensabnahme aus dem verbindlichen Nachtrag bleibt erforderlich und wird nicht durch Rollenfelder ersetzt.

### 5. Optionaler separater Folgeumfang: Effort-Updates

- `configuration_update` ist in der exhaustiven Grok-Item-Konvertierung bislang nicht unterstützt; nicht als fertige Fähigkeit darstellen.
- Request-level Effort unverändert lassen; Updates an ursprünglicher chronologischer Stelle, keine benachbarten Updates.
- Resume/Fork/Compaction und Konflikte mit automatischer Trunkierung/Compaction zuerst modellieren und prüfen.
- Gateway bewahrt/entfernt Updates bereits capability-gesteuert über `supports_reasoning_effort_updates`; stilles Strippen bei Fallback benötigt externen Vertrag.

### 6. Optional: gemessene Key-Policy-Untersuchung

- Hauptpfad: Session-ID → `conv_id` → Responses `explicit_key.or(conv_id)`; Sidecalls verwenden absichtlich den Parent-Key.
- Keine Prompt-Hashes oder Zeitstempel beimischen; bestehende Defaults erhalten.
- GPT-6-Keys sind optional und können abgerechnete sitzungsübergreifende Wiederverwendung trennen; separaten Accounting-Effekt messen.
- Globaler Worker-Key ist weder Standard noch Privacy-Autorisierung; gleiche Modell-/Auth-Verarbeitungsgrenze voraussetzen.

### Nächster konkreter Schritt und spätere Abnahme

1. WIP/Status erneut prüfen und sichern; gepflegten SDK-Owner und Pin bestimmen; Fragment-, Autoritäts- und Lifecycle-Entwurf bestätigen.
2. Kernumfang in kohärenten Grok-Inkrementen implementieren; externe Verträge koordinieren, nicht Gateway-/Generator-Änderungen still übernehmen.
3. Im zutreffenden Verifikationspfad Wire-Golden-Fixtures durch Fork + Gateway für Lite/nonLite und andere unterstützte Backends prüfen; bei lokalen Rust-Builds den SIG-Build-Vertrag anwenden.
4. Trust-/Exklusions-/Konfliktfälle, legitime Ausnahmen, genau einmalige Regeln beider Pfade und alte Custom-Templates abdecken. §MOD-Abnahme mit unabhängig bewerteten Fällen: echte Violations/Code Smells erkennen und passend beheben; bereits guter Code bleibt korrekt und wird ohne konkreten Bedarf nicht umgebaut. Fehlalarme und unnötige Refactors getrennt von übersehenen Defekten auswerten.
5. Resume/Fork/Sidecalls/Modellwechsel/Compaction sowie Tools/Schema/Reihenfolge und exakte Usage-Weitergabe nachweisen.
6. Synthetische Inferenz auf tatsächlicher Subscription-Route nur im autorisierten Implementierungs-/Sicherheitsumfang: kalt/warm/TTL/Rollenänderung, gesamte Input-Kosten, Latenz und Verhalten getrennt messen. Hohe Hit-Rate beweist keine Ersparnis.

## Existing references

### Grok-Owner (absolute Pfade; Zeilen sind Rechercheanker)

- [Fork-Status](/home/fg/projects/org/forks/grok-build/docs/fork/STATUS.md): dokumentierter CI-Release-Pfad und Artefaktprovenienz; repo-lokale Hinweise unterliegen den Workspace-Gates.
- [F6/F7-Request-Projektion](/home/fg/projects/org/forks/grok-build/docs/fork/design/f6-f7.md): bestehende Context-/Exklusionshoheit, besonders §§1–2.
- [Cargo-Pin](/home/fg/projects/org/forks/grok-build/Cargo.toml): gepflegte SDK-Abhängigkeit statt Cargo-Cache ändern.
- [Responses-Serializer](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-sampling-types/src/conversation/responses.rs): `instructions: None` (136–170), System als input/system (207–214), User als input/user (215–223), Key-Fallback (149–153).
- [Conversation](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-sampling-types/src/conversation.rs), [Provenienz](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-sampling-types/src/context_provenance.rs), [Policy](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-sampling-types/src/context_policy.rs), [Occurrence](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-sampling-types/src/source_occurrence.rs): vorhandene Typ-Owner.
- [Request Builder](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-chat-state/src/actor/request_builder.rs): effektive Sicht nach Exklusion/Pruning/Memory, 22–219.
- [Prompt-Template](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-agent/templates/prompt.md), [Rendering](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-agent/src/prompt/context.rs): Extend/Full/Template/Custom (268–297).
- [User-Regeln](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-agent/src/prompt/user_message.rs): Regeln 43–94, Produktprosa derzeit User-Regel 180–187.
- [Discovery](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-agent/src/prompt/agents_md.rs): Home/Configured/Project, 18–41.
- [Config](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/agent/config.rs), [Provider](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/agent/model_providers.rs): bestehende Auflösungsowner.
- [Remote-Katalog](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/remote/client.rs): konsumiert `api_backend`, 649–657.
- [Sidecalls](/home/fg/projects/org/forks/grok-build/crates/codegen/xai-grok-shell/src/session/acp_session_impl/side_call.rs): Parent-Key, 102–129.

### Externe Abhängigkeiten — nur koordinieren

- [Modellprojektion](/home/fg/projects/org/platform/sigma-crates/shared/models/src/project.rs): Responses-Backend 30–59,131.
- [Gateway-Body](/home/fg/projects/org/platform/sigma-crates/infra/gateway/src/openai_body.rs): Codex system → developer 98–112,1478–1490; Lite stellt additional_tools als developer voran, native Lite fügt keinen Fallback ein.
- [Wire-Vertrag](/home/fg/projects/org/platform/sigma-crates/shared/openai-wire/src/responses.rs): unbekannte Top-/Nested-Felder 146–155,260–319; Provider-Body startet aus bewahrter Map (`openai_body.rs:1442–1538`).
- [Capabilities](/home/fg/projects/org/platform/sigma-crates/infra/gateway/src/routing/candidate_metadata.rs): Updates 1176–1179; Unsupported-Entfernung `openai_body.rs:1003–1011`, nonLite-Autocompaction-Konflikt offen.
- [Normalisierte Usage](/home/fg/projects/org/platform/sigma-crates/infra/gateway/src/routing/completion.rs): Cache-Write-Verlust 330–379 am eigenen Usage-Modell beheben.
- [Sigma-Template](/home/fg/projects/org/governance/sigma/agents/templates/agent-config-generator/grok-config-toml.template): neue Config-Felder nur über SSOT/Generator, keine installierten/generierten Dateien handpatchen.
- Verbindliche §MOD-Quellen unter `/home/fg/projects/org/governance/sigma/context-modules.d/200_policy.d/laws.d/modeling.d/`: `authoring.law.md`, `quality.law.md`, `reuse.law.md`, `naming.law.md`, `codegen.law.md` und deren fachlich erforderliche aktive Regeln. Sprache: `/home/fg/.grok/skills/rust/SKILL.md`. Vor der Zusammenstellung bestehende atom-/composite-basierte Kontext-Komposition und managed Rule-Owner auf Wiederverwendung prüfen; die Workspace-Zusammenfassung `/home/fg/projects/AGENTS.md` ist bereits User-Kontext und kein eigenständiger Developer-Nachweis.
- Größenmessung 2026-10-28 für diese fünf Dateien: authoring 61.771 Bytes, quality 37.756, reuse 20.351, naming 3.238, codegen 16.098; zusammen **139.214 Bytes (~136 KiB)**, ohne Rust-Skill/weitere Quellen. Das ist eine Bytemessung, keine Tokenzahl; den tatsächlich gerenderten Developer-Block mit passendem Tokenizer und dem wirksamen Modellbudget messen.

### Öffentliche Verträge aus der Recherche vom 2026-10-28

- [Responses-Create-Schema](https://developers.openai.com/api/reference/resources/responses/methods/create.md): unterstützte Rollen und gemeinsame Input-/Output-Parameter; kein ausgewiesenes separates Developer-/User-Textmaximum.
- API-Modellgrößen: [Luna](https://developers.openai.com/api/docs/models/gpt-6-luna.md), [Sol](https://developers.openai.com/api/docs/models/gpt-6-sol.md), [Astra](https://developers.openai.com/api/docs/models/gpt-6-astra.md).
- [Prompt Caching](https://developers.openai.com/api/docs/guides/prompt-caching.md): GPT-5.6+/GPT-6 ab 1024 sichtbaren Tokens; implizite/explizite Content-Grenzen; `prompt_cache_options.mode`, TTL 30m (Default, Writes 1.25×) oder 12h (2.5×); `prompt_cache_breakpoint {mode: explicit, ttl?}`.
- Vollständig gerenderter berechtigter Prefix und Einstellungen müssen passen; ein gleicher Key garantiert keinen Hit. Usage: `input_tokens_details.cached_tokens`, `cache_write_tokens`, `cache_write_tokens_details.ttl_30m_tokens/ttl_12h_tokens`.
- [Reasoning](https://developers.openai.com/api/docs/guides/reasoning.md): Update-Semantik. [Model Spec](https://model-spec.openai.com/2026-08-18.html): root > system > developer > user > guideline; tatsächliche Rolle statt Tag-Text.
- [Codex Pricing](https://developers.openai.com/codex/pricing): API-Preise sind nicht ChatGPT-Plan-Kontingente. Keine eigene öffentliche Rollentarifierung und keine belegte Subscription-Quota-Ersparnis.

## Mandatory context

- Arbeitsrepository: `/home/fg/projects/org/forks/grok-build`; größere Arbeiten im autorisierten Geschwister-Worktree nach Workspace-Regeln, keine WIP-Verluste/Resets/Reverts.
- Der korrigierte Fluss ist native Responses → Gateway → Codex-Rollenabbildung, nicht die Annahme eines reinen Chat-Completions-Pfads.
- AGENTS-/Workspace-/Personal-Regeln sind heute User-Kontext, nicht insgesamt Teil des primären Systemprompts.
- Workspace-/Faruk-Gates haben Vorrang vor repo-lokalen Prozesshinweisen. Maßgeblich sind die tatsächliche Runtime und die wirksamen Aufträge; keine Jail- oder Mount-Beschränkungen durch Aufgabenvererbung übernehmen. Veröffentlichung, Installation und Serviceänderung jeweils nach ihrem eigenen geltenden Vertrag behandeln.
- Lokale Rust-Verifikation läuft über `sig build` nach Registrierung und Laden der relevanten Skills; kein direktes Cargo und kein Status-Polling. CI-Evidenz bleibt an den tatsächlich geprüften Commit gebunden.
- Entscheidungen/Forschung und echte Nutzeranforderungen auseinanderhalten. SDK-/Gateway-Support und Nutzeneffekt bleiben bis zum Nachweis offen.

## Ongoing work/limits

- Keine laufenden Rechercheworker, keine ausstehenden Build-Ereignisse; temporäre Forschungsberichte wurden nach Synthese bewusst gelöscht und werden nicht referenziert.
- Übergebender Parent: ehemals SIG Beatrice, native Grok-Session `01a18293-ed54-7320-b6bd-f95b3a6f4d8b`; eigene SIG-Bindung nach Erstellung verifiziert und erfolgreich abgemeldet. Die Folgesession registriert ihre eigene Identität.
- Dauerhaftes Ursprungstranskript: `/home/fg/.grok/sessions/%2Fhome%2Ffg%2Fprojects/01a18293-ed54-7320-b6bd-f95b3a6f4d8b/updates.jsonl` (gezielt lesen; keine Secret-Inhalte weitergeben).
- Der Integrationsauftrag umfasst diese verschobene Übergabe, die beiden verlinkten bestehenden Sigma-Planungsowner und einen kleinen Planungsverweis im Fork-Status. Keine Implementierung, Builds/Checks/Tests, Installationen, Services, Veröffentlichung, Commits oder verschachtelten Agents im Handoff-Auftrag. Frühere Session-/SIG-Bindungen sind Provenienz; der Lead muss seine aktuelle Registrierung bei späterer Koordination gegebenenfalls aktualisieren.
