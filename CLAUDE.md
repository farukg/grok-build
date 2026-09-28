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

- Toolchain: pinned in `rust-toolchain.toml` (nightly); `rustup` installs it.
- `protoc` is required. Either install DotSlash (`cargo install dotslash`) so `bin/protoc` works,
  or install protoc from the OS (`apt-get install -y protobuf-compiler`) and export
  `PROTOC=$(command -v protoc)`.
- Scope every command to the crates you touch, for example
  `cargo check -p xai-grok-pager -p xai-grok-shell`, `cargo test -p xai-grok-pager <filter>`,
  `cargo clippy -p xai-grok-pager -p xai-grok-shell -p xai-grok-tools --tests`.
  The pager crate has about 10,000 tests: use test filters while iterating and run the full
  crate once before you report an increment as done.
- Known failures that are not yours: four `doctor` tests fail on upstream too (see
  `docs/fork/STATUS.md`).

## Git rules (binding)

- Work on the branch named in your stream file. Commit small, coherent increments with
  conventional subjects (`feat(pager): …`, `fix(shell): …`, `test(pager): …`, `refactor(…): …`)
  and a 1–4 line body saying what and why.
- **No AI attribution anywhere**: no `Co-Authored-By`, no "Generated with …" line, no `Review:`
  trailer, in commits or anywhere else.
- Never rewrite published history and never force-push. Never delete branches or other agents'
  work; commit work in progress before you switch context.
- Push only to this fork (`farukg/grok-build`). **Never push to, or open a pull request against,
  `xai-org/grok-build`.** Check the target repository of every push.
- A stream is finished only when `main` contains its progress. Integrating means: once the
  stream's build and tests are green, bring its work onto `main` yourself (a squash of the stream
  into a few clean commits on top of `main`, pushed as a fast-forward) and update
  `docs/fork/STATUS.md`. No pull request, no waiting for a merge.
- If your environment only lets you push to a `claude/…` branch, push the work there and say so;
  `main` still has to receive it before the stream counts as done.

## Reporting

- After each increment update your stream file in `docs/fork/streams/` (commits, what works,
  what is next, open questions with file:line evidence). Do not edit other streams' files;
  `docs/fork/STATUS.md` is updated only when something lands on `main`.
- Messages to Faruk: German, short, results only (what changed, what is verified, what is open).
