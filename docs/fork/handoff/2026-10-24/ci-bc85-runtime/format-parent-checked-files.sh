#!/usr/bin/env bash
set -euo pipefail
cd /home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format
rustup run nightly-2026-08-13 rustfmt --edition 2024 --config-path rustfmt.toml --config skip_children=true crates/codegen/xai-grok-shell/src/session/acp_session_tests/prompt_mode_transition_tests.rs crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row_opens_child.rs
git diff --stat -- crates/codegen/xai-grok-shell/src/session/acp_session_tests/prompt_mode_transition_tests.rs crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row_opens_child.rs
