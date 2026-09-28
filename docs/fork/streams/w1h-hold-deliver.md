# W1-H — hold a steered subagent until `deliver`

Branch: `grb/w1h`. **Landed on `main` as `71a6fae` (shell) and `6fa71ae` (pager).**

## Goal (Faruk, 2026-09-28)
Subagents keep their behavior; only the view looks like a main session view. A human can prompt
a running subagent to steer it before its final answer. From the first human prompt on, the
answer is held back from the caller; the human steers as long as needed and then presses
`deliver`, which hands the latest answer to the caller (the agent that started the subagent).
Labels stay short (`deliver`, not a sentence).

## State
Shell:
- `SubagentDelivery::{OnTurnEnd, Held}` (S/extensions/notification.rs) is the one state; a
  `watch` channel per running child (S/agent/subagent/handle_request.rs) feeds the receipt drain,
  the progress publisher and the registry entry (`RunningChild.delivery`).
- A human prompt admitted to a running child sets `Held` after its receipt slot is reserved
  (S/agent/mvp_agent/child_prompt.rs).
- The receipt drain (S/agent/subagent/prompt_turn_receipt.rs) keeps its stream open while a
  completed run is held, so later human prompts still land in the same run; only `deliver` or a
  cancel lets the settlement finish. The last settled turn is the caller's answer.
- `x.ai/subagent/deliver { sessionId: <child> }` → `{kind: delivered | not_held | not_running}`
  (S/extensions/subagent_deliver.rs).
- `SubagentProgress.delivery` (serde default `onTurnEnd`, old sessions load unchanged); the
  publisher emits immediately on a change and in its 8 s heartbeat, so a reconnecting client sees
  a held child again. No new polling.

Pager:
- `SubagentAttemptInfo.delivery` from progress; `SubagentInfo::awaits_delivery()` (running and
  held); `SessionViews::child_info(id)`.
- Child view header shows `[deliver]` (clickable) while held; `F5` (action `deliver`,
  cheatsheet category Session) does the same. Toast only when nothing was released.

Tests: drain holds after the turn and releases on deliver (prompt_turn_receipt_tests.rs); shell
prompt → held → `x.ai/subagent/deliver` → released, finished child → `not_running`
(child_prompt_tests.rs); pager header offers `[deliver]` as a click target (header_tests.rs);
F5 releases only a held child (acp_handler/tests/subagent_delivery.rs).

## Open questions
1. Key: `F5` (free everywhere). `F9` stays unbound on purpose (an upstream test pins it after the
   old mouse toggle); `Ctrl+Y` is yank in the prompt. Change if you prefer another.
2. While held, the caller sees the subagent as still running. A foreground caller simply keeps
   waiting (possibly no need to pause the main agent any more); a background caller continues
   and gets the answer on `deliver`.

Verified: pager lib 10119 passed (only the four known doctor failures); shell lib 7076 passed
(the same 10 environment failures as `main`); clippy clean on the touched lines;
`cargo check -p xai-grok-pager-bin` builds.

## Next steps
1. Low priority: ↑ prompt history in the subagent view (today `prompt_history` skips subagent
   sessions, S/session/acp_session_impl/prompt_queue.rs ~L166).
