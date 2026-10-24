# Independent Pager review — 2026-10-23

Reviewer: 01a16838-5f1d-74c0-a97f-ed71d2c5ff7a, GPT-6.1 Sol. Read-only role could not write result; parent preserves returned inline evidence here. Static review only, no builds/tests/runtime interaction.

## Findings

P1: Todo focus remains on invisible surface after global preference propagation or Dock suppression. app/app_view.rs:2241–2245 sets visibility without repairing active_pane==Todo; app/agent_view/render.rs:1001–1008 can suppress Todo; input.rs:973–979 continues routing keyboard input to Todo independently of geometry/overlay.focused. Static repro: focus visible Todo in ViewA, switch to B, Ctrl+T global hide, returnA; or shrink focused Todo view to <=16rows. Existing TodoPane::on_state_change contract should be preserved and focus transferred by existing owner. Add observable keyboard routing regression.

P2: app/app_view.rs:5738–5748 distributes global preferences to all views at each draw (call app_view.rs:4265), including inactive views with unchanged preferences. No measured runtime slowdown claimed. Existing preference-change events and new-view initialization should own propagation; avoid unconditional per-frame writes to all views.

P3: docs/fork/streams/f2-f3-sidebars.md stale statements: line9 'closed means invisible (no dock, no queue above the prompt)', line11 queued in F7-dock inventory, line27 blanket no dock work with F7 closed, line39 'F7 closed means no snapshot, no render, no queue above the prompt'. Current requirement separates F7 and optional upper projection, queue independently immediately before Composer. Approved narrow healing: replace these stale statements at this existing owner preserving other contracts.

## Other observations

TodoPane::desired_height already checks visibility (todo_pane.rs:270–273); new explicit is_visible gate alone is no demonstrated functional fix. Dock predicate carries independent projection policy. New render/session tests have meaningful painting/state/store oracles and no statically identified fixture/API compilation defect; they omit focus loss and genuinely narrow widths. pin_theme available via test-support dependency. Global TOML owner, scope classification, session-override rejection and selective read-modify-write show no further confirmed persistence defect. Queue is immediately before Composer. 160cols24/40rows is not narrow-terminal evidence. Full GitHub-CI and actual PTY remain pending. This is Terminal Ratatui; browser verification does not apply.
