# Fork instructions for Claude agents

This is Faruk's fork of `xai-org/grok-build` (`farukg/grok-build`). The fork adds visibility and
control to the Grok Build TUI. You continue work that local agents started; everything you need
is in this repository.

## Start here

1. Read `docs/fork/STATUS.md`: what is on `main`, which work streams exist, their branches and
   what comes next.
2. Read the stream file for your task in `docs/fork/streams/` and the documents it links
   (designs in `docs/fork/design/`, reviews in `docs/fork/reviews/`, briefs in `docs/fork/briefs/`).
3. `docs/fork/plan/grb-001.md` is the original milestone plan (German). Designs are newer and win
   where they differ.

Path shorthands used in the docs (all under `crates/codegen/`): `P/` = `xai-grok-pager/src/`
(the TUI), `S/` = `xai-grok-shell/src/` (session logic), `T/` or `TL/` =
`xai-grok-tools/src/implementations/grok_build/…`, `CS/` = `xai-chat-state/src/`,
`ST/` = `xai-grok-sampling-types/src/`.

## Sigma skills as the implementation bar (read first — not a review gate)

`docs/fork/skills/` carries three Sigma skills. Read them before implementing and hold your code
to them while you write it. They are guidance for **implementing**, not a review process to run
afterwards:

- `skills/sigma-review.md` — the Sigma Rust audit standard: modeling, error handling, boundaries,
  tests. Treat it as the quality bar new code must clear on arrival.
- `skills/clean-diff.md` — shape changes so per-file diffs stay reviewable.
- `skills/rebase-able-commits.md` — keep the commit sequence easy to rebase onto upstream.

Standing rules that go with them:

1. Integrate completed progress into `main` promptly — finished work lives on `main`, never rots
   on a side branch.
2. Keep the fork docs current with every landed change (`docs/fork/STATUS.md`, your stream file,
   the plan snapshot when milestones move).
3. Prefer NEW files for new code: a new module, test file or doc beats scattering additions
   through existing files.
4. In files that upstream also has, change as little as possible: smallest diffs, no drive-by
   reformat, reorder or rewrap of untouched lines — every changed upstream line is future rebase
   cost.

## What Faruk wants

- More visibility and more control in the TUI with few keystrokes. No AI slop, no feature
  copied from other tools for its own sake.
- **Nothing may get slower** (TUI, shell, server). No new per-frame work beyond the visible rows,
  no polling, no extra startup I/O. A small cost is acceptable only when clearly justified.
- "Make invalid states unrepresentable" and "if it compiles, it works" — unless that would force
  an architecture-scale rewrite of grok; then stop and report with file:line evidence.
- Good, self-explanatory code instead of comments. No unnecessary tests or comments.

## Code rules (binding)

- Closed sets are enums, with payloads for state-specific data. No booleans, strings or
  `Option` fields that encode state; no state-gated sibling fields.
- Exhaustive `match` on owned enums; no `_ =>` catch-all on types this repo owns.
- No `unwrap`/`expect`/`panic!`/`todo!`/`unimplemented!` outside tests; propagate
  `Result`/`Option`.
- One implementation per concept. Reuse and extend grok's existing owner; never add a parallel
  helper, lookup or classifier. Classify once where data is constructed; consumers match the
  result instead of recomputing predicates.
- Parse untyped/wire data once at the boundary into typed values. Persisted and wire formats stay
  backward compatible (old sessions must still load; new fields are additive with serde defaults).
- Do not store values that can be derived; derive on read.
- Root-cause fixes only. Never weaken a test or its expected value to make a bug pass.
- Comments only for non-obvious constraints. Do not rewrite existing doc comments whose code you
  did not change.
- Tests only for real behavior with an implementation-independent oracle, driven through real
  entry points (`AppView`/`AgentView` input and render, ACP handlers, the session actor).
- Clean diffs: touch only what you change, no reformatting or reordering of untouched code,
  follow the conventions of the file you edit (e.g. tests in sibling `*_tests.rs` files).

## Build and test

- Verification for this fork runs on GitHub Actions. Check the implementation statically, commit and push to `farukg/grok-build`, then observe the exact commit's `linux-build.yml` run. Fix build errors and publish the correction until that run succeeds. Local Grok CLI builds and test runs are excluded from this workflow.
- After a successful GitHub build, use `/home/fg/projects/org/glue/workspace-scripts/sigma-ops.d/stage-grok-release.sh --run-id <id> --commit <full-sha>` to download, verify and stage that exact artifact when staging is requested. Preserve running sessions; the script prints restart instructions.
- Toolchain: pinned in `rust-toolchain.toml` (nightly); `rustup` installs it.
- `protoc` is required. Either install DotSlash (`cargo install dotslash`) so `bin/protoc` works,
  or install protoc from the OS (`apt-get install -y protobuf-compiler`) and export
  `PROTOC=$(command -v protoc)`.
- `linux-build.yml` builds the release artifact on pushes to `main`. Use `linux-test.yml` for explicitly requested CI tests; scope test filters to the changed behavior.
- Known failures that are not yours: four `doctor` tests fail on upstream too (see
  `docs/fork/STATUS.md`).

## Git rules (binding)

- Work on the branch named in your stream file. Commit small, coherent increments with
  conventional subjects (`feat(pager): …`, `fix(shell): …`, `test(pager): …`, `refactor(…): …`)
  and a 1–4 line body saying what and why.
- **No AI attribution anywhere**: no `Co-Authored-By`, no "Generated with …" line, no `Review:`
  trailer, in commits or anywhere else.
- Before you start a stream, rebase its branch onto the current `main` so it builds on the latest
  state.
- Never rewrite `main`'s history and never force-push `main`. Every other branch may be
  force-pushed (use `--force-with-lease`). Never delete branches or other agents'
  work; commit work in progress before you switch context.
- Push only to this fork (`farukg/grok-build`). **Never push to, or open a pull request against,
  `xai-org/grok-build`.** Check the target repository of every push.
- A stream is finished when its progress is on `main` and the exact commit's GitHub build is green. Publish scoped commits as a normal fast-forward to `farukg/grok-build`, observe CI, and fix failures in follow-up commits. Update `docs/fork/STATUS.md` with the verified outcome.
- If your environment only lets you push to a `claude/…` branch, push the work there and say so;
  `main` still has to receive it before the stream counts as done.

## Reporting

- After each increment update your stream file in `docs/fork/streams/` (commits, what works,
  what is next, open questions with file:line evidence). Do not edit other streams' files;
  `docs/fork/STATUS.md` is updated only when something lands on `main`.
- Messages to Faruk: German, short, results only (what changed, what is verified, what is open).
