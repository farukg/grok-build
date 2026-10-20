# W3 — pager consumes first-class child sessions

Branch: `grb/w3`. Shell side is W1-S/W1-H (on `main`).

## Done
- `ShellChildSupport::{FirstClass, Legacy}` parsed once from `initialize` `_meta.childSessions`
  (`P/acp/mod.rs`), kept as `AppView::shell_child_support`. A prompt from a subagent view on a
  legacy shell is refused with a toast (`P/app/dispatch/prompt.rs`), the composer keeps its text.
- Prompts from a child view carry `_meta.childOf` (`PromptTarget::ChildOf`) so a restarted shell
  continues the child through the resume route (`S/agent/mvp_agent/child_prompt.rs`).
- `_meta.childResume = resumed{sourceId}` on a prompt response marks `AppView::follow_resumed_child`;
  the spawn that names that source (`resumed_from`) opens in place of the finished view
  (`P/app/acp_handler/session_notification.rs`, `apply_child_view_follow_up`). A continuation nobody
  asked for stays in the background.
- By-id shell handlers (set_model, compact, mode, …) reach a running child (`8ea9759`).

## Open
1. Finished children: set_model/compact/mode answer `unknown session id` (accepted by Faruk). A fix
   would remember the choice in the registry entry and pass it as a resume override.
2. ~~Fork from a child view.~~ Verified by code: children persist like root sessions
   (`sessions/<cwd>/<child id>`, `S/agent/subagent/handle_request.rs:931`), `x.ai/session/fork` copies from disk
   by id and cwd (`S/session/fork.rs`, `storage/jsonl/copy.rs:270`), and the pager forks with the child view's
   id and effective cwd. Pinned by `fork_from_a_child_view_forks_the_child_session_in_its_cwd`. Not run live.
3. ~~End-to-end tests.~~ Pager side: `prompting_a_finished_child_ends_in_its_continuation_view` (prompt with
   `childOf` -> `childResume` answer -> spawn with `resumed_from` -> continuation view open). Shell side
   (`parent_sees_final_response_after_human_turns`) is covered by `child_prompt_tests.rs` through `MvpAgent::prompt`.
   A woken child (`_meta.childWake`) needs no pager handler: the wake re-emits `SubagentSpawned`.
4. ~~`!cmd`, execute-plan and bash prompts from a child view do not carry `childOf`.~~ Done: `Effect::SendBashCommand`, `ExecutePlan` and `SetModeThenPrompt` carry the view's `PromptTarget`.
