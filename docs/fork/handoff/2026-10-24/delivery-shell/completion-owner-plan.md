# Completion admission owner — work in progress

Baseline HEAD: c0e3db3ff50410c7d1b4abc8b17beb346b5b2884. Existing dirty Shell/Tools/Pager changes are retained. No branch operation, build, check, test, formatting or publication is authorized here.

## Responsibility/invariants before edits

- Tools coordinator owns attempt identity, result classification, waiter delivery and completion buffering. A child turn ending is not parent cancellation. Held settlement remains the existing receipt owner; no ingress operation changes policy.
- Shell SessionActor owns parent queue admission and safe sampling/tool boundaries. Goal mode must not discard subagent results; ordinary background-task goal suppression remains separate.
- ChatState and session storage own transcript projection and durable acknowledgement. The task-id-only resource ledger cannot be authoritative for subagent attempts. Completion projection must carry occurrence identity in the same persisted transcript record.
- Restart recovery must use existing terminal result/history owners, not a new JSON registry. A resource hydration test is not restart proof.

## Capability inventory from source

Current wake: coordinator::finish_child -> ShellChildRunner::on_completed -> present_child_completion -> XaiSessionNotification + synthetic Prompt -> turn::handle_prompt/build_wake_turn_message -> ChatState. Active turn: interjection safe reducer -> drain_subagent_wakes_at_safe_point. Per-tool path: TaskCompletionReminder separately drains and marks task-id ledger. Startup: session_setup::heal_orphaned_subagents only repairs unfinished/running children and never re-admits completed results. Metadata status is committed before the parent notification is persisted. This is an additional crash boundary, not just a reported-resource race.

Retained capabilities and target verification: success/error wake (idle, active and tool-boundary) -> Shell owner plus real sampler integrations; attempt reuse/retries -> coordinator identity plus strict transcript append; crash recovery -> storage durable append/reload and real session restart; held/stop/cancel -> existing receipt/coordinator regressions, unchanged policy.

## Adjacent receiver API contract

No versioned durable Submit/Observe general-purpose human-prompt receipt API exists in the examined owner. Ordinary ACP session/prompt remains terminal RPC; queue observations are transient. Completion-specific occurrence admission must not be advertised as SIG human-prompt ReceiverExact. No new public SIG API or capability is promised by this unit. SIG owner should keep the proposed contract explicit/unimplemented rather than treat completion WIP as a receiver guarantee.

## Concurrent CI issue

Reported failure actor::tests::apply_turn_request_pruning_is_noop_under_half_window concerns context pruning/token accounting, not completion admission. Ownership evidence: xai-chat-state/src/actor/tests.rs and actor/{request_builder.rs,mutations.rs,state.rs}; xai-grok-sampling-types context projection is concurrently owned. It needs a separately assigned context-pruning worker; this unit will not weaken the assertion or make an unrelated change.
