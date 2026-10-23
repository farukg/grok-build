# W1-H — explicit subagent completion delivery

## Current policy

Running attempts default to automatic completion delivery. Opening, leaving, observing or
prompting a child view does not change its delivery policy. Only the explicit header button
or F5 switches automatic/manual delivery. The policy lasts for the running attempt; it is
not a persisted global preference. Finished children cannot change delivery mode.

Manual delivery holds the final answer for the caller. Returning to automatic delivery
releases an already pending answer through the existing receipt settlement. This control
is completion delivery, not human takeover or an instruction to cancel active tools.

## Owners

- Shell `extensions/notification.rs`: `SubagentDelivery::{OnTurnEnd, Held}`.
- Shell `agent/mvp_agent/child_prompt.rs`: prompt admission preserves delivery; explicit
  delivery requests update the running child's watch channel, refusing terminal children.
- Shell `extensions/subagent_deliver.rs`: `x.ai/subagent/deliver` accepts `sessionId` and
  additive `delivery` (`onTurnEnd` or `held`). Omission retains the older release operation.
  Outcomes are `delivered`, `not_held`, `held`, `not_running`.
- Shell `agent/subagent/prompt_turn_receipt.rs`: held completed settlement remains open for
  steering receipts until automatic delivery or cancellation. The latest receipt settles
  the caller's answer.
- Shell progress publisher observes the same delivery channel; progress carries the mode.
- Pager `app/dispatch/turn.rs`: explicit requested target, no optimistic mode mutation.
  Failed RPCs leave the reported mode unchanged and can be retried.
- Pager `app/agent_view/render.rs`: clickable `[delivery:auto F5:manual]` or
  `[delivery:manual F5:auto]` for running children; no toggle on finished children.
- Pager `app/agent_view/panes.rs::dock_subagent_rows` and `views/dock/mod.rs::paint_row`:
  caller dock displays a leading bold warning-colour manual-delivery notice with the
  open-child/F5 recovery action. It derives from the same attempt mode as the child header.
  Manual mode is also marked in shared subagent labels.

## Verification and limits

Author-only changes await Lead CI. Regression tests cover explicit target dispatch, failed
RPC retry, child header click targets, navigation preserving both modes, terminal refusal,
and dock warning reset. Shell receipt-settlement tests remain the owner of caller wake-up.

Progress carries independent, defaulted observations: `completion` is `working`, `settling`
(initial turn ended, admitted receipts pending), or `pending` (all receipts settled);
`caller` is `notWaiting` or `waiting`, derived from coordinator foreground replies and
blocking query waiters. Pager preserves them across duplicate spawn reconciliation, resets
them on terminal/new attempts, and rejects late terminal progress through the lifecycle gate.
The dock always marks running manual children. With `pending` + `waiting` it leads with
`MANUAL — CALLER BLOCKED: answer ready; open child, F5: auto`; working/settling children
instead show their actual phase and whether the caller waits. No activity text inference or
second delivery flag is used. Handler, lifecycle, navigation and paint regressions are authored;
compiler/runtime verification remains pending CI.

Live steer/queue admission is separate from final completion delivery. Shell routes eligible
automatic child text prompts into the existing coordinator Steer safe-point path; manual
prompts retain the resident Queue path. Full live-tools integration remains unverified in the
Shell unit; completion hold was not the ingress delay's root cause.

Caller-completion continuation (author-only): subagent results no longer inherit ordinary
background-task goal suppression. Runner injection, idle digest, active safe-point projection
and per-tool reminders now retain success/error information during goal execution. The
subagent-only goal gate was removed; bash/workflow goal policy, explicit stop/cancel, AutoWake
configuration and Held settlement were not changed. The actor regression covers successful
and failed active completions with and without a goal; the idle goal digest contains the real
failure text and a successful sibling. These are authored actor/reminder regressions, not a
real child-to-parent sampler or restart acceptance result. Attempt-keyed durable admission,
terminal-result recovery and process-crash tests remain open. Decision:
`context-modules.d/500_work.d/epics.d/product.d/grok-build/001-subagent-results-during-goals.adr.md`.
