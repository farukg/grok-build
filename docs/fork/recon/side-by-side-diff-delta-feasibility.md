# Side-by-side diff display and delta feasibility recon

Scope: read-only recon of the structured-diff display pipeline. Fork root `/home/fg/projects/org/forks/grok-build`, main HEAD `bba92132d59e1a82a2a939dd26c77df3958c710f` (line numbers refer to that commit). No source was modified. Companion probe evidence: local `delta` 0.20.1 render captured during recon (excerpt in §4.3).

One-line answers up front:

1. **Is side-by-side implemented today?** No. Every diff surface renders strictly sequential unified rows; `dual_line_numbers` is GitHub-unified (two number columns, one content column), and no `[scrollback.blocks.edit]` key changes that.
2. **How could it be activated?** Add a `side_by_side: bool` to `RawEditBlockConfig`/`EditBlockConfig`, map it into `DiffRenderConfig` at the existing conversion site (`edit.rs:1260-1266`), and add a split-row assembly branch next to `assemble_diff_line_outputs` (§2).
3. **Do crates provide the prerequisites?** Yes for everything except ready-made split layout: in-tree `similar` 2.7 (+ optional `inline` feature for word-level highlights), `syntect` 5.3 + `two-face` 0.4, `anstyle-parse` 0.2. `git-delta` on crates.io is `[[bin]]`-only (no `[lib]`), so delta reaches grok only as a subprocess (or a heavy vendoring) (§3).
4. **Is delta's rendering easily integrable?** Not "easily". Grok already has the core visual (syntect FG spans over insert/delete background bands, unified form); delta would add split+wrap alignment and word-level highlights, but at the cost of an external binary, an ANSI re-parse layer, and a theme system that bypasses grok's quantization/terminal-native guarantees (§4). Recommendation: in-house split on existing infra (§4.4).

## 1. The fork's structured-diff display pipeline

### 1.1 Hunk construction — `crates/codegen/xai-grok-pager-diff/src/lib.rs`

| Function | Role |
|---|---|
| `build_diff_hunks` (`lib.rs:28`) | `SearchReplaceEditDetail` records → `Vec<DiffHunk>` via `similar::TextDiff::from_lines` (`lib.rs:71`), with `line_prefix` re-attachment, `MAX_CONTEXT=3` trimming, blank-context stripping. |
| `diff_hunks_from_strings` (`lib.rs:165`) | whole-file old/new text → grouped hunks (`TextDiff::configure().timeout(WHOLE_FILE_DIFF_TIMEOUT)` + `grouped_ops(MAX_CONTEXT)`). |
| `extract_edit_hunks` (`lib.rs:318`) | ACP `ToolCall` → hunks, three strategies: `raw_output` as `ToolOutput::SearchReplace`, `ToolCallContent::Diff.meta` as `SearchReplaceEditContextInformation`, fallback full-text diff from `Diff.old_text/new_text`. |
| `stitch_overlapping_hunks` (`lib.rs:205`) | folds consecutive same-file edits into one hunk in post-state `ln` coordinates (bails to separate hunks when coordinates would lie). |
| `diff_hunks_to_patch` (`lib.rs:383`) | hunks → unified diff patch text (`--- a/`, `+++ b/`, `@@ -l,c +l,c @@`) — this is the ready-made stdin payload for any external renderer. |

`DiffHunk = Vec<DiffLine>`, `DiffLine { text, lo (old line), ln (new line), tag: similar::ChangeTag }` (`lib.rs:12-18`). Line-tagged and line-numbered on both sides — the data model already carries everything a split pairing needs.

### 1.2 Rendering — `crates/codegen/xai-grok-pager/src/scrollback/blocks/tool/edit.rs`

- `DiffRenderConfig` (`edit.rs:73-88`): `indent`, `gutter_bg`, `indent_bg`, `hunk_separator`, `dual_line_numbers`. Defaults at `edit.rs:90-99`.
- `render_diff_hunks_core` (`edit.rs:149`) is the single hunk walker behind all four public fronts (`render_diff_hunk_highlighted`, `render_diff_hunks_highlighted`, `render_diff_hunks_with_styles`, `EditToolCallBlock::render_diff_lines` at `edit.rs:1005`). Per hunk: `gutter_layout` (`edit.rs:569`), two syntect highlighters (old side + new side, `edit.rs:197-199`), per-`DiffLine` span assembly.
- `assemble_diff_line_outputs` (`edit.rs:377`) builds **one output row per `DiffLine`**: gutter spans + content spans, with `background: Some(theme.diff_delete_bg | diff_insert_bg)` (`edit.rs:389-392`), `content_start_col` from `compute_bg_start` (`edit.rs:470`). Soft wrap via `wrap_text` (`edit.rs:483`) + `project_styles_onto_wrap_segments` (`edit.rs:519`, splits only at existing span or wrap edges, preserving whole `Style`s); continuation rows carry `joiner: Some(..)` (`edit.rs:464`).
- Syntax highlighting is **already FG-spans-over-BG-band**: `render_content_spans` (`edit.rs:688-735`) emits syntect FG spans (`syntect_to_ratatui_fg`, FG only, no BG) and the insert/delete background is painted by the row layer — exactly the "syntax highlighting despite overlaid added/removed background" delta shows. Exception: on bandless themes (`Theme::diff_uses_line_fg()`, theme `mod.rs:351-354`) changed lines paint a whole-line red/green FG instead.
- Progressive highlight: `EditHighlightPhase` (`edit.rs:57-69`) — per-hunk syntect first paint, background worker upgrade to full-file-scoped styles (`compute_file_scoped_styles` `edit.rs:279`, caps `EDIT_HL_MAX_BYTES`=2 MiB / `EDIT_HL_MAX_LINES`=50k at `edit.rs:44-49`), with theme-kind guard (`edit.rs:1013-1023`).
- `EditHighlightPhase`/highlighter design keeps old and new side state separate (`edit.rs:196-199`) — directly reusable per split pane.

### 1.3 Config plumbing — `pager.toml` `[scrollback.blocks.edit]`

Documented in `crates/codegen/xai-grok-pager/docs/user-guide/06-theming.md:241-253` (§"Block Styling: Edit Diffs"; the user-guide lives in the pager crate and is embedded via `include_str!` from `crates/codegen/xai-grok-pager/src/docs.rs:74`). Keys: `indent`, `vpad`, `bg`, `hunk_separator`, `dual_line_numbers`, `expanded_by_default`, `line_summary`.

Chain: `RawEditBlockConfig` (`crates/codegen/xai-grok-pager-render/src/appearance/config.rs:967-995`, serde `Documented`, `#[serde(default)]`) → `EditBlockConfig` (`config.rs:371-390`, defaults `:392`) via raw→cfg at `config.rs:1410-1411` → consumed in `EditToolCallBlock::rendered_output` (`edit.rs:1215`, `let edit_cfg = &ctx.appearance.scrollback.blocks.edit;` at `edit.rs:1217`) → converted to `DiffRenderConfig` at `edit.rs:1260-1266` → `render_diff_lines` (`edit.rs:1326`, `self.render_diff_lines(&theme, ctx.width, &config)`).

### 1.4 Block viewer — `crates/codegen/xai-grok-pager/src/views/block_viewer/mod.rs`

`BlockViewer::for_edit` (`mod.rs:838`) → `build_edit_items` (`mod.rs:896-937`): calls `edit.render_diff_lines(theme, 500, &DiffRenderConfig::default())` (`mod.rs:900-905`) at a fixed wide width (NoWrap assumption noted at `mod.rs:914-916`), flattens to `ContentLine`s with per-line `DiffLineMeta { tag, text, lo, ln }` and per-line `bg`. Note: the viewer **ignores the user's edit-block config** (hardcoded `DiffRenderConfig::default()`) — a side-by-side toggle would also need this call site to receive the appearance config.

### 1.5 Theme / quantization context

`Theme::current()` builds the concrete theme and quantizes every color to the terminal's color level (`Theme::quantized`, `crates/codegen/xai-grok-pager-render/src/theme/mod.rs:230-266`, incl. `diff_delete_bg`/`diff_insert_bg` at `:264-266`; ANSI-16 chrome overrides for Basic/legacy-ConHost at `:334-339`). Syntax FGs quantize per token (`crates/codegen/xai-grok-pager-render/src/syntax.rs:23-50`) with a polarity-safe ANSI-16 mapping under the terminal-native lock (`syntax.rs:56+`). Any split renderer inheriting `Span`s from these pipelines keeps all of this for free.

### 1.6 Side-by-side presence: absent, with evidence

- `rg -i 'side[_-]by[_-]side'` over `crates/codegen/` yields 3 hits, all unrelated to diffs: welcome hero box (`views/welcome/mod.rs:317`, `views/welcome/hero_box.rs:13`) and a legend comment (`scrollback/blocks/context_info.rs:248`).
- `dual_line_numbers` (`edit.rs:580-600`, `render_gutter` `edit.rs:620-645`) renders **two line-number columns on one content column** — Delete rows show only `lo`, Insert rows only `ln`, Equal rows both. There is exactly one content span stream per row (`assemble_diff_line_outputs`).
- No config key in `RawEditBlockConfig`/`EditBlockConfig` (`config.rs:371-395`, `:967-1011`) and no field in `DiffRenderConfig` (`edit.rs:73-88`) selects a two-pane layout.

## 2. Activation sketch for side-by-side (if built in-house)

The whole toggle rides existing plumbing; the new work is one row-assembly branch.

**Config (mechanical):**
1. `RawEditBlockConfig` + `EditBlockConfig`: `side_by_side: bool` (default `false`), doc comment, `Default` impls (`config.rs:371-410`, `:967-1011`).
2. Docs row in `06-theming.md` §Block Styling: Edit Diffs (`:241-253`).
3. `DiffRenderConfig` field `side_by_side: bool` (`edit.rs:73-99`); map it at the existing conversion (`edit.rs:1260-1266`).

**Render path (the actual work):**
1. **Pairing** — walk each `DiffHunk` between Equal rows; group maximal Delete-run/Insert-run pairs (`ChangeTag` already on every `DiffLine`). Left pane rows: Delete lines + blank filler for unmatched Inserts; right pane: Insert lines + blank filler for unmatched Deletes; Equal rows appear in both panes. `DiffLine.lo/ln` feed the existing dual gutter per pane unchanged.
2. **New assembler** — `assemble_split_row(left, right, layout, theme, config)` next to `assemble_diff_line_outputs` (`edit.rs:377`): per-pane gutter (`gutter_layout` `edit.rs:569` splits naturally into per-pane layouts), a divider column, then left/right content spans. Per-pane background still `theme.diff_delete_bg`/`diff_insert_bg`; `compute_bg_start` (`edit.rs:470`) applies per pane.
3. **Width split** — `content_width = width - layout.total` (`edit.rs:194`) becomes two panes: `left_w = (content_width - 1) / 2` etc., with a minimum-pane threshold (e.g. effective pane < ~30 cols → fall back to unified) so narrow terminals keep today's rendering.
4. **Soft wrap** — reuse `wrap_text` + `project_styles_onto_wrap_segments` (`edit.rs:483`, `:519`) per pane at pane width; pad the shorter pane's wrapped rows with blanks so paired rows stay height-aligned (`joiner` continuation semantics from `edit.rs:464` carry over per pane). This is the trickiest alignment work; delta's own wrapping behavior (`side-by-side-view` manual) is the reference.
5. **Highlighter reuse** — the existing two-highlighter scheme (old/new side, `edit.rs:196-199`) maps 1:1 onto left/right panes; `EditHighlightPhase::FileScoped` upgrade (`render_diff_lines` `edit.rs:1005-1026`) is pane-agnostic since it keys on `ln`.
6. **Selection/copy** — `BlockLine` selection ranges, `gutter_span_count` (`edit.rs:113-118`), and `SelectionBoundary` header entries (`edit.rs:1288-1303`) need per-pane variants; copy semantics stay on `diff_hunks_to_patch` (`EditToolCallBlock::copy_text`, `edit.rs:884-888`), which is pane-independent.
7. **Block viewer** — `build_edit_items` (`views/block_viewer/mod.rs:896`) must stop hardcoding `DiffRenderConfig::default()` (`:900`) and take the appearance config, and its 1:1 meta mapping (`:910-917`) must model two panes per visual row (or keep the viewer unified initially and gate split to scrollback).
8. **Theme edge** — bandless themes (`diff_uses_line_fg()`, theme `mod.rs:351`) keep the whole-line-FG branch per pane; quantization needs no change (colors, not spans).

## 3. Crate prerequisites

| Crate | Status | Verdict |
|---|---|---|
| `similar` 2.7.0 | in workspace deps (root `Cargo.toml:274`, locked 2.7.0) — already powers `build_diff_hunks`. Default features (`text`); the `inline` feature (iterating word-level changes with emphasis ranges; cf. `terminal-inline` example in upstream `Cargo.toml`) is **not** enabled — adding `features = ["inline"]` unlocks word-level intra-line highlights in-house. | **usable, already in tree** |
| `syntect` 5.3 + `two-face` 0.4 | root `Cargo.toml:281`, `:311` (`default-onig` / `syntect-onig`). Provides syntax set + curated theme defaults; already wired via `crate::syntax::get_syntect` and the FG-only span pipeline. | **usable, already in tree** |
| `anstyle` 1.0 / `anstyle-parse` 0.2 / `anstyle-lossy` 1.1.4 / `anstyle-syntect` 1.0.4 | root `Cargo.toml:130-133`. `anstyle_parse::{Parser, Perform}` already drives ANSI stream splitting in `crates/codegen/xai-ratatui-inline/src/segment.rs:3,37` (that `Perform` discards SGR; an SGR-collecting `Perform` producing ratatui `Style`s is a small, pattern-known extension). | **usable, already in tree** — the parse-back half of a delta subprocess route |
| `git-delta` 0.20.1 (crates.io) | upstream `Cargo.toml` (fetched from `raw.githubusercontent.com/dandavison/delta/0.20.1/Cargo.toml`) declares only `[[bin]] name = "delta" path = "src/main.rs"` — **no `[lib]` target**. Importing as a library is impossible without vendoring `src/` and restructuring binary-crate internals (deps: `bat` 0.26, `syntect` 5, `git2`, `palette`, `sysinfo`, …). | **not usable as a dependency**; subprocess or vendoring only |
| `bat` 0.26 | exists as a real library crate (delta itself consumes it for syntax/themes). Grok already owns an equivalent stack (`syntect` + `two-face` + own tmThemes per `syntax.rs:8-22`); pulling `bat` in would duplicate the syntax engine for theme-ecosystem access only. | **not needed** |

## 4. Delta rendering: what it is, what grok has, feasibility

### 4.1 What delta's mechanics are (grounded in upstream sources)

From the README at `raw.githubusercontent.com/dandavison/delta/0.20.1/README.md` (Features section, verbatim excerpts):

- "Language syntax highlighting with the same syntax-highlighting themes as bat"
- "Word-level diff highlighting using a Levenshtein edit inference algorithm"
- "Side-by-side view with line-wrapping"
- "Handles traditional unified diff output in addition to git output" (and delta reads the diff from stdin — `git config core.pager delta`).

Side-by-side section: "By default, side-by-side view has line-numbers activated, and has syntax highlighting in both the left and right panels" and "Side-by-side view wraps long lines automatically". Manual: `https://dandavison.github.io/delta/side-by-side-view.html`.

### 4.2 What grok already has vs. what delta would add

| Capability | Grok today | Delta |
|---|---|---|
| Syntax highlight FG over insert/delete BG | yes (`render_content_spans` `edit.rs:688-735` + row-layer BG) | yes (bat-based) |
| Progressive/full-file highlight with caps | yes (`EditHighlightPhase`, `edit.rs:44-69,279`) | no equivalent concept |
| Layout | unified rows only | split panes with wrap alignment |
| Word-level intra-line highlight | no | yes (Levenshtein inference; second bg color, see probe) |
| Theme source | grok `Theme`, terminal-level quantization, ANSI-16/polarity-safe fallbacks (`theme/mod.rs:230-266,334-339`; `syntax.rs:43-60`) | bat tmTheme themes + own gitconfig styles; emits truecolor SGR independent of host terminal level |
| Line numbers | single or dual column, unified (`edit.rs:569-668`) | left/right per pane |

### 4.3 Local probe (`delta` 0.20.1 at `/usr/bin/delta`; full capture in the recon scratch log)

Piping a small Rust unified patch through `delta --side-by-side` shows the mechanics concretely: two panes separated by box-drawing columns; both panes' code lines carry syntect-style truecolor FG (`ESC[38;2;86;156;214m fn` …) over pane-wide insert/delete backgrounds (`ESC[48;2;45;21;22m` delete / `ESC[48;2;18;39;26m` insert); word-level highlights render as bold + a shifted background on just the changed token (`ESC[1;48;2;90;32;32m x`); unmatched old lines pair against blank pane rows. Two integration-relevant findings from the same probe: (a) `delta` picks up the invoking user's global gitconfig `[delta]` settings — a plain `delta < patch.txt` here rendered side-by-side because the user's gitconfig enables it, so grok would need `--no-gitconfig`/explicit flags to be deterministic; (b) delta prepends its own decorations (`[~] a/test.rs ↦ b/test.rs`, `▏ b/test.rs:1:`) that grok would have to suppress or strip.

### 4.4 Subprocess-route integration shape (if chosen)

`EditToolCallBlock::copy_text`'s generator `diff_hunks_to_patch` (`xai-grok-pager-diff/src/lib.rs:385`) already produces the stdin payload. Pipeline: spawn `delta --color=always --width <w> --side-by-side --line-numbers … < flags pinning theme/config`, capture ANSI, re-parse into `Line`/`Span`s with an SGR-collecting `anstyle_parse::Perform` (in-tree pattern: `xai-ratatui-inline/src/segment.rs`), cache per (hunks-version, width, theme) and refresh off the render thread (mirroring the `EditHighlightPhase` worker idea). Hard parts: external-binary dependency in the shipped TUI (PATH/version drift, packaging), async caching, rebuilding selection/copy/OSC8 semantics from reparsed spans, and color governance — delta's truecolor output bypasses grok's quantization and terminal-native/polarity-safe guarantees, and its themes don't track grok's `ThemeKind`.

### 4.5 Recommendation

**Build side-by-side in-house on the existing infrastructure.** The data model (`DiffLine` with `lo`/`ln`/`tag`), the dual gutter, the per-side syntect highlighters, FG-span-over-band painting, wrap projection, progressive highlight, and config plumbing all exist; the missing piece is one split-row assembly branch (§2, effort class: medium — one new assembler + pane pairing + wrap alignment + viewer passthrough; the wrap alignment and selection semantics dominate). Add word-level highlights later via `similar`'s `inline` feature (one feature flag + an emphasis-to-style mapping) — no external dependency.

The delta **subprocess** route is viable but second-best: it delivers delta's mature layout instantly, yet costs an external binary, an ANSI re-parse layer, non-determinism from user gitconfig, and theme divergence from grok's quantized/terminal-native story (all evidenced in §4.3/§4.4). It fits best as an opt-in "render via delta" escape hatch, not the default. **Vendoring** delta is the worst option: binary-crate internals, large dep surface (`bat`, `git2`, `sysinfo`), upstream churn; no `[lib]` means nontrivial restructuring for zero capability the fork lacks.
