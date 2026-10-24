# Restart continuation — blockiert vor Quelländerungen

## Status

Die vier verlangten Handoff-/Vertragsdateien wurden zuerst gelesen. Anschließend wurden aktueller Working Tree und Indexinventar sowie die vorhandenen typisierten Admission-/Storage-/Receiverowner gelesen. Kein vorhandener Output wurde zurückgesetzt, keine Sicherung reappliziert. Keine Produktdatei wurde geändert. Die Zustelltransaktion ist **nicht fertiggestellt und nicht verifiziert**.

Die bindende Worker-Vorgabe verlangt einen grünen Ausgangsstand vor Produktänderungen und bei defektem Ausgangsstand einen Stopp. Im aktuellen, vom Vorgänger hinterlassenen Source bestehen bereits statisch eindeutige Typ-/Signaturwidersprüche. Lokale Grok-Builds/Tests sind ausdrücklich ausgeschlossen; Commit/Push und damit neue CI-Verifikation sind diesem Worker ebenfalls untersagt. Deshalb wurde vor weiteren Produktänderungen gestoppt, nicht eine Fertigstellung behauptet. Die früher grüne isolierte Revision aus restart-handoff.md deckt diesen Source nicht ab.

## Konkrete Ausgangsdefekte

Alle Repositorypfade sind relativ zu `/home/fg/projects/org/forks/grok-build/`.

- `crates/codegen/xai-grok-shell/src/session/storage/jsonl/completion.rs:197–200`, `append_completion_sync`: liest `projection.mode` und referenziert `xai_chat_state::completion::CompletionProjectionMode::{Resume,Fresh}`. Der aktuelle Typ `crates/codegen/xai-chat-state/src/completion.rs:49–56` enthält kein `mode`; das Modul definiert keinen `CompletionProjectionMode`. Außerdem berücksichtigt jener Match weder `Continue` noch `TurnClosed` aus dem aktuellen `CompletionProjectionAck`.
- `crates/codegen/xai-grok-shell/src/session/storage/jsonl/delivery_receiver.rs:44`, `recover_delivery_state`, konstruiert `DeliveryExecution::Interrupted` ohne `parent_task`; derselbe Fehler besteht in `recover_deliveries_owned` bei Zeile 239.
- Dieselbe Datei, `append_delivery_sync:199`, konstruiert `DeliveryExecution::Processing` ohne `parent_task`.
- Dieselbe Datei, `reconcile_delivery_terminals:117` und `finish_delivery_turn_sync:227`, ruft `settle_turn` mit zwei Argumenten auf. Der aktuelle Owner `crates/codegen/xai-grok-shell/src/session/delivery_receiver.rs::DeliveryReceiverState::settle_turn` verlangt `turn_id`, `parent_task`, `origin`, `progress`. Diese neue Parent-Task-/Origin-Fencing-Semantik darf nicht durch Weglassen der Parameter zurückgebaut werden.

Diese Befunde sind Quellbelege, kein ausgeführter Compilerlauf. Die Liste ist ausdrücklich keine vollständige Prüfung des umfangreichen WIP.

## Vorhandene Owner erhalten

- ChatState: `src/{completion,input_projection}.rs`, `InputAdmission`, `ParentTask`, `InputProjection`, `CompletionProjectionAck`, generischer `StrictAppendError`.
- Shell: `session/acp_session_impl/completion_admission.rs`, `current_turn_admission`, `project_subagent_completion`, `commit_observed_subagent_tool_result`, sichere Completion-Drainpfade.
- Receiver: `extensions/session_delivery.rs`, `session/acp_session_impl/delivery_receiver.rs`, `session/delivery_receiver.rs`, `session/receiver_execution.rs`, `session/storage/jsonl/delivery_receiver.rs`.
- Dauerhafter Completionowner: `session/completion_delivery.rs`, `session/storage/jsonl/completion.rs`.
- Im aktuellen untracked Inventar existieren bereits `tests/test_completion_admission.rs`, `tests/test_admission_process_recovery.rs`, `tests/admission_support/`, `session/storage/jsonl/completion_tests.rs` und `session/acp_session_tests/completion_persistence.rs`. Ihre Existenz ist kein Lauf- oder Vollständigkeitsnachweis.

Kein Pager-/Sigma-Edit, kein Stage/Commit/Push, kein Build/Check/Test/Formatter, keine Credentials und keine Runtime-Sessions wurden angefasst.

## Offene Entscheidung

Historische Interrupted-Records ohne typisierten Grund erlauben keine sichere Unterscheidung zwischen absichtlichem Human stop/cancel und Runtime Error. Lead/Nutzer muss die Legacy-Migrationsentscheidung treffen; keine Klassifikation anhand Fehlermeldungstext. Das blockiert fachlich nicht den neuen typisierten Forward-Vertrag. Der konkrete hier wirksame Blocker ist stattdessen der verpflichtende Ausgangsstand-Gate des Workers.

## Nächste Schritte / unausgeführte Verifikation

Ein autorisierter Owner muss den vorhandenen unvollständigen Typmigrationsstand reconciliieren und dessen Ausgangsprüfung über den zulässigen CI-Weg herstellen beziehungsweise die widersprüchliche Worker-Gate-Vorgabe anpassen. Danach Forward-Protokoll vervollständigen, ohne `ParentTask`-/Origin-Fencing oder vorhandene korrekte Änderungen zurückzunehmen.

Noch nicht geprüft: Attempt-Reuse, tatsächliche persistierte Occurrence bei unbekannten/späten ACKs, sichere aktive Sampler-/Toolübergänge, Idle-/Goalwake, Held/Auto und Humanstop, unabhängige Children nach Caller-Turnende, Prozessneustart und Ackverlust, Receiverkonflikte und terminale Fehler, Receiptrevisionen und side-effect-free Observe. Die vorhandenen Regressionen und die Integration sind durch Lead via exakt korrelierter GitHub-CI zu prüfen; keine lokale Ausführung erfolgt.

## Observations

`wakeup-final.md` enthielt nach eigener Einleitung überholte Bestandsaussagen wie „kein ChatState-/Storage-Edit“ und „keine Receiver-API“. Das aktuelle Inventar widerlegt diese historischen Aussagen; die Datei erhält deshalb einen expliziten Current-truth-Hinweis. Daraus wird weder funktionierende native Receiverunterstützung noch eine Freigabe des fail-closed Consumers abgeleitet.
