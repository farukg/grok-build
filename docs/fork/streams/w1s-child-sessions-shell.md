# W1-S — shell: child sessions are first-class ACP sessions

Branch: `grb/w1s`. **Landed on `main` as `c084eea`** (plus `1765af0`, the pager-minimal build fix found by CI).

## Goal
A human can prompt a child session by its session id exactly like a root session, from any
client. The shell decides admission once, from the child's registry entry.
Design: `docs/fork/design/subagent-session-view.md` §4 "Shell types" / "Shell behavior".
Review of the old draft: `docs/fork/reviews/w1s.md`.

## State
Implemented (shell only):
- **Registry** (`S/agent/mvp_agent/session_registry.rs`): `SessionHost::Child(ChildHost { parent_session_id, reach })`,
  `ChildReach::{Addressed { address, residence }, Unaddressed}` (the coordinator mints an address
  for every child except workflow-owned ones), `ChildResidence::{Running(RunningChild), Finished}`,
  `RunningChild { handle, turns, parent_prompt_index }`. Children never enter the root presence
  map, so every root enumeration (broadcasts, roster, counts, idle sweep) is unchanged.
  `finish_child` keeps a finished child promptable until its parent is released; `release(parent)`
  drops its children recursively; a child that outlives its parent leaves nothing behind.
- **Registration** (`S/agent/subagent/spawn.rs`, `handle_request.rs`): `run_shell_child` hands the
  promoted child's handle and receipt sink back over a oneshot; `ShellChildRunner::run` registers
  it on the LocalSet and marks it finished after the worker returns (same identity on wake).
- **Prompt admission** (`S/agent/mvp_agent/child_prompt.rs`, `acp_agent.rs::prompt`):
  `ChildPromptAdmission::{Live { handle, turn }, Woken { parent_session_id, address }, Refused(ChildPromptRefusal)}`.
  Live reserves a receipt slot (`try_reserve_owned`), runs the unchanged root prompt path and
  settles the receipt with the turn result, so the parent still receives the child's last turn
  (telemetry operation: `Queue`, `Interject` for `sendNow`). A closed receipt stream or a
  finished child → `Woken` via `send_human_subagent_message` (coordinator same-id wake,
  T/task/coordinator/wake.rs). Workflow child → `WorkflowOwned`; 64 receipts in flight →
  `Saturated`. Root/unknown ids keep the old path and error.
  Woken response: `EndTurn` with `_meta.childWake = {"kind":"accepted","messageId":…}`; refusals
  are `InvalidRequest` with `data.childWake` (same shape as `x.ai/subagent/message`).
- **Cancel** by child id cancels a running child's turn.
- **Capability**: initialize `_meta.childSessions = "firstClass"`.
- Leader: no change needed. Requests auto-subscribe the requesting client; a wake re-emits
  `SubagentSpawned`, which re-registers the child route. The draft's `ChildRoute` map is gone.
- Tests (`S/agent/mvp_agent/tests/child_prompt_tests.rs`, through `MvpAgent::prompt`): running child
  settles the parent receipt with its turn; workflow child refused; finished child reaches the
  coordinator wake (typed rejection for an unknown address); wake needs the feature; finished
  child is released with its parent.

## Open questions (for Faruk)
1. Waking a finished child under the same id needs `features.active_agent_messages` (default off,
   xai-grok-config-types/src/registry.rs `ActiveAgentMessages`). Without it the prompt continues the
   child like `x.ai/subagent/resume` (M4): a new child starting from the finished one's history,
   response `_meta.childResume = {kind: resumed | queued | refused}`; the pager sees it as a new
   subagent. No error any more.
2. Decided (Faruk): ContinuedAs after a restart is the pager's job in W3 via
   `x.ai/subagent/resume` on "unknown session id".
3. Decided (Faruk): subagent behavior stays as it is; the `is_subagent` gates for hooks and the ↑
   prompt history are not switched to authority. ↑ history in child views is a low-priority
   follow-up (see `streams/w1h-hold-deliver.md`).
4. By-id handlers other than prompt/cancel (set_model, compact, mode, …) still resolve only root
   sessions.

Verified: `cargo test -p xai-grok-shell --lib` 7074 passed, the same 10 environment failures as
`main`; `cargo check -p xai-grok-pager-bin`; clippy on the touched crates clean.

## Next steps
1. Done: after a shell restart the child id is unknown. The pager stamps `_meta.childOf = <parent session id>`
   on prompts sent from a child view (`PromptTarget::ChildOf`, text prompts, send-now and skill blocks;
   not bash or execute-plan), and the shell continues the child through the M4 resume route
   (`continue_child_with_prompt`). Without `childOf` an unknown id is still `InvalidParams`.
2. Pager W3: consume `childSessions: "firstClass"` (prompt child sids, handle `childWake`,
   resume via `x.ai/subagent/resume` on "unknown session id").
