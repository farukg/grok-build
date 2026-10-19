---
name: rebase-able-commits
description: >
  Reshape a local branch history with fixup/autosquash, non-tip amendments, and conflict-aware rebases so git pull --rebase onto upstream stays easy. Activate only in /home/fg/projects/org/forks/ and /home/fg/projects/ai-ag/. Do NOT load for SIG-owned trees or for rebasing one worktree onto another local worktree; use git-worktree-local-rebase.
generated_by: sig context compose-docs
---

# Rebase-able commits

Companion to the **`clean-diff`** skill. clean-diff governs the *shape* of a single commit's diff; this skill governs the *sequence* — making the whole branch safe to reorder, fold, and rebase onto its base. Both serve the same end: curated history that a reviewer reads top-to-bottom and that rebases onto base trivially (`git pull --rebase` against upstream or company base). The governing law is **`git.law.md` §MOD.GIT** (base-first, curated thematic history low→high impact, `--force-with-lease`, backups).

**Scope (HARD):** load and apply this skill **only** in repositories under
`/home/fg/projects/org/forks/` and `/home/fg/projects/ai-ag/`. Do **not**
reshape SIG-owned or other non-fork, non-AIAG history so a future
`git pull --rebase` stays pretty; those trees commit pragmatically
(§MOD.GIT.COMMIT-CHEAP).

This skill is for the common in-place edits *in that scope*: "amend a commit that isn't the tip", "fold this fix into the commit that owns it", "rework the foundation commit without re-touching everything above it".

## §RAC-CORE-ENABLER — The core enabler: disjoint file sets per commit

A non-tip commit can be amended and everything above it replayed **without conflicts** exactly when the commit you change touches files that the later commits do not. Before reworking a lower commit, verify the disjointness:

```bash
# Does anything ABOVE the target commit touch the files I'm about to change?
git log --no-ext-diff --oneline <target>..HEAD -- <file1> <file2> ...
# empty  → conflict-free replay; proceed
# nonempty → those commits will need conflict resolution, or fold differently
```

If a concern naturally spans files that later commits also edit, that is a signal to either (a) put the change in a commit at/above those, or (b) accept a small guided rebase. Designing each commit around a **bounded, mostly-disjoint file set** is what keeps a branch rebase-able over its lifetime.

## §RAC-RECIPE-FOLD — Recipe A — fold a working-tree change into an owner commit (fixup + autosquash)

The cleanest way to land "this belongs in commit X, not as a new commit on top". Used e.g. to fold a config-extraction cleanup back into the commit that introduced the config.

```bash
# 0. SAFETY NET first (always, before any history rewrite)
git tag prerebase-$(date +%Y%m%d-%H%M)        # or: git branch backup/<name>-prerebase HEAD

# 1. Make the edits in the working tree (they only touch files disjoint from
#    intervening commits — verify with the disjointness check above).

# 2. Commit them as a fixup pointing at the owner commit
git add -A
git commit --fixup=<owner-sha>                # message becomes "fixup! <owner subject>"

# 3. Autosquash-rebase onto base. Interactive -i is required for --autosquash to
#    take effect; a no-op sequence editor accepts the auto-generated todo (the
#    fixup is already positioned right after its owner).
GIT_SEQUENCE_EDITOR=true GIT_EDITOR=true git rebase -i --autosquash <base>
```

Verify: `git log --oneline <base>..HEAD | rg -i fixup` prints nothing, the owner commit's new SHA carries the folded change, and the commit count is unchanged (minus the squashed fixup).

Then check the owner commit **against its parent** (`git show --no-ext-diff <owner-new-sha> -- <files>`): a fixup folds *mechanically* but can still poison the owner's diff if the folded edit reordered fields or rewrote unchanged lines. The fold is only done when that diff stays clean per the clean-diff no-disturbance rule — not merely when the `fixup!` line disappears.

> Interactive `git rebase -i` with a real editor is unavailable in this harness. Drive it non-interactively: `GIT_SEQUENCE_EDITOR=true` (or a scripted editor) accepts the todo; `GIT_EDITOR=true` accepts any message prompt. A pure fixup replay needs no message edits.

## §RAC-RECIPE-B — Recipe B — edit a specific non-tip commit in place

When the change is larger than a fixup (new files, restructuring) and you want to build it *at* the commit's tree state:

```bash
git tag prerebase-$(date +%Y%m%d-%H%M)
GIT_SEQUENCE_EDITOR='sed -i "s/^pick <short-sha>/edit <short-sha>/"' git rebase -i <base>
# rebase stops at the target commit, working tree = that commit's state
#   … make edits, then:
git add -A && git commit --amend --no-edit
git rebase --continue                          # replays the rest (conflict-free if disjoint)
```

## §RAC-RECIPE-C — Recipe C — reorder / regroup before review or push

This is `git.law.md §MOD.GIT.2` in practice **for `/home/fg/projects/org/forks/` and `/home/fg/projects/ai-ag/`**: few thematic commits, ordered bottom = lowest-impact, top = highest-blast-radius, so dropping the top-N still leaves the bottom rebasing cleanly onto base (`git pull --rebase`). Do not run this regroup on SIG-owned trees.

```bash
git tag prerebase-$(date +%Y%m%d-%H%M)
# Either reorder via the todo list:
git rebase -i <base>                           # move/squash/reword lines
# Or rebuild thematically from scratch:
git reset --soft <base>                        # all changes staged, no commits
#   … then re-commit in thematic groups, low-impact first
```

## §RAC-KEEP-EDITORIAL — Keep the editorial layer rebase-proof

Anything keyed to a commit **SHA** (annotations, review notes, learning-material overlays, scripts that hard-code SHAs) orphans on every rebase, because rebase rewrites SHAs. Key such overlays to the commit **subject** instead (stable across rebase), falling back to SHA only where subjects collide and that repo isn't rebased. (Pattern proven in the cypress-35114 commit-map: `ann.commits[sha] || ann.commits["subj:" + subject]`.)

## §RAC-WORKTREE-CAVEAT — Worktree caveat

A branch checked out in another worktree cannot be moved with `git branch -f` ("checked out at …"). Move it from inside that worktree instead:

```bash
git -C <worktree-path> reset --keep <new-sha>
```

`reset --keep` refuses to overwrite local modifications. On a refusal, keep the modifications: commit them when they are your own work, then retry; when the worktree belongs to someone else, report and stop.

A rebase that rewrites SHAs in the main worktree does **not** auto-update a separate push worktree pointing at the old SHAs — re-point it explicitly when intended.

## §RAC-EXTERNAL-DIFF — External diff driver caveat

If `difft` (or any external diff) is the configured diff driver, `git diff` / `git apply` / `git show … | git apply` can mangle patch extraction (especially around binary/NUL-byte files). Use `git diff --no-ext-diff` for machine-readable patches, `git checkout <sha> -- <file>` to lift a file version, and the Edit tool for precise edits, rather than piping ext-diff output.

## §RAC-PRE-FLIGHT — Pre-flight checklist

- [ ] Backup tag/branch created **before** any rewrite.
- [ ] Disjointness verified for the commit being changed (`<target>..HEAD -- <files>` empty).
- [ ] Each resulting commit's per-file diff is clean (apply the **`clean-diff`** skill).
- [ ] Commit order is low→high impact; dropping the top-N still rebases onto base.
- [ ] Gate re-run after the rebase (project's typecheck/lint/tests — **not** `sig build` for non-SIG repos).
- [ ] Push only when policy allows; `--force-with-lease`, never `--force`. Never push a repo whose policy says the remote is the human's.
- [ ] Backup deleted only after `git diff <backup> <new-head>` shows no content difference.
