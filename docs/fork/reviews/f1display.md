# F1-display — lead review of 851aee70 (binding)

`d2bb9590` + fixup are on main as `a80ccd98`. Rebase onto main (drop d2bb9590/acfc1a46, keep 851aee70's content).

## Reject / rework in 851aee70
1. **Implicit kind defaults again.** `DisplayDefaults::get` now falls back to `default_display_form(kind)` for every settings kind. That reintroduces exactly the regression fixed on main (`292a78cc`: every kind defaulted → push overrode block policies → 42 failing tests). Rule: `DisplayDefaults::get` returns only an explicit user choice. `default_display_form(kind)` may exist only as what the F7 row *shows* for an unset kind — a separate function the sidebar calls, never consulted on push.
2. **`Stub → MessageKind::UserPrompt`.** The design says Stub stays internal and never maps to a settings row. Mapping a test stub to UserPrompt silently subjects stubs to the user-prompt setting. Keep `MessageKind::Stub` (not in `settings_kinds()`), or make `MessageKind::from` return `Option` and exclude Stub. No merging of distinct kinds.
3. **Removed the pipeline one-row truncation** (`if ctx.mode == Collapsed && lines.len() > 1 { truncate(1) }` in `RenderBlock::output`). That is the single place that guarantees "Collapsed = one row at every width" (design §3). Keep it; per-block collapsed output may only improve *which* first line is shown.
4. **`toggle_fold` no longer checks `is_foldable()`**, and `next_fold_mode` default now maps `Truncated → Expanded` instead of `Collapsed`. Both change upstream fold behavior for every block (non-foldable blocks become foldable; collapsing a running Execute preview now expands). F7 must not route through `toggle_fold` at all (design: explicit form applies via state, not via `collapse_mode()`/`is_foldable()`), so leave upstream's `toggle_fold` / `next_fold_mode` semantics unchanged.
5. **Doc-comment churn.** ~15 doc comments rewritten without a behavior change ("Legacy …", "Truncated-height cache key: …"). Revert every doc edit whose code did not change (clean-diff: line history).
6. **`let _ = is_running;`** — keep upstream's form; no churn.

## Keep
- Per-block renderer changes that make the normal first line a meaningful one-row summary (tool blocks' label + key detail), if they do not change Expanded output.
- The `mode` key in the output cache (correct: cached output depends on display mode) — verify it is needed by a failing case, else drop.

Commit as focused commits (renderer first-line improvements per block group; cache key), each leaving upstream fold semantics intact.
