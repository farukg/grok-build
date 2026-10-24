# Runtime environment: grok-leader service and CLI clients

How Faruk runs the built `grok` binary. Binding for every staging and
verification decision in this fork.

## Operating model (Faruk, 2026-10-24)

- Faruk starts the **grok-leader service first**, then the grok CLI. Every
  further `grok` invocation connects to that one leader.
- There is always **exactly one grok-leader** and one or more grok-CLI
  processes talking to it. Assume this topology; never assume a leaderless or
  multi-leader setup.
- Reconnect after a leader restart does not work well currently: whenever the
  grok-leader restarts, Faruk restarts the grok CLI as well.

## Staging duty

After staging a new binary via
`org/glue/workspace-scripts/sigma-ops.d/stage-grok-release.sh`, always tell
Faruk explicitly whether restarting the grok CLI is sufficient or whether the
grok-leader service must also be restarted. Never leave that answer implicit
in the script's generic restart instructions.
