# Formatter diagnosis

- Workflow `.github/workflows/linux-test.yml:50` includes five packages: sampler, shell, tools, pager, and pager-pty-harness. The initial pass accidentally omitted the fifth package; corrected below.
- Baseline: `c0e3db3ff50410c7d1b4abc8b17beb346b5b2884` in isolated detached worktree `/home/fg/projects/worktrees.d/org/forks/grok-build@compaction-pty-format`.
- CI-equivalent formatting applied via pinned `rustup run nightly-2026-08-13 rustfmt --edition 2024 --config skip_children=true` to every tracked `.rs` file in the five exact package roots. Root `rustfmt.toml` honored. The original `cargo +nightly-2026-08-13 fmt ... -- --check` was not run; no builds/checks/tests were run.
- First four-package commit preserved: `b2b198801abd27795315e39c64494a368c22a285` (95 files, +1605/-562).
- Fifth-package correction: `02330d3ad44183d1de71cf885c6ac45813598234`, formatting `crates/codegen/xai-grok-pager-pty-harness/benches/paste_latency.rs` (+5/-1; 6-line diff hunk).
- A final formatting pass over all five packages was idempotent; `git diff --check` passed. Worktree clean.
- The new PTY compaction test itself was already correctly formatted; it was not changed. The only harness change was the authored bench file above.
- Exact changed paths: `/tmp/grok-goal-4f7553a1dbe6/implementer/compaction-pty/format-paths.txt`; commit stats: `/tmp/grok-goal-4f7553a1dbe6/implementer/compaction-pty/format-stat.txt`; captured check/verification output: `/tmp/grok-goal-4f7553a1dbe6/implementer/compaction-pty/format-check-output.txt`.
