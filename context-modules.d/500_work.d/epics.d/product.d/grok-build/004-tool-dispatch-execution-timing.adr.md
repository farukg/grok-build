# Actual tool dispatch timing is independent of call lifetime

## Context

Pending ACP calls and native ToolStarted precede permission and resource admission. ToolStarted also registers calls for existing turn accounting. The existing dispatch duration includes resource waits, authentication recovery and workflow post-processing; moving these anchors would change legitimate telemetry and call-budget behavior. The recoverable dirty-source baseline is `/tmp/grok-tool-execution-baseline.snajNc`, including its HEAD and source capability inventory.

## Decision

The generic typed runtime owns one execution guard shared by ToolDyn and ErasedTool. It starts after typed decoding, at the actual Tool::execute entry reached only after the shell's permission/admission and same-path resource guard. It finishes at the typed terminal, stream protocol end or actual dispatch-future/stream cancellation, before output conversion and post-processing. Each dispatch attempt has a typed execution ID and one monotonic anchor; authentication retries are separate executions, not duplicate starts of the same execution.

The narrow xai-tool-protocol::tool_execution module owns timing DTOs; native decoding does not gain the full runtime/protobuf dependency. Native Event::ToolExecution carries this additive timing. EventWriter's existing cancellation callback becomes one typed lifecycle callback, preserving cancellation delivery and additionally publishing timing-only ACP ToolCallUpdate metadata on the existing persistence/replay rail. Timing never independently asserts a successful tool outcome or a killed backend process. Legacy ToolStarted, ToolCompleted, telemetry duration, call-budget accounting and borrowed cancellation identity retain their owners and semantics. No second payload store, poller or startup query is introduced.

## Consequences

Missing timing decodes explicitly as Unknown; malformed supplied timing is a typed decode error. A finished dispatch carries its own execution ID, duration and termination, so it does not require receipt of its start or base call. Persisted replay retains that provenance. DispatchCancelled proves dispatch cancellation only, not termination of a detached process. Returned includes a returned typed error; only the existing typed outcome owner decides success, failure or cancellation, without a second classifier in timing.

Live elapsed projection is not supplied by wall timestamps or receipt clocks. ToolExecutionObservation receives the exact same Tokio Instant anchor at start and terminal. The active clock owner's RetryClockProjection::at(Instant) is private and sampler-specific; a shared public anchor-projection API/type is required before the runtime and Pager can consume it without a second calibration owner. Final duration is authoritative independently of that projection.

The measured source path is local typed dispatch. Proxy contexts deliberately do not serialize observers; backend-hosted model tools and direct bash-mode execution have no newly fabricated execution stamps. Runtime overhead is per observed dispatch: observer/context ownership, a stream wrapper and two lifecycle publications; there is no per-frame or startup work. Static labels remain serde enum labels, without label String copies.

## Status

Producer source and controlled-time regression sources are authored, not compiled or executed. Pager timing/orphan/replay state integration and shared-clock projection remain dependent work. This does not close timer finding E while consumer receipt clocks remain. Cargo dependency lock regeneration, formatting, CI and performance evidence remain unverified.
