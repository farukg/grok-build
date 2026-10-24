# Independent Shell/Tools delivery review

## Result

No supported concrete defects found in the scoped Shell/Tools delivery and steer changes or the additional CI command changes. Confidence: moderate-high from source tracing and fixture inspection; runtime behavior and compilation remain unverified because no builds, checks, or tests were run.

## Reviewed exact paths

- `crates/codegen/xai-grok-shell/src/agent/mvp_agent/acp_agent.rs`
- `crates/codegen/xai-grok-shell/src/agent/mvp_agent/child_prompt.rs`
- `crates/codegen/xai-grok-shell/src/agent/mvp_agent/tests/child_prompt_tests.rs`
- `crates/codegen/xai-grok-shell/src/agent/subagent/child_runtime.rs`
- `crates/codegen/xai-grok-shell/src/agent/subagent/child_runtime_tests.rs`
- `crates/codegen/xai-grok-shell/src/agent/subagent/handle_request.rs`
- `crates/codegen/xai-grok-shell/src/agent/subagent/mod.rs`
- `crates/codegen/xai-grok-shell/src/agent/subagent/prompt_turn_receipt.rs`
- `crates/codegen/xai-grok-shell/src/agent/subagent/prompt_turn_receipt_tests.rs`
- `crates/codegen/xai-grok-shell/src/agent/subagent/tests/rest.rs`
- `crates/codegen/xai-grok-shell/src/agent/subagent/tests/wake.rs`
- `crates/codegen/xai-grok-shell/src/extensions/notification.rs`
- `crates/codegen/xai-grok-shell/src/extensions/subagent_deliver.rs`
- `crates/codegen/xai-grok-shell/src/session/acp_session_impl/parent_message.rs`
- `crates/codegen/xai-grok-shell/src/session/acp_session_impl/parent_message_tests.rs`
- `crates/codegen/xai-grok-shell/src/session/acp_session_impl/updates_tests.rs`
- `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator.rs`
- `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator/query.rs`
- `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator_state.rs`
- `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator_tests.rs`
- Additional CI scope: `.github/workflows/linux-test.yml`; PTY coverage: `crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/finished_child_manual_compact_cancel.rs`, `crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs`, and `crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e_shell_tools.rs`.

Pager production changes and codex-rs were excluded. The PTY report at `/tmp/grok-goal-4f7553a1dbe6/implementer/compaction-pty/result.md` was consulted for coverage mapping.

## Observations

- Traced default automatic delivery and explicit policy-setting through ACP prompt routing, coordinator active-message admission, Shell safe-point projection, receipt drain, and completion. Automatic prompts do not implicitly hold/release policy; safe-point Steers retain existing-turn sampling semantics.
- Projection commits persist one child `UserMessageChunk` per admitted message and forward the same notification only after commit. Each notification gets a separately generated event ID; prompt identity is taken from the running child turn. The test asserts ordered model input, child-session gateway visibility, per-message durable uniqueness, and no parent settlement before the next-sample barrier. The active-tool case uses the shipped `run_terminal_command` route and a loopback Bash `/dev/tcp` gate; it releases the command by socket write and asserts tool output precedes Steers. Bash availability and the test tool configuration are CI assumptions, not locally verified.
- Receipt drain closes admission only after buffered receipts settle, keeps Held completion pending, and restores automatic delivery without creating a turn or cancel command. Cancellation and terminal duplicate-settlement behavior have targeted unit assertions.
- Coordinator caller state is derived from live foreground reply/blocking query receivers. Abandonment wakes the loop; stale query receivers are removed, and observation updates are sent after coordinator events. Tests cover multiple waiters, last-waiter drop, and foreground handoff.
- Workflow filters map to the declared `xai-grok-tools`/`xai-grok-shell` library test targets and test names/modules. The PTY case is registered in `pty_e2e_shell_tools`; `xai-grok-pager-bin` owns binary `xai-grok-pager`, and the invocation's absolute `PAGER_BINARY` prevents fallback binary builds. Ignored-test selection uses the registered test-name substring.

## No-test caveat

This was a read-only review. No build, check, formatter, test, or CI run was performed; compiler compatibility, Bash `/dev/tcp` availability in CI, and actual test execution remain unverified.
