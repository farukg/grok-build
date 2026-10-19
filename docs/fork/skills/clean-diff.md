---
name: clean-diff
description: >
  Structure readable, single-purpose commits whose per-file diffs preserve line
  history and isolate semantic change. Activate only in /home/fg/projects/org/forks/
  and /home/fg/projects/ai-ag/ (git pull --rebase hygiene). Do NOT load for SIG-owned
  or other non-fork, non-AIAG trees, or for routine isolated edits.
generated_by: sig context compose-docs
---

# Skill: clean-diff

Produce diffs where a reviewer can understand **what** changed from the diff alone, and **why** from the commit message. If either requires digging through the codebase, the diff failed.

"Minimal lines changed" is a consequence of clean diffs, not the goal. The goal is **comprehensibility** — semantic clarity of every changed line.

**Scope (HARD):** load and apply this skill **only** in repositories under
`/home/fg/projects/org/forks/` and `/home/fg/projects/ai-ag/`. Those trees
must stay `git pull --rebase`-clean onto upstream or company base. Do **not**
load it for SIG-owned or other non-fork, non-AIAG trees (`sigma-crates`,
`zsh-config`, `sigma/`, `work-tools/`, …); those follow
§MOD.GIT.COMMIT-CHEAP / `always-commit-non-aiag-repos`.

---

## §CLEAN-DIFF-GUIDING-PRINCIPLE — Guiding Principle

Every commit tells a story with one plot point. A reviewer reads the commit message (the "why"), then the diff (the "what"), and both align without noise. Anything that breaks this — mixed concerns, unnecessary renames, formatting churn — is a violation.

---

## §CLEAN-DIFF-REVIEW-UNIT — The Review Unit: one commit, one file, against its parent

A reviewer reads each changed file **per commit** — the diff of that commit against its parent, in a split view. That single pane, not the cumulative branch diff, is what must be legible. Optimize every commit's per-file diff; perfect legibility *across* a chain of commits is neither enforceable nor the goal.

### The move-recognition rule (the core failure)

A diff tool pairs a removed line with an added line only when they stay similar enough to be recognized as the *same* line, edited or moved. Two outcomes:

- **Good — recognizable.** A line moves or changes slightly; the differ pairs old↔new and shows a tight, attributable hunk. Example: moving `localHolds.value--` up one block, or `cy:open:fast` → `cy:open`. The reader sees exactly what changed.
- **Bad — unrecognizable.** New logic is interleaved into an existing function, shifting and lightly rewording the surrounding lines. The differ gives up pairing and renders a **large red block deleted + a large, slightly-different green block re-added**. The real change is buried; the reader has to re-read the whole region to find it.

### The fix: land new code as one contiguous block

When you add non-trivial logic, place it so the differ sees a clean insertion, not interleaved churn:

| The new code is… | Place it as | Call-site diff becomes |
|---|---|---|
| Substantial, cohesive, reusable | **a new file** + import | 1 import + 1 call (tight split-change) |
| Cohesive but local | **a new function/helper**, appended at a natural boundary, + a call | a small, recognizable edit |
| Small, inline-only | inline — but as one contiguous group, never scattered | one green block |

Extraction has a second payoff: the big body leaves the touched file's diff entirely. It appears **once**, as a clean new file or function, instead of as interleaved noise inside an existing function.

### Judge sensibility — don't extract blindly

New-file vs new-function vs inline is a per-change judgment. Extract when it genuinely makes the call-site diff cleaner **and** the extracted unit is a sensible standalone concern. Do not manufacture indirection — a one-line helper used once, a file per trivial constant — just to shrink a diff; that trades diff noise for structural noise, which is worse.

> This per-commit diff quality is the *shape* of a change. Its companion — structuring the **sequence** of commits so they reorder, fold, and rebase cleanly — lives in the `rebase-able-commits` skill and is governed by `git.law.md §MOD.GIT.2`.

---

## §CLEAN-DIFF-TOUCH-ONLY — Touch only what you change (the no-disturbance rule)

A clean per-commit diff shows **exactly** the lines your change requires — nothing more. Every extra changed line is noise the reviewer must read and rule out. The move-recognition rule above is about where to *land new code*; this rule is about *not disturbing what's already there*.

### Never reorder what already exists

Pre-existing fields, object keys, config options, imports, match arms, function/method order — leave them where they are. Reordering turns untouched lines into delete+add pairs that bury the real change and manufacture false merge conflicts. Add new entries at a natural boundary (end of the group, or beside the most related existing entry **without moving the others**); never relocate existing ones to "tidy up" while you're in there. Tidying is a separate prep commit, never a rider on a feature change.

### Leave near-identical lines byte-identical

If a line was already there and your change doesn't *require* touching it, **do not rewrite it** — not for style, not for a marginally nicer phrasing. A line that stays byte-for-byte identical drops out of the diff entirely. A line you reworded to ~95% the same renders as delete+add and competes for the reviewer's attention with the line that actually changed. "Was schon vorher da war und fast 1:1 gleich ist, bleibt."

Corollary: when you change **one** item in a block, the neighbours you don't change stay exactly as they were — *including their inline comments*. Example: bumping `pageLoadTimeout` 30s→60s changes only that line; the sibling `responseTimeout`/`execTimeout` lines and their `// …` comments stay untouched and disappear from the diff.

### Don't rewrite a statement when only its body changes

A frequent self-inflicted wound — folding an unchanged leading statement into the change:

```js
// pre-existing — only the body needs to change
const portBusy = await isPortInUse(port);
if (portBusy) {
  abort();
}
```

Wrong — inlining the await rewrites two lines that didn't need to:

```js
if (await isPortInUse(port)) {   // delete+add, pure noise
  reclaimOrAbort();
}
```

Right — keep the leading lines as context, edit only the body:

```js
const portBusy = await isPortInUse(port);
if (portBusy) {
  reclaimOrAbort();              // the only line in the diff
}
```

Same for splitting/merging declarations, swapping equivalent constructs (`if/else`↔ternary, `.forEach`↔`for`), and re-wrapping lines: if what's there already works and you don't need to change it, don't.

---

## §CLEAN-DIFF-COMMENT-DISCIPLINE — Comment discipline

Comments are part of the diff and earn their place the same way every line does: by explaining non-obvious **why**, never by narrating self-explanatory **what**.

- **Keep comments exceptional.** Add one only for non-obvious why, an external constraint, safety proof, lifecycle, or interop contract that code cannot carry. Rename or restructure before commenting. `// increment counter` over `counter++`, a paragraph repeating a well-named function, headings, and narration are noise; delete them.
- **Preserve genuine rationale.** Remove a comment only when its information is now carried by code or the comment is noise. If it carries a durable external rationale, retain it or move that rationale to its current-truth owner; do not use a commit message as the only owner.
- **Preserve compliant pre-existing comments.** Leave a compliant comment byte-identical when only passing through. A discovered comment violation follows `§MOD.QUALITY.AUTHORITY` progressive healing and may be removed or restructured; do not preserve it merely because it predates the change.
- **Don't double-comment.** If a line already carries a sufficient inline comment, don't also add a `/* */` block above it saying the same thing. Pick one.
- **Match the surrounding comment style** (line vs block, and language) instead of introducing your own — a style switch is diff noise. Follow the repo's convention for which natural language comments use.

---

## §CLEAN-DIFF-PREP-COMMITS — 1. Prep Commits (Vorbereitende Commits)

Separate mechanical changes from semantic changes. Each prep commit is reviewable in isolation and creates a clean baseline for the real work.

### When to Use Prep Commits

| Situation | Prep commit | Then |
|-----------|-------------|------|
| No formatter exists | `[check-green] [setup] add black + isort configuration` | Feature work |
| Inconsistent formatting | `[check-green] [format] apply black to existing codebase` | Feature work |
| Variable needs renaming | `[check-green] [rename] userList → users across module` | Behavior change |
| Imports are messy | `[check-green] [imports] reorganize imports with isort` | New imports for feature |
| Dead code blocks feature | `[check-green] [cleanup] remove unused helpers in utils.py` | New helper implementation |
| Type annotations missing | `[check-green] [types] add type hints to service layer` | Logic changes |

### Rules for Prep Commits

- Prep commits change **zero behavior** — tests pass before and after identically
- Prep commits are **tool-verifiable** — `git diff --stat` shows only the expected file types
- Register format-only prep commits in `.git-blame-ignore-revs` so `git blame` stays useful
- Never combine a prep commit with the feature commit that depends on it

---

## §CLEAN-DIFF-COMMIT-ANATOMY — 2. Commit Anatomy (Atomic Commits)

One commit = one logical change, explainable in one sentence without "and."

### Commit Type Separation

These concerns belong in **separate commits**, never mixed:

The build-standing marker (`[unbuilt]`, `[check-green]`, `[tests-green]`) is the first token of the subject and precedes the type tag per §MOD.GIT.COMMIT-CHEAP.

| Type | Example message |
|------|----------------|
| Format | `[check-green] [format] apply black to recipe module` |
| Rename | `[check-green] [rename] calculate_score → compute_balance_score` |
| Refactor | `[check-green] [refactor] extract ingredient validation to own method` |
| Feature | `[check-green] [add] hobby recipe analysis endpoint` |
| Fix | `[check-green] [fix] prevent division by zero in nutrient calculation` |
| Test | `[tests-green] [test] add edge cases for empty ingredient list` |
| Config | `[check-green] [config] add CORS allowed origins for staging` |
| Deps | `[check-green] [deps] add strawberry-graphql 0.291` |
| Migration | `[check-green] [migrate] add analysis_cache column to Recipe` |
| Cleanup | `[check-green] [cleanup] remove deprecated export endpoint` |

### Commit Ordering in a Feature Branch

Structure commits as a reviewable narrative:

```
1. [setup/format/rename] — prep commits (mechanical, zero behavior change)
2. [refactor]             — structural changes (extract, move, reorganize)
3. [add/fix]              — the actual feature or fix
4. [test]                 — tests for the new behavior
5. [config/deps]          — supporting infrastructure
```

---

## §CLEAN-DIFF-DIFF-MINIMIZATION — Diff Minimization Techniques

### Universal (All Languages)

| Technique | Why |
|-----------|-----|
| **Trailing commas** — enforce them | Adding a field = 1 line added, not 2 (add + modify previous line's comma) |
| **Add fields at end** of objects/dicts/records | Existing lines untouched |
| **Add methods at bottom** of class/module | Existing line numbers stable |
| **Add imports at end** of their group | No reshuffling existing imports |
| **New complete blocks** over scattered small edits | One contiguous green block > 15 scattered +/- lines |
| **Keep variable names** unless semantic reason to rename | Rename = every usage line changes = review noise |
| **Guard clauses at top** of function | Prepending doesn't disturb existing logic lines |
| **Default parameters at end** of signature | Existing call sites unchanged |
| **Prefer explicit over clever** | 3 clear lines > 1 cryptic line hiding a behavior change |
| **Extract before modifying** | "New file added" is easier to review than "large function modified" |

### When No Trailing Commas

If the codebase doesn't enforce trailing commas and you can't add them (e.g. JSON):
- Add new items as **second-to-last** — only the new line shows in diff, last item's comma is untouched
- Or: introduce trailing commas as a prep commit first

### Large Additions

When adding significant new code (new class, new endpoint, new module):
- Prefer **new files** over cramming into existing files
- If it must go in an existing file, add as a **contiguous block** at a natural boundary (end of class, after last function)
- Never scatter new logic across 10+ locations in one file — extract to a helper first

### Worked example (abstract)

A config file grew by several inlined helper bodies plus genuine value changes. The per-file diff became one giant red-then-green block — move-recognition failed.

Fix: extract each helper into its own small module under a scripts/lib area; the config keeps only imports, wirings, and real value changes. Net behavior identical; every remaining line is attributable.

A follow-up pass restored original field order, left unchanged sibling lines (and their comments) byte-identical, and kept leading context statements instead of inlining them (§CLEAN-DIFF-TOUCH-ONLY) — so only fields whose values changed remained in the diff.

---

## §CLEAN-DIFF-LANGUAGE-SPECIFIC — 4. Language-Specific Rules

### Python / Django

- **Formatter**: `black` (deterministic, trailing commas, no config debates)
- **Import sorter**: `isort` with `profile = "black"`
- **New function args**: add with defaults at end → `def foo(a, b, new_arg=None):`
- **Dict/list literals**: trailing commas always → black enforces this
- **New methods**: add at bottom of class, before private methods (if convention exists)
- **Type hints**: add in dedicated prep commit, not mixed with logic
- **Django migrations**: one logical schema change per migration file; descriptive names (`0042_add_recipe_analysis_cache.py` not `0042_auto.py`)
- **Django settings**: new settings at end of their section with comment

### JavaScript / TypeScript / Vue

- **Formatter**: Prettier with `trailingComma: "all"`
- **Linter**: ESLint with `eslint-config-prettier` (no format conflicts)
- **Destructuring with defaults**: `function foo({ a, b, c = default }) {}` — new optional params don't touch call sites
- **JSX props**: new props on own line at bottom of prop list
- **React/Vue hooks & helpers**: extract to a separate file/composable before using in the component, so the component diff is a small wiring change, not a new body
- **Vue SFC**: keep `<script>`/`<template>`/`<style>` concerns in separate commits where possible; a logic change shouldn't ride along with template re-indentation
- **Config / plugin bodies**: move non-trivial middleware/task/setup bodies into an imported module; the config file should read as imports + wiring (see the worked example in §CLEAN-DIFF-DIFF-MINIMIZATION)
- **Object spread**: `{ ...obj, newProp: value }` shows the exact addition

### ReScript

- **Punned records**: `{ foo, bar }` not `{ foo: foo, bar: bar }` — one line per new field
- **Pipe composition**: adding `|> newTransform` is a clean single-line addition
- **New match branches**: add before the wildcard/catch-all `| _ =>`
- **Module exports**: new functions at bottom of module

### SQL / Migrations

- One logical operation per migration file
- Use `IF NOT EXISTS` / `IF EXISTS` for idempotency
- Add columns with `DEFAULT NULL` first, then `NOT NULL` constraint in follow-up migration
- Index creation in separate migration (locking concern)
- Always include both up and down operations

### YAML / Ansible

- New tasks at bottom of playbooks
- Block style (`|`) for multi-line strings, never inline `\n`
- New keys alphabetically or at end of section
- YAML anchors introduced in separate prep commit
- Jinja2 macros: extract to separate template files

---

## §CLEAN-DIFF-FORMATTER-FIRST — 5. Formatter-First Doctrine

When starting work on a project **without consistent formatting**:

### Step 1: Introduce formatter (before any feature work)

```
Commit 1: [check-green] [setup] add black + isort to pyproject.toml
Commit 2: [check-green] [format] apply black + isort to entire codebase
Commit 3: [check-green] [setup] add .git-blame-ignore-revs with format commit hash
Commit 4: [check-green] [setup] add pre-commit hook for black + isort
```

### Step 2: Choose formatters that support clean diffs

| Concern | Good choice | Why |
|---------|------------|-----|
| Trailing commas | black, Prettier | Prevents comma-thrashing |
| Deterministic output | black, Prettier | Same input = same output, no phantom diffs |
| Import sorting | isort, eslint-plugin-import | Prevents merge conflicts in import sections |
| Idempotent | All good formatters | Re-running produces zero diff |

### Step 3: Maintain `.git-blame-ignore-revs`

```bash
# .git-blame-ignore-revs
# [check-green] [format] apply black to entire codebase (2026-02-16)
abc123def456789...
```

Configure globally: `git config blame.ignoreRevsFile .git-blame-ignore-revs`

---

## §CLEAN-DIFF-LARGE-REFACTORS — 6. Large Refactors

Large changes are the hardest to keep reviewable. Strategy: **decompose into a sequence of small, individually reviewable commits**.

### Patterns

**Extract → Migrate → Delete**
```
1. [refactor] extract UserService from monolithic views.py    (new file, copy)
2. [refactor] update imports to use UserService               (mechanical)
3. [cleanup] remove duplicated code from views.py             (deletion)
```

**Skeleton → Implement → Wire**
```
1. [add] add RecipeAnalysis model + empty service             (structure only)
2. [add] implement analysis calculation logic                  (fill in body)
3. [add] wire analysis endpoint into GraphQL schema            (integration)
```

**Facade → Migrate → Remove**
```
1. [refactor] add new API alongside old                       (new code, old untouched)
2. [refactor] migrate callers from old to new API             (mechanical substitution)
3. [cleanup] remove old API                                    (pure deletion)
```

### Rules for Large Refactors

- Commit each coherent intermediate step without a build gate. Start its subject with `[unbuilt]`, `[check-green]`, or `[tests-green]` to record the standing.
- Each commit should be ≤ 400 changed lines (ideally ≤ 200)
- If a commit exceeds this, it probably does two things — split it
- Use `git rebase -i --autosquash` to clean up history before review

---

## §CLEAN-DIFF-ANTI-PATTERNS — 7. Anti-Patterns

| Anti-Pattern | Why It Fails |
|--------------|-------------|
| Format + logic in same commit | Reviewer can't distinguish noise from signal |
| Rename + behavior change | Every usage line changes; behavior change is invisible |
| Auto-format entire file on save | Diff shows "whole file changed" when only 3 lines matter |
| Import reshuffling without reason | Noise; triggers merge conflicts for others |
| Clever one-liner replacing 5 clear lines | Hides the actual change behind cleverness |
| >400 lines changed without split | Invites rubber-stamp reviews |
| "WIP" / "asdf" commit messages | Pollutes `git log`, breaks `git bisect` |
| Scattered edits across 15 locations | No coherent story; extract to a block instead |
| New logic interleaved into an existing function body | Differ can't pair lines → renders a big delete + slightly-different re-add; the real change is buried. Extract to a new file/function so the call site is a clean insertion |
| Reordering pre-existing fields / keys / options / imports | Turns untouched lines into delete+add; buries the real change and manufactures merge conflicts. Add at a boundary; never relocate existing entries |
| Rewording a line to ~95% identical | delete+add competes with the line that actually changed; keep it byte-identical so it drops out of the diff |
| Inlining/re-wrapping an unchanged statement to fold in a body change | Two noise lines for a one-line change; keep the leading statement (e.g. `const x = await f()`) as context |
| Comment restating self-explanatory code | Pure noise; delete it. Retain genuine external rationale in code or move it to its current-truth owner, never a commit message alone. |
| Stripping / restyling / re-translating a pre-existing comment in passing | Adds diff and drops context you didn't author; leave it alone |
| Deleting + creating + modifying in one commit | Three different review tasks forced into one |
| Changing indentation AND code simultaneously | Indentation diff buries the real change |

---

## §CLEAN-DIFF-AGENT-PROTOCOL — 8. Agent Protocol

When working in a CleanDiff-scoped project (`/home/fg/projects/org/forks/` or `/home/fg/projects/ai-ag/`), the agent must:

1. **Check for formatter** — if none exists, propose introducing one as first action (prep commit)
2. **Separate concerns** — never mix format/rename/refactor/feature in one commit
3. **Plan commit sequence** — before writing code, outline the commit chain
4. **Minimize touched lines** — use trailing commas, append-at-end, block additions
5. **Self-review the diff** — before committing, mentally verify: "Can a reviewer understand this diff without context?"
6. **Prefer new blocks** — when adding logic, prefer a new function/method/file over scattering edits
7. **Preserve names** — don't rename variables/functions unless there's a semantic reason documented in the commit message
8. **Use prep commits** — if the real change requires mechanical prep (format, rename, extract), do that first
9. **Touch only what you change** — never reorder existing fields/keys/imports; leave unchanged lines and compliant unchanged comments byte-identical; route a violating comment to item 10 and `§MOD.QUALITY.AUTHORITY`; don't rewrite a statement when only its body changes
10. **Comment discipline** — retain comments only for non-obvious why, external constraints, safety proofs, lifecycle, or interop contracts that code cannot carry; delete narration and keep durable rationale in code or its current-truth owner; don't double-comment

### Diff Size Guidelines

| Size | Action |
|------|--------|
| ≤ 50 lines | Fine as-is |
| 50–200 lines | Verify it's one concern |
| 200–400 lines | Consider splitting |
| > 400 lines | Must split unless it's a single new file or format-only |

---

## §CLEAN-DIFF-RELATED — Related

- **`rebase-able-commits` skill** — the companion: structuring the commit *sequence* so it reorders, folds (`--fixup`/`--autosquash`), and rebases cleanly. Clean per-commit diffs (this skill) + rebase-able sequence (that skill) are the two halves of curated history.
- **`git.law.md` §MOD.GIT.1/2** — the governing law for **forks/ and ai-ag/**: base-branch-first, curated thematic history ordered low→high impact so `git pull --rebase` onto upstream stays easy, `--force-with-lease`, backup-branch safety net. SIG-owned trees are out of this ceremony.
- **`git-worktree-local-rebase` skill** — mechanics for rebasing a worktree branch onto a local branch safely.
