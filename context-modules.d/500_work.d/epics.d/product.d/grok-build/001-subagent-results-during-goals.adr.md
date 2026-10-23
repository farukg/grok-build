# Subagent results remain admissible during goal execution

## Context

The coordinator distinguishes successful completion, runtime failure and intentional interruption. Shell admits eligible asynchronous results either as a new idle turn or at the active turn's next safe sampling/tool boundary. Explicit Held settlement belongs to the child receipt lifecycle.

A separate goal-loop gate suppressed these same results in four places: the runner's wake decision, prompt injection, Shell's completion reducer and the per-tool completion reminder. Some branches also marked discarded results reported. Thus a goal caller could permanently lose a child's success or actual error without any explicit human stop.

## Decision

Goal execution does not suppress subagent result admission. Delete the subagent-only copy of the goal gate from spawn context, completion data and injection arguments. Both Shell completion paths and the tool reminder preserve subagent results during a goal. Keep the existing interruption classifier, explicit kill/waiter decisions, AutoWake feature setting and child Held policy.

Ordinary background commands and workflows retain their existing goal-specific policy. Their goal gate is not a subagent delivery policy.

## Consequences

A goal caller can receive completed and errored child results without a new human prompt. An active caller still waits for an existing safe boundary; this decision neither interrupts a tool nor starts a parallel turn. Ending the caller's turn is not cancellation. Explicit human stop/cancel and Held settlement remain separate controls.

The existing task-ID-only reported resource and separately persisted transcript do not establish attempt-safe, crash-atomic admission. This decision does not claim to solve restart recovery, occurrence deduplication or general ACP prompt receipts. Those require an additional replacement transaction at the persistence owner, including terminal-result publication before attempt reuse and verification across process restart. No public durable Submit/Observe capability is introduced.

## Status

Accepted policy; authored locally, not compiled or executed. The goal-policy correction is partial progress toward completion admission/recovery, not completion of that workstream. Lead owns CI and runtime acceptance.
