# W1-S — shell: child sessions are first-class ACP sessions

Branch: `grb/w1s` (based on `main` `3b54b425`).

## Goal
A human can prompt a child session by its session id exactly like a root session, from any
client. The shell decides admission once, by the session's host:
- resident child → prompt runs through the same `SessionCommand::Prompt` path, and a turn receipt
  is registered with the coordinator so the parent still receives the child's final response;
- finished known child → wake with the same identity (`T/task/coordinator/wake.rs` ~L54-146);
- child unknown after a restart → `ContinuedAs` via M4's `human_resume_request` (`T/task/resume.rs` ~L225-249);
- workflow child → typed refusal.
The shell advertises `childSessions: "firstClass"` in the initialize response `_meta`.
Design: `docs/fork/design/subagent-session-view.md` §4 "Shell types" / "Shell behavior".

## State
- Six commits on the branch (`git log main..grb/w1s`), head `919d7c9e`; all partial, not compiled
  together.
- A binding review of the latest state is in `docs/fork/reviews/w1s.md` (8 points). Summary:
  carry the host inside the admission instead of re-matching it; drop `PromptAdmission::Unknown`;
  replace the `closed: Arc<AtomicBool>` flag by a typed finished-child state in the registry;
  actually execute `Woken` and `ContinuedAs`; the leader's `child_routes` map is written but never
  read (make it decide routing or remove it) and pruning must happen when the parent closes;
  correct the receipt telemetry operation; rustfmt.

## Next steps
1. Address every point of `docs/fork/reviews/w1s.md`.
2. `cargo check -p xai-grok-shell --tests`, then `cargo test -p xai-grok-shell` green.
3. Behavior tests only for the admission outcomes: resident child prompt reaches the child and the
   parent receives the child's last response; finished child wakes with the same id; workflow
   child is refused.
4. PR into `main`. The pager side (W3 in the W2 stream file) consumes the capability.
