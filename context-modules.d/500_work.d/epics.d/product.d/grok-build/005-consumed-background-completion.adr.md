# Receipt-bound live background completion admission

## Context

Ordinary terminal completion has two producer stages: acknowledged durable enrollment, then unacknowledged UI/model publication. Terminal waiters can consume the result between those stages. The tool reminder's task-ID reporting cache and the shell's instantaneous queued-item sweep did not fence later publication. The notification bridge independently manufactured a new prompt from the later snapshot, with a separate admission wait for every notification. Consequently an already-consumed result could reopen a completed caller, and a ready burst could become several turns or serialized admission timeouts.

The baseline is `495bebf4e72d773160020e2ebc5e33ef326323d1` with the preserved dirty scoped sources in `/tmp/grok-consumed-background-baseline`. The existing strict tool-result/completion JSONL receipt is the consumption authority. `ReportedTaskCompletions` is only the existing reminder cache, not proof of durable consumption. No additional result ledger is needed.

## Decision

- Preserve the terminal actor's enrollment-before-exposure protocol and waiter cancellation semantics. Publication is not consumption.
- Carry terminal observations from inlined completion reminders through the existing invocation metadata and strict tool-result commit. Resolve missing observations for typed consumed results through the bound terminal backend, without rereading results already observed by the tool.
- Retire queued completion members only after the existing durable receipt acknowledgement. Retain unconsumed members of a mixed batch and never remove the currently executing input.
- Gather terminal publications from the notification receiver's finite ready cohort. Use the existing refcounted reservations while routing that cohort so terminal monitor events cannot race into a separate model turn. Forward their UI observations normally.
- Submit the cohort as one actor-owned completion batch. Read authoritative receipt state once before live admission. Already-receipted or durably suppressed identities are UI-only, even if a stale declared wait still names them. Deduplicate queued identities and invoke the existing goal/human-stop admission owner for genuinely outstanding candidates.
- Queue retained candidates using the existing typed notification-batch projection. All admission responses share one existing 250 ms bridge budget; timeout preserves the actor's fallback, not a discarded result. At most one synthetic trace belongs to the batch's actual queued prompt.
- Live notification rows never reinterpret an existing receipt as recovery. A receipt racing admission is excluded again at strict projection. Explicit persisted recovery retains the existing terminal-only continuation path and its original occurrence binding.
- Inputs carrying durable completion identities have drain custody. Human stop parks the batch behind the existing barrier rather than deleting outstanding siblings. No final-answer flag, notification TTL, polling loop, or new persistence/wire format is introduced.

## Consequences

Late consumed completions still persist and emit `x.ai/task_completed`, dispatch the completion hook, and update the task snapshot. Their `will_wake` is false and they do not request a synthetic trace or acquire a new model input. A selected outstanding result still breaks its declared wait through the existing admission policy, including the goal exception. Jointly available results share one queued input; duplicates and consumption races do not create another input. Failed/interrupted receipt-bound execution remains recoverable through the explicit existing recovery owner.

The additional receipt read is per completion batch, not per frame or startup. Missing tool observations require a terminal lookup only for consumed IDs lacking existing invocation metadata. Moving finalized observation metadata avoids another full snapshot clone. Performance and end-to-end model scheduling remain unmeasured.

## Status

Source authored locally. New regressions cover consumed publication before/after final completion, stale waits, jointly available results, duplicate publication, partial batch consumption, receipt/admission races, selected goal waits, human-stop parking, monitor-event isolation, one trace, and one admission budget. All tests, compilation, formatting, CI, and runtime acceptance are **UNRUN**. No publication or deployment is claimed.
