#!/usr/bin/env bash
set -euo pipefail
cd /home/fg/projects/org/platform/sigma-crates
mode=${1:-check}
format_args=(--edition 2024 --config skip_children=true)
case "$mode" in
  check) format_args+=(--check) ;;
  apply) ;;
  *) exit 2 ;;
esac
rustup run nightly-2026-08-13 rustfmt "${format_args[@]}" \
  infra/gateway/src/server/responses_envelope.rs \
  infra/gateway/src/server/mod.rs \
  infra/gateway/src/server/sse.rs \
  infra/gateway/src/server/responses_chat.rs \
  infra/gateway/src/server/sse_anthropic_to_chat.rs \
  infra/gateway/src/server/sse_chat_to_responses.rs \
  infra/gateway/src/server/cg.rs \
  infra/gateway/src/routing/opaque_reasoning/responses_terminal.rs \
  infra/gateway/src/routing/opaque_reasoning/responses_terminal/tests.rs \
  infra/gateway/src/server/sse/tests.rs \
  infra/gateway/src/server/sse/tests/wire_contract.rs \
  infra/gateway/src/server/tests.rs \
  > /tmp/grok-goal-4f7553a1dbe6/implementer/missing-model/gateway-format-${mode}.log 2>&1
if [[ "$mode" == apply ]]; then
  exec bash "$0" check
fi
