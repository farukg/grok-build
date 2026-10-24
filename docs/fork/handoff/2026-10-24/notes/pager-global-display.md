# Global Pager display integration

## Status

The requested Pager display integration is authored. No local build, test, check, formatter, staging, commit, push or service change was performed for this unit. Compilation, formatting and runtime evidence remain with Lead CI. Shell and Tools sources were not edited. Existing delivery and independent queue changes are retained.

Acceptance owner: criterion 5 in `/home/fg/.grok/sessions/%2Fhome%2Ffg%2Fprojects/01a15e70-cc42-70d2-8b7b-a20615d0abf4/goal/plan.md`. The earlier scratch-plan path was not canonical; no additional approval or split is pending.

## Causes and owner changes

### Global Tasks and Todo preferences

The prior Tasks shortcut manipulated per-view dock hiding or a three-state overlay. Todo visibility was also local. Tasks data synchronization could auto-open or auto-hide the pane, overriding explicit intent.

The existing Pager panel settings store remains the single durable owner. Tasks and Todo are stored under `[sidebar]` in `<grok-home>/panel-settings/global.toml`, using the existing disclosure codec, atomic read-modify-write, damaged-file handling and error diagnostics. Their storage scope is classified once by `SidebarSection::storage_layer`: session-layer edits target global storage, and invalid session-file overrides produce load warnings rather than shadowing the global preference. Only the preference actually changed by input is written; other writers' keys are preserved by the existing read-modify-write owner.

Ctrl+G and Ctrl+T now toggle their visibility preferences directly, independently of focus. AppView propagates the changed live preferences to existing views, retains them for newly attached views, and initializes from the persisted store after restart. Minimal-mode Todo uses the same preference. The existing runtime chrome cache is a mirror, not another persisted source. Failed writes retain the live intent and report the storage failure through the existing panel-save toast; they do not claim persistence succeeded.

Task synchronization no longer invents display changes when task data arrives or finishes. Task, Todo, workflow, subagent and queue data are not changed by visibility preferences.

### F7 and the additional upper display

F7 now contains the work projection regardless of Dock or Tasks/Todo visibility preferences. The same work-row owners supply workflows, subagents, background tasks, monitors and scheduled loops. F7 also includes the actual Todo checklist, with textual status for every item. Todo rows are part of the keyboard navigation shape, so End/PageDown can reach them; the read-only Todo list is not conflated with its global upper-display preference. Cursor repair handles a changing Todo count.

The Dock feature now controls only the additional upper projection. With Dock enabled, the global Tasks preference controls the additional upper work display, and the global Todo preference controls the upper checklist. F7 remains available when either preference or Dock is off. No alternate future dock content was introduced.

The two work projections derive their rows from the same owners but retain surface-local hit-test geometry and keyboard focus. Clicking a row in either surface resolves its actual painted height and the same stable work identity. Only the focused projection paints keyboard selection. Work/status cue clicks open or focus F7 without changing the global upper-display preference.

Manual child-delivery warnings remain derived by `SubagentInfo::delivery_warning`. F7 and the upper projection show the same authoritative warning, including `MANUAL - CALLER BLOCKED` semantics for a ready answer and a waiting caller. No independent delivery flag was added.

### Queued input

Queued input is no longer projected in work snapshots, work counts or F7 painting. Its only active projection is directly before the composer in AgentViewLayout, after optional notices and work chrome. Visibility derives from pending rows, not from a toggle. The shortcut changes focus; dismissing focus cannot hide pending input. The queue has no close control. There is no dock queue-collapse state.

When a short viewport contains queued input, layout prioritizes queued input and the composer over optional work/status chrome. Resize does not move the queue back into F7 or the work dock. Empty queues remain absent. Physical terminals too small to fit even one input row and one queue row still require runtime usability assessment; no impossible one-row-screen guarantee is claimed.

## Authored regression coverage

- Real AppView input across two sessions: both global toggles propagate immediately; switching sessions and toggling again preserves each session's Todo data.
- New-view and restarted-view restoration from the actual global panel store.
- Failed global preference write: live intent remains visible and the storage error is reported without replacing the obstructing path.
- Independent store instances edit different preference keys; reopening preserves both. Session-file attempts to override global-only preferences are diagnosed and ignored.
- Real render matrix: Dock on/off, F7 open/closed, queue empty/nonempty, all four Tasks/Todo preference combinations, and multiple viewport heights. Assertions cover queue adjacency, visibility, absence from dock counts, unchanged queue identities after focus/dismiss and absence of a hide control.
- Pure layout regression combines optional work, Todo, notices and recording indicators across viewport sizes and checks that queued input is immediately adjacent to the composer.
- F7 retains actual work rows, authoritative manual/caller-blocked warnings and real Todos under both Dock settings and both upper-display preferences. Keyboard End reaches the Todo row.
- Both painted work projections open the same child after resize. Clicks and keyboard activation use the selected surface's actual row geometry and identity.
- Task synchronization preserves explicit hidden and visible preferences through live task arrival and completion.
- Existing legacy dock/queue tests were migrated to the new projection contract. Background-task view/stop icon tests still exercise actual painted controls, now through F7 with the upper dock disabled as well as enabled. A task without a scrollback entry still opens its real viewer.

All cases are source authoring, not passing test results.

## Complete path list

All source paths below are relative to `/home/fg/projects/org/forks/grok-build/`:

- `crates/codegen/xai-grok-pager/src/actions/defaults.rs`
- `crates/codegen/xai-grok-pager/src/app/app_view.rs`
- `crates/codegen/xai-grok-pager/src/app/app_view_tests.rs`
- `crates/codegen/xai-grok-pager/src/app/panel_settings/types.rs`
- `crates/codegen/xai-grok-pager/src/app/panel_settings/codec.rs`
- `crates/codegen/xai-grok-pager/src/app/panel_settings/store.rs`
- `crates/codegen/xai-grok-pager/src/app/panel_settings/store_tests.rs`
- `crates/codegen/xai-grok-pager/src/app/agent_view/panel_persistence.rs`
- `crates/codegen/xai-grok-pager/src/app/agent_view/context_sidebar.rs`
- `crates/codegen/xai-grok-pager/src/app/agent_view/context_sidebar/commands.rs`
- `crates/codegen/xai-grok-pager/src/app/agent_view/context_sidebar/outline.rs`
- `crates/codegen/xai-grok-pager/src/app/agent_view/context_sidebar_tests.rs`
- `crates/codegen/xai-grok-pager/src/app/agent_view/input.rs`
- `crates/codegen/xai-grok-pager/src/app/agent_view/panes.rs`
- `crates/codegen/xai-grok-pager/src/app/agent_view/render.rs`
- `crates/codegen/xai-grok-pager/src/app/agent_view/queue.rs`
- `crates/codegen/xai-grok-pager/src/app/agent_view/sidebars.rs`
- `crates/codegen/xai-grok-pager/src/app/agent_view/mod.rs`
- `crates/codegen/xai-grok-pager/src/app/agent_view/session.rs`
- `crates/codegen/xai-grok-pager/src/app/agent_view/dock_input_tests.rs`
- `crates/codegen/xai-grok-pager/src/app/agent_view/task_icon_mouse_tests.rs`
- `crates/codegen/xai-grok-pager/src/app/agent_view/task_status_tests.rs`
- `crates/codegen/xai-grok-pager/src/app/agent_view/links.rs`
- `crates/codegen/xai-grok-pager/src/app/mouse.rs`
- `crates/codegen/xai-grok-pager/src/views/tasks_pane.rs`
- `crates/codegen/xai-grok-pager/src/views/agent.rs`

No config owner outside Pager was needed. No generated outputs or documentation files in the repository were created. The source action-help owner was reconciled with the new behavior; it no longer describes a hideable queue or mutually exclusive legacy task pane.

## Open execution evidence

Lead must run the pinned formatter, compile all affected consumers, and execute the named Pager persistence, AppView, F7, mouse, queue and layout regressions on the integrated source. Actual multi-session interaction, restart restoration, narrow-terminal behavior and simultaneous projections still require CI/PTY evidence. The existing panel-store owner documents that truly simultaneous writes from separate processes are not interprocess-locked; this unit preserves its existing atomic read-modify-write contract rather than claiming a new cross-process transaction guarantee. The authored two-store regression covers sequential stale-reader edits without overwriting unrelated preferences.
