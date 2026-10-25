# Attempt-bound input admission and receiver recovery

## Context

Coordinator terminal results, caller model inputs and session receipts previously had different acknowledgement boundaries. Task-ID-only reporting could suppress a later attempt, and enqueue success could survive without a durable model input. External human deliveries also require a durable admission receipt without waiting for the standard prompt RPC's terminal response.

The replacement reuses the coordinator's AttemptId, terminal publication callbacks, ChatState strict append acknowledgement, the session persistence FIFO, chat JSONL and summary mutations. It does not introduce another result store. The recoverable continuation baseline is `c256ab292b86ad6fb42d76d200c953257d34c862` plus the preserved dirty source/index archive in the contracted continuation evidence directory.

## Decision

A child result is identified by `(subagent_id, attempt_id)` through preparation, publication, completion buffering, tool observations and caller admission. The producer commits its classified outcome before foreground consumers or publication can expose it. Passive completion buffering becomes visible only after history publication; its existing terminal-publication state retains the pending buffer entry until then. A receipt acknowledgement or reader teardown before the visibility callback retires that pending entry rather than resurrecting it later. Attempt reuse cannot turn an old publication callback into the current attempt's outcome. The destructive task-ID-only completion drain is deleted; observation and explicit attempt-bound projection acknowledgement are the sole buffer API.

ChatState and session storage commit the model input and its occurrence receipt in one chat JSONL record. Summary receipts retain that fact across history replacement. A duplicate reconciles authoritative history and the original stored occurrence, never the retry's proposed prompt ID. A projected but unfinished obligation may continue under a new parent-task instance without appending another input. The continuation rebinds all outstanding child receipts from the same prior parent-task turn, including a multi-result tool response or idle digest; it must not resume those siblings as separate model inputs. The first projection receipts remain unchanged. Successful exact-parent terminals settle obligations. Failed or interrupted caller execution remains recoverable, subject to the independently persisted human-stop policy.

Idle digests and active buffered results retain occurrence-bound durable result references. Published payloads persist the typed reference and raw text, deriving their combined model text on read. Missing references in older payloads retain their original text without inferring a new cause or identity. The read-only completion-state observation merges the actual chat receipts with the existing summary projection state; it is not another ledger. Receipt observation runs only for relevant completion admission, not per frame, by polling or on ordinary session startup.

Input admission carries the parent-task identity and cancellation token to the storage append lock. A closed task cannot acquire a new projection. Receiver execution leases and terminal settlement are fenced by both turn ID and parent-task instance. Cancellation captures the binding before tearing down the task. An unprojected delivery can be settled only by its exact delivery origin, digest and queued turn ID.

Snapshot-reference updates and teardown do not establish an external terminal child state. They retain the started artifacts for the same completion data; the actual producer/publish owner materializes terminal status, cause and timestamp from the committed outcome. Regression fixtures start with Running metadata, retain it unchanged through preparation, then verify the original Completed/Cancelled/Failed, snapshot and worktree oracles after actual publication.

`x.ai/session/delivery` implements the separate external human-delivery contract: exact UTF-8 payload digest, stable session-bound delivery identity, durable queued obligation before `admitted`, monotone receipts, conflict without scheduling, side-effect-free `observe`, and `delivery_changed` hints. Projected receiver execution is never automatically replayed after process loss; unresolved execution becomes `interrupted` and may later resolve terminally. Receiver identity tombstones survive explicit history deletion. Standard `session/prompt` retains terminal RPC semantics.

Goal execution does not suppress eligible child success or runtime-error information. Active delivery waits for the existing sampling/tool safe points. Ordinary caller turn completion does not cancel independent children. Explicit human stop/cancel, child Held/Auto settlement and the AutoWake setting remain separate policies.

## Consequences

The same input is not appended again after an unknown or lost acknowledgement, a late callback, child-ID reuse or process reconstruction. A continuation samples retained authoritative history rather than regenerating the child output. This does not promise exactly-once external tool side effects after a model-visible receiver interruption.

Historical interrupted child records lacking a typed cause remain unresolved. Neither error-message text nor the legacy `cancelled` display label establishes human intent. Any decision to migrate those records requires the Lead/user; the forward typed contract does not depend on that decision.

Retained capability verification owners are coordinator lifecycle tests; strict JSONL projection/recovery tests; native ACP child success/error × idle/active/sampling/tool/goal tests; foreground tool-result tests; and subprocess admission, projection and terminal crash-boundary tests. These tests are authored, not executed in this continuation. Recovery scans only relevant persisted obligations; no timer, polling loop or per-frame work is added.

## Status

Authored locally, with pinned formatting on the continuation's edited Rust files. No Rust build/check/test, staging, commit, push or release acceptance is claimed. Lead owns compilation, CI and runtime acceptance. Legacy ambiguous-interruption migration remains an explicit open decision.
