# Owner-takeover dispatch plan (Krillin, 2026-10-24)

Status: planned — implementation starts only after Faruk's explicit GO.

## Input

- Handoff: `docs/fork/handoff/2026-10-24/owner-handoff/handoff.md` (Sokrates → Krillin).
- Checkpoints: `handoff/grok-canonical-20261024` = `074f54b5` (base `c8e075ea`, 27 paths,
  attempt-admission lane), `handoff/grok-runtime-20261024` = `14a5f56c` (base `bc85ceb9`,
  14 paths, runtime repair lane). Overlapping files: `agent/subagent/tests/mod.rs`,
  `agent/subagent/tests/rest.rs`, `acp_session_tests/auto_wake_suppression_tests.rs`,
  `task/coordinator_tests.rs`.
- CI evidence at `bc85ceb9`: build run `40058325363` green; test run `40058633091` failed
  with 27 distinct FAILED tests + 2 hangs (`ci-bc85-runtime/diagnostics.json`,
  full capture `ci-c8-fixtures/bc85-failed-ci.log`).
- Open review P1: three metadata phase oracles in `rest.rs:563`, `mod.rs:2514`, `mod.rs:2603`
  (`ci-bc85-runtime/review.md`); the interrupted author was migrating exactly these.
- Constraint: builds and tests run ONLY via commit/push + GitHub Actions. No local cargo.
  Model pinning for authoring: GPT-6 Luna / GPT-6.1 Sol where the prior orders demand it.
- Faruk's live-binary issue list (see Krillin session todos `gb-*`) maps onto the
  carried-over orders; mapping below.

## Wave A — coherent snapshot, green CI (merge + oracle completion + failure closure)

Branch: `grb/owner-takeover` cut from `origin/main` (`bc85ceb9`).

Dispatch surface: workflow `grok-owner-wave-a` (`~/.grok/workflows/grok-owner-wave-a.rhai`,
smoke-checked 2026-10-24). Three agents: merge+oracle author (`sigma/openai/gpt-6-luna`,
read-write), CI triage analyst (`sigma/anthropic/claude-sonnet-5-5`, read-only) in one
parallel wave; then merge-fidelity refuter (`sigma/anthropic/claude-opus-5-5`, read-only).
Lead launch runbook:

1. `git worktree add /home/fg/projects/worktrees.d/org/forks/grok-build@owner-takeover -b grb/owner-takeover bc85ceb9`
2. In the worktree: `git merge --no-commit handoff/grok-runtime-20261024` (same base,
   clean), then `git merge --no-commit handoff/grok-canonical-20261024` (conflicts surface).
3. `git diff --name-only --diff-filter=U > <wave-a>/merge-conflicts.txt`.
4. Export both checkpoint sides of every conflicted path:
   `git show 074f54b5:<path>` → `<wave-a>/baseline/canonical/<path>`,
   `git show 14a5f56c:<path>` → `<wave-a>/baseline/runtime/<path>`.
5. Launch the workflow with args: worktree, conflicts, baseline, reports (all under
   `context-modules.d/500_work.d/epics.d/product.d/grok-build/wave-a/`), runtime_wt
   (`worktrees.d/org/forks/grok-build@ci-c8-format`), diag (`docs/fork/handoff/2026-10-24/ci-bc85-runtime/diagnostics.json`),
   capture (`docs/fork/handoff/2026-10-24/ci-c8-fixtures/bc85-failed-ci.log`).
6. On completion: Lead reviews the merge report + refuter verdict; triage report files the
   classification; then commit the resolution in coherent increments, push the branch,
   observe the exact commit's `linux-build` (+ scoped `linux-test`), iterate failures
   (fix wave via a second workflow once real CI evidence exists).

1. Lead: create integration worktree, `git merge` both handoff branches; surface conflicts.
2. Worker A1 (merge + phase oracles, one subject — same files): resolve the four overlap
   files semantically (attempt-admission fixtures vs runtime wake-fixture migration) and
   finish the three interrupted phase oracles per review.md P1. Author only; no build.
3. Worker A2 (CI failure triage + fixes): from `diagnostics.json` + capture, partition the
   27+2 into: addressed by checkpoint content, known environment failures (STATUS.md list:
   `agent::config` ×3, `mcp_file_input` ×3, `unified_list` ×1), and genuinely open;
   root-cause the open ones from source + assertion output and author fixes
   (incl. `tool_call_telemetry` — no assertion stdout in capture, needs source analysis).
   Author only; no build.
4. Lead: review diffs, commit in coherent increments, push to `farukg/grok-build`
   integration branch, observe the exact commit's `linux-build` (+ scoped `linux-test`
   where warranted), iterate failures until green.
5. Gate: same final SHA green in build + relevant tests.

## Wave B — Faruk's issue list + carried-over orders (parallel lanes after A, or on GO per lane)

| Lane | Covers | Entry evidence |
|---|---|---|
| Completion/delivery transaction | `gb-wake-1` completion wake; `gb-waitsem-2` wait-state/stop-hook semantics; restart-proof receipts, exactly-once projection | `notes/delivery-*`, `delivery-shell/`, goal-plan orders, ADR 001/002 |
| Compaction | `gb-compact-run-3` submit-while-running, no input-abort, explicit manual abort; `gb-compact-sub-4` queue-through for running subagents | `compaction-pty/`, failed test `child_prompt_tests::finished_coordinator_child_compacts_persisted_checkpoint_without_parent_delivery` |
| Dashboard subagent sessions | `gb-dash-sub-5` session-list switch, same sorting/view/actions; `gb-subid-6` lost session ids | `notes/pager-global-display.md`, session-views stream |
| Background-thing visibility | `gb-zombie-7` invisible/duplicated tasks-monitors-subagents, zombie question | `notes/pager-global-restart-*.md`, `persistence_cleanup_stale_sessions` failures |
| Carried-over remainder | dynamic parent-goal evaluation + AppView replay; global task/todo display prefs; work items in F7 (+optional top, queued above input); mid-run steer + tool/completion wire contracts | `goal-plan.md` checklist, `notes/requirements.md` |

Each lane: explore/author workers (author only, no build), Lead integrates, commits,
pushes, CI-observes per increment. Gateway/wire needs go to Sokrates
(`orc.direct.Sokrates`) with concrete wire/request/receipt evidence, producer/consumer
change and rollout boundary — never unilateral gateway edits (outside grok-build scope).

## Wave C — final acceptance and staging

1. Same-SHA green build + relevant tests.
2. Stage via `org/glue/workspace-scripts/sigma-ops.d/stage-grok-release.sh --run-id <id> --commit <sha>`.
3. Report to Faruk explicitly whether grok-CLI restart suffices or the grok-leader service
   must restart too (`docs/fork/recon/runtime-environment.md`).
4. Verify the actually selected + running grok stand from the staged artifact, not from
   older release paths.
5. Real terminal/PTY acceptance on the running binary.

## Verification boundaries

- Workers author only; every build/check/test runs through GitHub Actions after the batch
  is integrated (repo rule; no local cargo).
- Pushes only to `farukg/grok-build` (`origin`); never to `xai-org/grok-build`.
- No force-push on `main`; integration branch may be force-pushed with lease.
- Stream file updates per lane; `docs/fork/STATUS.md` only when something lands on `main`.
