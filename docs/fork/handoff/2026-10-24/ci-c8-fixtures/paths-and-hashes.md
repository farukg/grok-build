# Paths and SHA256

Relative paths use the isolated worktree. Absolute paths, HEAD/continuation/review baseline hashes and protected source hashes are in source-manifest.json.

| Source | Authored SHA256 |
|---|---|
| `crates/codegen/xai-chat-state/src/actor/tests.rs` | `71487ca07f98949304b6cd337349cc8f7a33b0eb104b9d4120703762b10bb3e4` |
| `crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs` | `f22efce217074a8d17e1e07bb751362560884cdeaf4673f8f411f263c8143921` |
| `crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_selection.rs` | `f61e9566504c751440f3dc18ac7555fb6bf1cf247ff43404521454380793319d` |
| `crates/codegen/xai-grok-pager/src/app/acp_handler/tests/completion_replay.rs` | `b70ef9b494f5d34c3134a12bbe0a3c8bd7f5c49cb4cf1be03b668afd85dab777` |
| `crates/codegen/xai-grok-pager/src/app/acp_handler/tests/mod.rs` | `7b9dbc28dca34c3ded08d290dfe0603191e4fab582b57fcebee7be348f00aec9` |
| `crates/codegen/xai-grok-shell/src/agent/mvp_agent/tests/subagent_spawn_context_tests.rs` | `9e65c937a93509a4ce1945f4feff8ea9fecff23d876c69d95c742d4537ea42e1` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/completion_test_support.rs` | `7c6b3b8007f62e9712feccc89b1a922a021d65bea498ea641e73c52d0f309ff3` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/isolated_spawn_e2e.rs` | `6a7dccfc370d258d4ffcfe9dc88ae96cd874ff0a458997550b0b9b521ad314b6` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/mod.rs` | `95601aa68ebef9030ed7f228ec95d53b57e518426f50d3a3b9037b9e7e953687` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/tests/completion_fixture.rs` | `a688233b34bd78b5b762f83be14039c8508637fb0b027493bd6dc4d8d1a85e75` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/tests/coordinator_fixture.rs` | `b969d5d2a40487d5e6fd11975752438b47d973df992faab79093a9686408599b` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/tests/mod.rs` | `5a9c98c2bce204f388d5795217cb2431ac656ec0c06ea4e5a2644cd19c3f78b0` |
| `crates/codegen/xai-grok-shell/src/agent/subagent/tests/rest.rs` | `4322470710c7bc820a505541ff3c880f3bc499cfd4e5436ce42fc7eafba7fc90` |
| `crates/codegen/xai-grok-shell/src/agent/testkit/mod.rs` | `23619502f0c07cda7eed01d8efc03ac79b2298e0c394df0fffa693ebc0ba9c0f` |
| `crates/codegen/xai-grok-shell/src/session/acp_session.rs` | `1a8078722c230629658342365205b9420a410aee35d65a8edb2365546d539764` |
| `crates/codegen/xai-grok-shell/src/session/acp_session_tests/auto_wake_suppression_tests.rs` | `87be37ae0829d823401c09fc9d4496848416604f988f1f1089548922009b8507` |
| `crates/codegen/xai-grok-shell/src/session/acp_session_tests/completion_persistence.rs` | `350143d1eee4627a0f6c9ee9085150097b325f6447d578817e845b4924155e0c` |
| `crates/codegen/xai-grok-shell/src/session/acp_session_tests/support.rs` | `61a583a0accce3bdde84aef4caf62accf67c4c0048e62bbcf448356f7ea99fc8` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/backend_tests.rs` | `e44b19f78f0a6f6fb8404653a6c609f23e2ecc80ee6afae98867d67c3a658500` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator/active_message_tests.rs` | `f71f985adb155c9b6cd506974cbcfaf081f1e9f68da2066b2862afae602af04f` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator/agent_targets_tests.rs` | `be2464f541ca5d91eeaade8b01346c189e323fe2b7c57fde8dadad8224c9107c` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator/root_targets_tests.rs` | `60b70ff1dc9866522ff945e2d7c0403d947d90239bd54755d78cbf2bf50e97b2` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator_tests.rs` | `d702f6479012f4dbe61ff356c4298463b11ad90aba7469485a8717566388f8e4` |

## Artifact SHA256

- `fixtures.patch`: `f5ec2b195dadf4f6fccd4018950bcee1a19fc96ff0cd60b0f966f4cd30a87fcf`.
- `continuation-fixtures.patch`: `1fa942e8bcc47ffc0888715da7ec95f9d276d33d858783723524e933e2b6117a`.
- `continuation-goal.patch`: `3a957dd842e12e83090f279e010146ac616090d18f319f062810679d0b7ac8eb`.
- `continuation-producer-fixtures.patch`: `ce047b93319ca4460d87426027b1af9a288eb29eb72d3596b5a8fcc24b23a230`.
- `continuation-pty.patch`: `014b7a0bbb69c925172d9a1c84d87688e72735aab77aa0735d094c3de86612f7`.
- `continuation-pruning.patch`: `eaa09b1b5c9bf73d0074990541c5fe0474eee8278becdcd346559c3a180b7bd4`.
- `review-corrections.patch`: `ec3bbaefb14dee9773ba91a5f7838a9898ee2bcd10a99f3c0ef6b09791bdfa8d`.
- `review-event-sync.patch`: `60063d757b65d4f8772c0ee235164abd0ac7de06397fb01c69a772f185b0a8c8`.
- `review-replay.patch`: `8b770eb67dd6e5d4c2c1acf893963abaa689b5e900cd7909de659254df763676`.
- `review-pty.patch`: `63189ed8c5f24a63c845e62600befd027d0668e5e9a97f424576769c17dea0d2`.
- `review-pruning.patch`: `eaa09b1b5c9bf73d0074990541c5fe0474eee8278becdcd346559c3a180b7bd4`.
- `continuation-production.patch`: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
- `review-production.patch`: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
- `continuation-baseline/prior-twelve-files.patch`: `33d0c8d252f18da03046aff401b3c9805d9ac90cbc330b718e4930f66c7cc090`.
- `review-baseline/fixtures.patch`: `1e8186a49fd3f3caab253fb7a8085b6d03bfdf4be906459761c34d6e6382f89f`.

## Captured inputs

- `/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-format/result.md`: `ba2fbe2cb1635c8423af7d06d69fa0bc5b21304878d299885dbbf6813608271f`.
- `/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-format/review.md`: `4d1a5aeab0083b27e174a5dcaeed2b093f62d4fad7af5b84e9f0d76762455a07`.
- `/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-format/failed-ci.log`: `947825dd02848b285644f571263f6bcd63b8a394960f8bf645e758de53baf523`.
- `/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-format/failed-ci-readable.log`: `947825dd02848b285644f571263f6bcd63b8a394960f8bf645e758de53baf523`.
- `/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures/pty-focus-review.md`: `e1bb3474bde92c617660f9e81d4be5cbb4872fadd8d2c15f1d794389a5357ece`.
- `/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures/production-gap-explicit-stop.md`: `6ceaf79fa0e1e2a8bc3e3c5402c034c9d5011f08a3d18ba7216c6f05da1a2c11`.
- `/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures/review-findings.md`: `aadb2315fd19eeaed0d1be02d5444c0822dc14fb6cfc6f5991be306ff5d51106`.
