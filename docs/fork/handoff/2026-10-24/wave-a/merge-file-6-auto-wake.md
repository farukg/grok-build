Status: done

Divergence inventory: the merged test module retains both `queue_lifecycle` and `fixtures` module declarations. Runtime typed `QueueInputRequest` and `TurnInputRequest` paths coexist with canonical `TaskWakeAdmission` fixtures and receipt/projection assertions. The mismatched wake prompt IDs previously embedded `at1.1` while their `CompletionWake` values minted an attempt ID.

Decisions and changes: added `completion_wake_prompt_id`, deriving the wake prompt ID from the single `CompletionWake` value. `wake_request`, `subagent_completed_input`, and `run_wake_turn` now share that bound identity between prompt ID, origin, and running-task stub. The promotion test now captures and asserts the actual request prompt ID instead of a hard-coded attempt. Both lane-specific test coverage and all assertions remain. Production `completion_prompt_id` uses the same `{subagent_id}/{attempt_id}` identity (with an additional unique suffix); admission binds current input through its prompt ID and completion projection retains `completion.id`.

Open questions: none identified in this file. Build/test verification was not performed, per the author-only constraint.