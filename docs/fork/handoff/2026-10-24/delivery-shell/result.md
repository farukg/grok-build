# Shell delivery result

Author-only continuation delivered. No builds, checks, tests, formatter, staging or publication ran in this continuation. All new regression results and compiler compatibility remain unverified; Lead CI owns verification.

## Observation contract retained

SessionUpdate::SubagentProgress has additive defaulted `completion: SubagentCompletion` (working/settling/pending) and `caller: tools::task::coordinator::SubagentCaller` (notWaiting/waiting). Receipt drain derives completion; coordinator derives caller from foreground reply/blocking query receivers. Existing watch publisher exposes policy/receipt/caller changes. Pager owns consumption, attempt reset and warning display.

## Additional authored integration coverage

`agent/subagent/tests/wake.rs` now shares a real coordinator -> run_shell_child -> ShellChildRuntime -> SessionActor -> mock Responses sampler fixture between two regressions:

- Two live Steers admitted during a parked first inference response must appear exactly once and in admission order in the next request, before parent completion.
- Two live Steers admitted while a REAL run_terminal_command is blocked on a loopback socket must not trigger sampling before the tool is released. The subsequent sample must contain the completed tool result before both messages. The tool is released with a socket write, not cancellation, process abortion, sleep or a timing workaround.

A named mock next-sample terminal barrier prevents completing the whole child task before assertions. Assertions cover one child turn, expected tool-call cardinality, terminal snapshot, child gateway UserMessageChunks, unique event IDs, existing child prompt identity and durable child updates.jsonl exactly once. The fixture explicitly enables test permission bypass and foreground completion waiting. Bash `/dev/tcp` is a test dependency for the real tool gate; CI must validate availability and the installed child's toolset. No tool/sampler behavior was mocked inside ShellChildRuntime or SessionActor.

## Authoritative caller regressions

`tools/.../task/coordinator_tests.rs` adds a watch observation to the existing TestControl/TestRunner harness and authoring for:
- foreground caller waits, then drops: NotWaiting arrives without another command or deadline;
- background child has no waiting caller; two blocking queries keep Waiting after the first query times out; dropping the last receiver clears it without another command/deadline;
- foreground budget hands off the caller while the child remains Running, clearing Waiting.

Tests use event/watch observations and paused virtual time. The second query is explicitly polled to enqueue before the snapshot barrier; no scheduling-delay assumption is required. Existing coordinator closure wake handling is retained.

## Confirmed child-visibility defect and fix

The caller's Accepted result was not enough to show its message in the child. Production `parent_message.rs::drain_parent_messages_at_safe_point` persisted UserMessageChunk and projected sampler input but never forwarded that chunk to the child gateway. Persistence Update does not provide that forwarding. Batch messages also reused one eventId, risking client/history dedup.

The authored fix forwards each same persisted notification to child gateway after successful delivery commit and assigns distinct eventIds. It sets promptId to the currently running CHILD prompt, not receipt or caller identity. Ordinary live-user-local-echo suppression is intentionally not applied: the sender has no local block in the child. Existing safe-point and terminal lifecycle still own projection; no second prompt/parallel turn/forced abort is added.

Exact Pager-facing event, metadata, receipt, queue and replay contract: `/tmp/grok-goal-4f7553a1dbe6/implementer/delivery-shell/child-visibility-contract.md`. No new variant is needed. Missing active-Steer queue row is deliberate; missing child projection update was a real Shell defect. Pager's filtering/navigation facts alone do not prove a second causal defect.

## Active versus authored

All fixes here are authored WIP, not deployment evidence. Parent reports selected PATH CLI grok1.0.45 (22fd28fbacea) [alpha], with later Git HEAD b82768d4. That does not identify an already-running leader or prove it contains this uncommitted work.

## This continuation's changed source paths

Root `/home/fg/projects/org/forks/grok-build/`:
- crates/codegen/xai-grok-shell/src/agent/subagent/tests/wake.rs
- crates/codegen/xai-grok-shell/src/session/acp_session_impl/parent_message.rs
- crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator_tests.rs

Existing prior Shell changes retained:
- agent/mvp_agent/{acp_agent.rs,child_prompt.rs,tests/child_prompt_tests.rs}
- agent/subagent/{child_runtime.rs,child_runtime_tests.rs,handle_request.rs,mod.rs,prompt_turn_receipt.rs,prompt_turn_receipt_tests.rs,tests/rest.rs}
- extensions/{notification.rs,subagent_deliver.rs}
- session/acp_session_impl/{updates_tests.rs,parent_message_tests.rs}
- Tools task/{coordinator.rs,coordinator/query.rs,coordinator_state.rs}

No Pager or codex-rs source is changed by this unit. No WIP was discarded.

## Verification remaining

Lead must compile, format, lint and execute the new named regressions plus existing receipt, parent-message persistence/terminal-race, child prompt, caller abandonment and Pager suites. Runtime child switching/replay remains Pager owner's integration responsibility. These author-only tests are not green evidence. Source verification of the real tool gate, gateway serialization field assumptions, durable flush before final child return and all exact-once assertions awaits CI. No new deployed behavior is claimed.
