# Wave A per-file merge report — file 2: subagent/tests/mod.rs

Status: done (regions 737/892/2429/2543 by the resumed luna author; the six
remaining regions finished by the Lead after five premature worker stops;
zero conflict markers in the file and tree).

## Region decisions

- 737, 892, 2429, 2543 (worker): runtime durable-fixture FIFO observation with
  typed `SubagentCompletionWake` text and `GetToolOverrides` barrier; typed wake
  fixture merged with real input-queue assertions preserving the runtime
  receiver fixture; canonical durable fixture/admission alongside the
  runtime-migrated fixture setup.
- promote-cancel assertion block (~2352): both sides semantically identical
  (same admission/publish/persist assertions); HEAD kept for its fuller expect
  messages; base's `present_child_completion` + try_recv loop dropped (dead API).
- helper block (~2447): canonical added `running_meta_json` +
  local `pending_shell_completion_fixture`; runtime had deleted the block because
  `completion_test_support.rs` owns `pending_shell_completion_fixture` (pub(crate),
  imported at the header). Local duplicate dropped; `running_meta_json` kept
  initially, then removed after all regions resolved (zero remaining users).
- unproven-exit test (~2501): HEAD's full lifecycle kept (meta==initial after
  cancel; initial Running/completed_at None; before_commit no SubagentFinished;
  admission commit; producer ack Interrupted/Error; committed == expected_terminal
  full-field comparison incl. completed_at/duration 42/tool_calls 0/turns 0/error;
  snapshot_ref None; after_commit still no updates; publish Published; meta bytes
  unchanged; replay publish + byte stability + persisted-once). Canonical's
  "meta still == initial_metadata after prepare_completion_admission" byte
  assertion dropped: merged production materializes terminal metadata inside
  `prepare_completion_admission` after the durable ack
  (`completion_publication.rs` `upload_committed_metadata`), so the assertion
  contradicts the merged production model; its intent (nothing terminal before
  the durable commit) stays pinned by before_commit + the initial Running parse.
  Shared tail already used HEAD's variables (`committed_data`, replay publish).
- header: canonical's `#[path = "completion_fixtures.rs"] mod completion_fixtures;`
  + import dropped, doubled `StorageAdapter`/`completion_publication` imports
  deduped; single import set via `super::completion_test_support`.
- startup-admission-timeout test (~2583, ~2650): HEAD kept both regions
  (migrated durable fixture with attempt ids; full phase lifecycle with Failed
  terminal, expected_terminal comparison, replay FIFO observation, persisted-once,
  byte stability). Base's `running_meta_json` meta-dir setup and canonical's
  compact form with the superseded mid-phase assertion dropped.

## Phase oracles

The promote-cancel, unproven-exit and startup-timeout tests now each pin, through
real entry points (`prepare_completion_admission`, `prepared_producer`,
`publish_committed_completion`, real fixture storage): Running before the durable
summary commit; complete terminal values (status, cause, completed_at, duration,
counts) after producer acknowledgement; unchanged terminal values and byte
stability after publish and replay; persisted exactly once.

## Duplicate-fixture ownership decision

Canonical's `tests/completion_fixtures.rs` duplicates the concepts that the
runtime lane owns split across `completion_test_support.rs` +
`tests/completion_fixture.rs` + `tests/coordinator_fixture.rs` (richer: durable
persistence handle, observed command channel, real transport). One owner: the
runtime modules. `completion_fixtures.rs` is deleted in the integration commit;
mod.rs routes everything through `completion_test_support`.

No build was run (author-only; the consolidated CI run covers it).
