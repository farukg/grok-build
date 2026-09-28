# M11 — agent and human control over subagents

Branch: `grb/m11` (based on `main` `b540b07e`; merge `main` before continuing).

## Goal
The calling agent and the human can see and control subagents: queue or steer messages,
stop / pause / resume, and always see a typed terminal state with the interruption cause and who
caused it (agent or human). Plan and file:line evidence: `docs/fork/recon/m11-subagent-control.md`
(B1 typed terminal state, B2 messaging, B3 stop/pause/resume, B4 visibility).

## State
- On `main`: `b540b07e` typed interruption state types in `crates/common/xai-tool-types/src/subagent_state.rs`.
- On the branch: `5f38e66d` wiring of the typed state through coordinator, result, notification and
  `meta.json`; `780a739d` is an interrupted snapshot (adds an untracked-then `control_subagent.rs`
  tool file and edits in task types, scheduler actor, task_output, notification drain). **Neither
  has been compiled.** Treat `780a739d` as a draft: keep what fits the plan, finish or remove the rest.
- `archive/m11-full-wip` holds an older, larger experiment; use it only as reference.

## Next steps
1. Merge `main`, make `cargo check -p xai-grok-tools -p xai-grok-shell --tests` green.
2. Finish B1 wiring: every construction site, old persisted `meta.json` without the new field parses
   to a named legacy variant at the one parse site; wire `status` strings stay as a projection.
3. B3: one gated tool `control_subagent { subagent_id, action: pause|stop }` calling the existing
   backend cancel with the cause; `kill_task` delegates to the same call; pause sets wake
   eligibility; resume of a paused child uses the existing wake path with a typed notice.
4. B4: `list_subagents` model tool on one coordinator query, reusing `SubagentSnapshotDto` as the
   single projection; `get_task_output` / completion words render the cause.
5. B2 human `delivery` field on `SendSubagentMessageRequest` through the existing `resolve_delivery`.
6. Pager: parsed terminal state in task rows and the child header (after W2 lands, the child view is
   the normal session view); `Action::PauseSubagent` next to the existing stop action.
7. Tests per B1–B4 behavior, then PR into `main` (can land in slices: B1+B3 first).
