# Owner-takeover execution plan (Krillin, updated 2026-10-25)

Status: implementation is authorized and in progress. No additional GO is required for
this scope. Acceptance, staging and final same-SHA CI success are not yet established.

## Current execution baseline

- Author tree: `/home/fg/projects/worktrees.d/org/forks/grok-build@owner-takeover`.
- Branch: `grb/owner-takeover`; Wave A baseline:
  `5abbd3c5599c18133ea271049d8c3da3510cc736`. The checkpoint merge is completed;
  subsequent author changes and preservation commits remain unverified.
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
  GitHub Actions, after a full static/source-format pass and review.
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
5. Complete refuter + lead review and the full static/source-format pass, including
   typo/API self-check. Integrate coherent commits and push normally to the fork.
   Do not run local compilation or tests.
6. Use one consolidated `linux-build` + `linux-test` pair for the wave. Observe the
   existing runs before deciding whether any additional run is justified; do not
   launch replacements merely because this plan was refreshed. Extra CI is allowed
   only after capture-based root-cause analysis and only at the minimum needed scope.
7. Gate: verified green build + relevant tests on the same final SHA, with review
   findings and the full static/source-format pass recorded. The baseline build succeeded;
   its test run was cancelled, so final combined acceptance remains pending.

## Wave B — Faruk's issue list + carried-over orders

Implementation is authorized. Coordinate lane dependencies with Wave A; authorization
is not evidence that Wave A's acceptance gate has passed.

| Lane | Covers | Entry evidence |
|---|---|---|
| Completion/delivery transaction | `gb-wake-1` completion wake; `gb-waitsem-2` wait-state/stop-hook semantics; restart-proof receipts, exactly-once projection | `notes/delivery-*`, `delivery-shell/`, goal-plan orders, ADR 001/002 |
| Compaction | `gb-compact-run-3` submit-while-running, no input-abort, explicit manual abort; `gb-compact-sub-4` queue-through for running subagents | `compaction-pty/`, failed test `child_prompt_tests::finished_coordinator_child_compacts_persisted_checkpoint_without_parent_delivery` |
| Dashboard subagent sessions | `gb-dash-sub-5` session-list switch, same sorting/view/actions; `gb-subid-6` lost session ids | `notes/pager-global-display.md`, session-views stream |
| Background-thing visibility | `gb-zombie-7` invisible/duplicated tasks-monitors-subagents, zombie question | `notes/pager-global-restart-*.md`, `persistence_cleanup_stale_sessions` failures |
| Carried-over remainder | dynamic parent-goal evaluation + AppView replay; global task/todo display prefs; work items in F7 (+optional top, queued above input); mid-run steer + tool/completion wire contracts | `goal-plan.md` checklist, `notes/requirements.md` |

### Wave B interaction and pane requirements

Status: requirements authored; implementation and acceptance pending/unverified.
The dashboard lane owns dashboard navigation; the carried-over remainder lane owns
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

Each lane uses native explore/author workers; Lead owns integration and verification.
Workers remain author-only; Lead ensures review and a full static/source-format
pass before the wave's consolidated CI pair. Compilation and Rust tests run only
in GitHub Actions; no local builds or test runs. Coherent commit increments
do not each trigger a separate CI cycle. Gateway/wire needs go to Sokrates
(`orc.direct.Sokrates`) with concrete wire/request/receipt evidence, producer/consumer
change and rollout boundary — never unilateral gateway edits (outside grok-build scope).

## Wave C — final acceptance and staging

1. Confirm review closure, the full static/source-format pass, and same-SHA green
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

- **GitHub Actions quota is scarce and binding**: one consolidated `linux-build` +
  `linux-test` pair per wave, only after refuter + lead review and a full
  static/source-format pass, including typo/API self-check. No local Grok builds
  or test runs are permitted; compilation and Rust tests are CI-only. Analyze run captures before fixes;
  additional CI requires capture-root-caused justification and minimum necessary scope.
- Native subagents only; no model pinning and no workflows for dispatch.
- Pushes only to `farukg/grok-build` (`origin`); never to `xai-org/grok-build`.
- Normal pushes only. No force push is authorized on any branch.
- Stream file updates per lane; `docs/fork/STATUS.md` only when something lands on `main`.
- Implementation authorization does not waive review, the static/source-format pass, same-SHA CI,
  runtime acceptance, or evidence requirements.
