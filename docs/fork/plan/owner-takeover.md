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

Progress 2026-10-24: steps 1–4 executed. The runtime checkpoint fast-forwarded
(`14a5f56c`), the canonical merge (`074f54b5`) is in progress in the worktree
`worktrees.d/org/forks/grok-build@owner-takeover` with five conflicted files:
`subagent_spawn_context_tests.rs`, `agent/subagent/tests/mod.rs`, `tests/rest.rs`,
`session/completion_delivery.rs`, `coordinator/agent_targets_tests.rs`.
Conflict list: `docs/fork/handoff/2026-10-24/wave-a/merge-conflicts.txt`; both checkpoint
sides of each path exported to `/home/fg/projects/tmp.d/grok-wave-a/baseline/{canonical,runtime}/`
(transient — git holds the authoritative bytes).

Dispatch surface: native subagents (resume-first), one merge author per conflicted file
plus a CI triage analyst, then an adversarial merge-fidelity refuter. A workflow-based
variant (`~/.grok/workflows/grok-owner-wave-a.rhai`) was tried and abandoned 2026-10-24:
the workflow agent surface rejects the anthropic Sigma slugs (HTTP 400 unknown model) and
Faruk ordered native subagents with session-id resume.

1. Resume the loaded contexts: file 1 (`subagent_spawn_context_tests.rs`) resumes the
   run-2 luna author (full inspection done, reported blocked on scope); files 2–4 resume
   the run-3 cancelled luna authors. File 5 (`agent_targets_tests.rs`) and the triage
   analyst spawn fresh (their opus jobs died in <1s on the workflow surface).
2. Triage analyst (read-only): classify the 27+2 failures from
   `docs/fork/handoff/2026-10-24/ci-bc85-runtime/diagnostics.json` and the capture
   `docs/fork/handoff/2026-10-24/ci-c8-fixtures/bc85-failed-ci.log` into env /
   addressed-by-checkpoints / open with file:line hypotheses (STATUS.md 'Known failures'
   is the env reference; the telemetry test has no assertion stdout and needs source
   analysis).
3. After all five files resolve: one adversarial refuter (read-only) checks dropped
   semantics against both exported sides, the three phase oracles
   (rest.rs ~563, mod.rs ~2514/2604) and the code rules.
4. Lead: review reports + refuter verdict, commit the resolution in coherent increments,
   relocate the checkpoint ADRs 001/002 out of `context-modules.d/` into `docs/fork/`
   (that tree stays sigma-repo-only) with the stream-file reference updated, push the
   branch, observe the exact commit's `linux-build` (+ scoped `linux-test`), iterate
   failures until green.
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
