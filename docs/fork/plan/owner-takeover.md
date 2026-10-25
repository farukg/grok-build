# Owner-takeover execution plan (Krillin, updated 2026-10-25)

Status: started work was integrated at `7ec4c932653b9a777db24e32b3200935d777b5e3`;
release build [run 40242190071](https://github.com/farukg/grok-build/actions/runs/40242190071)
failed at `5f50af26` with four shell compiler errors. Scoped S/D integration corrections
and removal of two obsolete chat-state imports are underway on `main`. No checks or
tests were launched. Release success, staging and runtime acceptance remain pending;
record artifact proof only from a successful actual GitHub release Run/Commit record.
No new feature or review project is authorized.

**Verification boundary: no checks, tests, formatters, local builds, or `sig build`;
no sig-orch communications.** This also forbids compiler/formatter verification
commands, indirect test scripts and hooks. Source reading/editing only; the only
verification in this delivery scope is the necessary GitHub release build after
integration. Historical local-guard instructions are superseded.

## Current execution baseline

- Preserved canonical `main` commit: `875ff73c`.
- Preserved owner commit: `e7ceeb50` in
  `/home/fg/projects/worktrees.d/org/forks/grok-build@owner-takeover`.
  Source integration began with `7ec4c932653b9a777db24e32b3200935d777b5e3`;
  release run `40242190071` failed at `5f50af26`. Four shell compile errors were
  reported: missing `fs` import, obsolete `observation.text`, sibling visibility for
  `project_notification_batch`, and receipt unit/bool return mismatch. Scoped S/D
  corrections and two obsolete chat-state import removals are underway on `main`.
  No checks or tests were launched; preservation commits are not the final source tip.
- Historical Wave A baseline: `5abbd3c5599c18133ea271049d8c3da3510cc736`.
  Its checkpoint merge completed before preservation; historical acceptance facts
  are retained below and do not describe current source acceptance.
- `linux-build` run `40128349776` succeeded at
  `5abbd3c5599c18133ea271049d8c3da3510cc736`.
- `linux-test` run `40128351855` completed as `cancelled` at the same SHA after
  Faruk authorized aborting the stalled subagent-delivery/live-steering step.
  GitHub returned `log not found: 120974515533` after cancellation. Historical
  local captures identify coordinator failures, two coordinator hangs and an
  active-tool steering hang; their fixes and combined acceptance remain pending.
- Historical isolated telemetry result at `5abbd3c5599c18133ea271049d8c3da3510cc736`:
  1 passed, 0 failed, 0.13s. The historical assertion failure's cause remains unknown.
  This prior local result is historical evidence only, not proof of current source
  or combined CI acceptance, and does not authorize further local execution.
- Use native subagents only, resume-first where a usable session exists. No model
  pinning and no workflow-based dispatch.
- All local Grok builds and test runs are forbidden, including `sig build` and
  local cargo compilation/tests. Compilation and Rust tests run exclusively in
  GitHub Actions, after a full source review by reading and review.
- This restriction is forward-looking: preserve correct source work and the
  verification worktree. Last local test `bld-1792890778976-5daa03f867344a02bf49e33bfa54b8e2`
  was cancelled; worker `3682143` is confirmed absent.
- No force push is authorized, including on the integration branch.

## Input and historical evidence

- Handoff: `docs/fork/handoff/2026-10-24/owner-handoff/handoff.md` (Sokrates → Krillin).
- Checkpoints: `handoff/grok-canonical-20261024` = `074f54b5` (base `c8e075ea`, 27 paths,
  attempt-admission lane), `handoff/grok-runtime-20261024` = `14a5f56c` (base `bc85ceb9`,
  14 paths, runtime repair lane). Overlapping files: `agent/subagent/tests/mod.rs`,
  `agent/subagent/tests/rest.rs`, `acp_session_tests/auto_wake_suppression_tests.rs`,
  `task/coordinator_tests.rs`.
- Historical CI at `bc85ceb9`: build run `40058325363` green; test run `40058633091`
  failed with 27 distinct FAILED tests + 2 hangs (`ci-bc85-runtime/diagnostics.json`,
  full capture `ci-c8-fixtures/bc85-failed-ci.log`). This is triage input, not the
  current branch's outcome.
- Wave A evidence: `docs/fork/handoff/2026-10-24/wave-a/`. The conflict manifest and
  blocked `merge-oracles-report.md` describe an earlier merge stage, not current
  unresolved conflicts. Exported checkpoint sides under
  `/home/fg/projects/tmp.d/grok-wave-a/baseline/{canonical,runtime}/` are transient;
  checkpoint commits hold the authoritative bytes.
- The later `wave-a/refute.md` reports no remaining conflict markers and sound phase
  oracles, but records one dropped SubagentCompleted shutdown assertion and two nits
  (import formatting and a misleading replay-test comment). Merge completion alone
  does not prove these findings closed; reconcile them against the current author
  tree and record disposition before acceptance.
- Faruk's live-binary issue list (Krillin session todos `gb-*`) maps to the lanes below.

## Wave A — coherent snapshot, oracle completion and failure closure

The branch was cut from `origin/main` at `bc85ceb9`; runtime checkpoint fast-forward
and canonical checkpoint merge are now historical completed integration steps. Do
not restart conflict-resolution dispatch from the old five-file manifest.

1. Lead reviews current integrated changes and existing author/refuter reports. Use
   native author workers for remaining root-cause fixes and a read-only adversarial
   refuter for merge fidelity and review closure.
2. Preserve both checkpoints' semantics and producer-ack phase oracles: running
   metadata before commit, full terminal metadata after producer acknowledgement,
   and unchanged terminal values after publish/replay. Historical locations were
   `rest.rs` ~563 and `mod.rs` ~2514/2603; resolve current locations rather than relying
   on stale line numbers.
3. Reconcile the historical 27+2 failures using diagnostics and capture into env /
   addressed-by-checkpoints / open with file:line evidence. `STATUS.md` known failures
   is the env reference; the telemetry failure needs source analysis if capture has
   no assertion stdout. Do not classify an old failure as fixed without evidence.
4. Verify disposition of checkpoint ADRs 001/002: their owner is `docs/fork/`, not
   `context-modules.d/` (sigma-repo-only), with stream references kept accurate.
5. Complete refuter + lead review and the full source review by reading, including
   typo/API self-check. Integrate coherent commits and push normally to the fork.
   Do not run local compilation or tests.
6. Historical wave plan called for one consolidated `linux-build` + `linux-test`
   pair. That historical instruction is not authorization for tests in the current
   delivery scope.
7. Historical acceptance gate remains unmet: the baseline build succeeded, its test
   run was cancelled, and no current final-SHA acceptance is established. The current
   scope authorizes only the necessary GitHub release build after started-work
   integration; no tests or review project are authorized here.

## Wave B — Faruk's issue list + carried-over orders

This table records previously started lane scope, not new implementation
authorization. Preserve and integrate existing work only; no new features or review
projects are in scope.

| Lane | Covers | Entry evidence |
|---|---|---|
| Completion/delivery transaction | `gb-wake-1` completion wake; `gb-waitsem-2` wait-state/stop-hook semantics; restart-proof receipts, exactly-once projection | `notes/delivery-*`, `delivery-shell/`, goal-plan orders, ADR 001/002 |
| Compaction | `gb-compact-run-3` submit-while-running, no input-abort, explicit manual abort; `gb-compact-sub-4` queue-through for running subagents | `compaction-pty/`, failed test `child_prompt_tests::finished_coordinator_child_compacts_persisted_checkpoint_without_parent_delivery` |
| Dashboard subagent sessions | `gb-dash-sub-5` session-list switch, same sorting/view/actions; `gb-subid-6` lost session ids | `notes/pager-global-display.md`, session-views stream |
| Background-thing visibility | `gb-zombie-7` invisible/duplicated tasks-monitors-subagents, zombie question | `notes/pager-global-restart-*.md`, `persistence_cleanup_stale_sessions` failures |
| Carried-over remainder | dynamic parent-goal evaluation + AppView replay; global task/todo display prefs; work items in F7 (+optional top, queued above input); mid-run steer + tool/completion wire contracts | `goal-plan.md` checklist, `notes/requirements.md` |

### Wave B interaction and pane requirements

Status: begun implementation is integrated on `main`; runtime and interaction
acceptance remain pending/unverified. The dashboard lane owns dashboard navigation;
the carried-over remainder lane owns
modal routing, focus continuity, Dock, Context and Todos integration. Reuse existing
owners and components; Lead integrates cross-lane behavior and acceptance evidence.

- Modals: Enter never pastes into chat input. It selects the applicable item, or is
  a no-op when none applies. `y` copies. Repeated Esc unwinds one layer at a time,
  then returns focus to input.
- Dashboard: Ctrl+Alt+Up navigates from the root session to the dashboard;
  Ctrl+Alt+Down returns from the dashboard to the originating session. Dashboard
  Ctrl+Alt+Left/Right switches Main/Subagents.
- Focus: Tab cycles all visible regions, including an empty Dock. Session switches
  preserve the focused visible pane.
- Dock: placement is a closed set of Off/Top/Bottom. Bottom retains the current
  lower position; Top uses the prior upper position. F7 always has its own independent
  Dock section, including when placement is Off or the Dock is empty.
- Context: a focused OFF row never overlaps neighboring rows.
- Todos: reuse the existing Todo owner, model and component, with no duplicate
  implementation. F7 has a permanent collapsible Todos section immediately above
  Context, independent of the existing Ctrl+T pane's visibility.
- Global pane state: Ctrl+G Tasks and Ctrl+T Todos toggles are global like F6/F7,
  preserved across session switches, new sessions and restarts, with focus continuity.

These historical lane instructions do not authorize new workers, review projects,
checks, tests, formatters, or sig-orch communication for the current preservation and
integration scope. Preserve only work already started; gateway/wire coordination is
outside this scope.

## Historical Wave C — final acceptance and staging

The following is prior delivery planning, not authorization for new review or tests
in the current delivery unit. Current integrated `main` is `5f50af26`; release build
run `40242190071` is in progress, with no success or staging claimed.

1. Confirm review closure, the full source review by reading, and same-SHA green
   GitHub Actions build + relevant tests; record exact run IDs and conclusions.
2. Stage via `org/glue/workspace-scripts/sigma-ops.d/stage-grok-release.sh --run-id <id> --commit <sha>`.
3. Report to Faruk explicitly whether grok-CLI restart suffices or the grok-leader service
   must restart too (`docs/fork/recon/runtime-environment.md`).
4. Verify the actually selected + running grok stand from the staged artifact, not from
   older release paths.
5. Real terminal/PTY acceptance on the running binary, including Wave B modal key
   routing, dashboard round-trip and Main/Subagents navigation, all-visible-region
   Tab traversal, pane focus continuity, Dock placements and independent F7 section,
   OFF Context row layout, reused Todos section, and global Tasks/Todos toggle
   persistence across session switches, new sessions and restarts. These interaction
   requirements remain pending/unverified until evidence is recorded. Do not claim
   staging, runtime selection or acceptance complete without evidence.

## Verification and ownership boundaries

- Historical CI quota and wave-pair instructions do not override the current scope:
  only the necessary GitHub release build after integration is authorized. No local
  builds/checks/tests/formatters, `sig build`, or sig-orch communications.
- Pushes only to `farukg/grok-build` (`origin`); never to `xai-org/grok-build`.
- Normal pushes only. No force push is authorized on any branch.
- Preserve and integrate started work only; no new features or review projects.
- Historical review, test, and runtime acceptance gates remain unverified and are not
  claimed complete by the current preservation/integration work.
