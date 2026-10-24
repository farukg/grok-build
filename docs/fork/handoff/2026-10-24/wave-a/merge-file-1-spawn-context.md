# Wave A per-file merge report — file 1: subagent_spawn_context_tests.rs

Status: done (resolved by the Lead after two worker tool-mechanics failures).

## Applied resolution (region 161-314, diff3)

- Runtime lane kept: `subagent_spawn_context_shares_parent_goal_loop_gate` — the
  real-fixture late-goal test (`completion_wake_receiver_fixture`, goal states
  None/Active/UserPaused, late ordinary task completion, child-completion admission
  with `CompletionProjectionAck::Appended`, durable receipt via
  `PersistenceMsg::GetCompletionState`, exactly-once conversation projection).
- Canonical lane kept: `subagent_spawn_context_keeps_completion_policy_when_parent_goal_changes`
  — parent goal gate flip must not change `auto_wake_enabled`.
- The base-section Arc test (`ctx.goal_loop_active.load`) was NOT re-added: the
  `goal_loop_active` field on `SubagentSpawnContext` exists in neither checkpoint side
  (verified: only `auto_wake_enabled` remains, subagent/mod.rs:378); both lanes deleted
  that mechanism. Re-adding it would resurrect a dead API. The invariant is superseded
  by the two live tests (behavior via real fixture, policy stability via canonical).

## Verification

- `grep` marker count in the file: 0.
- Both tests' APIs verified against the merged tree:
  `build_subagent_spawn_context` (mvp_agent/subagent_spawn.rs:171),
  `make_test_handle` (mvp_agent/tests.rs:1210),
  `tool_context.goal_loop_active_gate` (acp_session_impl/goal_support.rs),
  `SubagentSpawnContext.auto_wake_enabled` (agent/subagent/mod.rs:378).
- No build was run (author-only; consolidated CI run covers it).
