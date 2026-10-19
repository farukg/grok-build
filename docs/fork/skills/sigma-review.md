---
name: sigma-review
description: >
  Review or audit code in gitlab.myicecreamlab.com repositories against Sigma governance: mandatory reading set, discovery of task-specific mandatory reading, reading ledger, GitLab access, reports to Sigma and edits to the owning repo's default branch. Workstation runtimes gate on sig build; runtimes without it gate on the GitLab pipeline. Activate for reviews, audits, or review-driven fixes of Rust, ReScript, SpacetimeDB, boilerplate-rust, or codegen code.
generated_by: sig context compose-docs
---

# Sigma Review

Review and audit protocol for every agent runtime working on repositories
hosted on `gitlab.myicecreamlab.com`. It fixes three things exactly:

1. which Sigma files are mandatory reading, and how to discover the
   task-specific additional mandatory reading;
2. how the runtime reaches GitLab and which build gate it owns;
3. where results land: reports in Sigma on GitLab, edits in the owning GitLab
   repository on its default branch — never anywhere else.

A finding is valid only when it cites a rule the agent has actually read in
this session.

## §SIGMA-REVIEW-HARD-RULES — Hard Rules

1. No finding, verdict, or "looks good" before the reading ledger
   (§SIGMA-REVIEW-LEDGER) exists and covers every file from Steps 1–3.
2. "Read" means the full body, every line. Files longer than one read page are
   read in pages until the last line. Frontmatter summaries, `abstract:` /
   `overview:` blocks, and `*.composite.md` render manifests are indexes, not
   substitutes — read the `*.law.md` / `*.skill.md` / `*.prompt.md` body.
3. Memory of an earlier session is not reading. Re-read in every session.
4. Precedence when texts conflict: `200_policy.d/laws.d/**` > `gov.d/quality-core`
   > stack skills > product/fork knowledge > target-repo conventions. A repo
   convention never overrides a HARD law (§MOD.QUALITY.AUTHORITY).
5. Never cite a § id whose defining file is not in the ledger
   (§SIGMA-REVIEW-DISCOVERY 3f).
6. Use the report destination required by the current task contract and owner
   (§SIGMA-REVIEW-REPORT-DESTINATION). A contracted scratch report or local
   working evidence is valid; neither implies publication.
7. Publication is separate from review completion and requires direct current
   authorization under workspace policy. Publication from `ai-ag/` is
   forbidden. Never automatically switch branches, rebase, or push. Follow the
   contract's branch/verification requirements for authorized work.

## §SIGMA-REVIEW-EXECUTION — Execution Model

Pick the gate from the runtime, then stay on it.

**Workstation** — the agent can run `sig build` and register with `sig-orch`
(local Claude Code, Grok, Codex, Kimi, OpenCode, headless workers):

- Builds, checks, tests, and clippy go through `sig build`
  (§AGT.BEH.11, `§RUST-CARGO-DEPENDENCIES`, `§SIG-RUST-RESCRIPT-CARGO-SINGLETON`).
  No raw `cargo`, `cargo-quick`, or `cargo-localbuild`.
- Register before coordination (`sig-orch agent register`) and wait on the
  direct channel. A `## Build Result` is the completion event.
- The workstation checkout under `/home/fg/projects/...` is the working tree
  (§SIGMA-REVIEW-OBTAIN-SIGMA). A stack skill's `sig build ... cargo clippy`
  or `cargo fmt --check` is satisfied by that green `sig build`.
- Record the `request_id` when required by the task contract. Build success
  verifies the requested build only; it does not authorize publication.

**Pipeline** — the agent has no workstation `sig build` (a cloud session, or
any runtime whose checkout is not the workstation):

- Do not build or test locally. No `cargo build/check/test/clippy`, no
  ReScript or pnpm builds, no `sig` binary builds. Follow the task contract;
  do not infer permission to publish from pipeline ownership.
- When a task explicitly requires an authorized publication pipeline, report
  its status for that revision. Otherwise report local verification and any
  external gate that remains unverified.
- A stack skill's `sig build` or `cargo fmt --check` is satisfied when the
  GitLab job that runs that check is green. If no job runs it, say so in the
  report and do not run it locally.

Rules about the **code under review** (lint denials, formatter config, CI
contracts, codegen regeneration order) stay review criteria on both gates.

## §SIGMA-REVIEW-GITLAB-ACCESS — GitLab Access

Re-verify with the probe commands below when something fails; do not guess.

### Git

- Clone/fetch/push with the SSH-form URL
  `ssh://gitlab@gitlab.myicecreamlab.com:2222/<path_with_namespace>.git`.
  The session's git config rewrites it (`url.*.insteadOf`) to
  `https://gitlab.myicecreamlab.com/...`; the egress proxy injects the GitLab
  credential (user `faruk`, Owner access). No token, SSH key, or credential
  helper is needed — never add one, never print one.
- The remote therefore shows as `https://gitlab.myicecreamlab.com/<path>.git`.
  Before any push, `git remote get-url origin` must be on
  `gitlab.myicecreamlab.com`; otherwise stop.
- Shallow clones (`--depth 1`) suffice for reading; for edits a shallow clone
  is fine too — push works from it.
- Commits are signed automatically by the session (`commit.gpgsign`); do not
  change git identity or signing config.

### GitLab REST API

`glab` is the tool (§MOD.18 / §MOD.REUSE.0). Use `glab api` when `glab` is
installed. The `curl` form below is the same API for a session where `glab`
is absent and the egress proxy injects the credential: base
`https://gitlab.myicecreamlab.com/api/v4`, plain `curl -sS`, no auth header.
Never add or print a token.

- **Use numeric project ids.** Path-encoded ids (`projects/org%2Fgovernance%2Fsigma`)
  return 404/403 through the proxy. Resolve the id first:
  ```bash
  A=https://gitlab.myicecreamlab.com/api/v4
  curl -sS "$A/projects?search=<slug>&simple=true&per_page=100" \
    | python3 -c "import json,sys;[print(p['id'],p['path_with_namespace'],p['default_branch']) for p in json.load(sys.stdin)]"
  ```
  Known ids: `23` org/governance/sigma (default `main`), `28`
  org/platform/sigma-crates (default `main`). Re-resolve for any other repo.
- **Default branch** = `default_branch` from that response (or
  `curl -sS "$A/projects/<id>"`). Never assume `main`: e.g.
  org/product/verify-receipts uses `prod`, org/ops/devops uses `ansible`.
- **Identity check:** `curl -sS "$A/user"` → username `faruk`.
- **Pipeline for a pushed commit:**
  ```bash
  curl -sS "$A/projects/<id>/pipelines?sha=<full-sha>"            # id, status
  curl -sS "$A/projects/<id>/pipelines/<pipeline-id>/jobs?per_page=100"   # name, stage, status
  curl -sS "$A/projects/<id>/jobs/<job-id>/trace" | tail -200     # failing log
  curl -sS "$A/projects/<id>/repository/commits/<sha>"            # last_pipeline.status
  ```
  Statuses: `created`/`pending`/`running` → not finished; `success`;
  `failed`/`canceled` → act. A push that matches no workflow rule creates no
  pipeline — then the pipeline gate is "no pipeline configured for this
  change", stated as such in the report.
- **Waiting:** pipelines take minutes. Do not poll in a tight loop. Schedule
  the runtime's completion event (a channel wake, a scheduled check ≥ 5 min)
  and end the turn; on wake read the pipeline once and act.

### Probe (run when access behaves unexpectedly)

```bash
git ls-remote ssh://gitlab@gitlab.myicecreamlab.com:2222/org/governance/sigma.git HEAD
curl -sS -o /dev/null -w '%{http_code}\n' https://gitlab.myicecreamlab.com/api/v4/user
```

`200` plus a SHA means access works; anything else is an environment problem —
report it with the exact output, do not work around it with another host.

## §SIGMA-REVIEW-OBTAIN-SIGMA — Step 0 — Obtain Sigma

`$SIGMA` is the Sigma checkout:

1. `/home/fg/projects/org/governance/sigma` if it exists (any runtime on the
   workstation);
2. otherwise clone into the session scratchpad:
   ```bash
   git clone --depth 1 ssh://gitlab@gitlab.myicecreamlab.com:2222/org/governance/sigma.git "$SCRATCH/sigma"
   ```

Record `git -C "$SIGMA" rev-parse HEAD` in the ledger. `CM` = `$SIGMA/context-modules.d`.

**Path mapping.** Sigma cites absolute paths such as
`/home/fg/projects/org/governance/sigma/context-modules.d/...`: strip
`/home/fg/projects/org/governance/sigma/` and resolve against `$SIGMA`. A cited
path in another repository (`/home/fg/projects/<path_with_namespace>/...`) is
cloned from `ssh://gitlab@gitlab.myicecreamlab.com:2222/<path_with_namespace>.git`
when it is mandatory reading. The target repository of the review is cloned
the same way.

## §SIGMA-REVIEW-CORE-READING — Step 1 — Core Mandatory Reading

| # | File | Why |
|---|---|---|
| C1 | `CM/300_library.d/prompts.d/gov.d/quality-core.gov.prompt.md` | Always-on Quality Core / §MOD surface, §MOD.17/§MOD.18 |
| C2 | `CM/200_policy.d/laws.d/modeling.d/authoring.law.md` | §MOD.0–§MOD.16 incl. §MOD.10 delivered-work checklist (read to the last line) |
| C3 | `CM/200_policy.d/laws.d/modeling.d/quality.law.md` | Quality authority, FSDP, 3-layer I/O, runtime-forbidden, stack, event-driven, lint, dead code |
| C4 | `CM/200_policy.d/laws.d/modeling.d/codegen.law.md` | Classification, bad-patch-around-codegen, resolve-at-compile-time, boundary drift |
| C5 | `CM/200_policy.d/laws.d/modeling.d/reuse.law.md` | Reuse gate, existing-tool-first, ownership classification |
| C6 | `CM/200_policy.d/laws.d/modeling.d/naming.law.md` | Reserved terms, prefixes, collisions |
| C7 | `CM/300_library.d/skills.d/audit.d/audit.skill.md` | Review scope, confidence scoring, false positives, finding format |
| C8 | `CM/200_policy.d/laws.d/security.d/secrets.law.md` | Security baseline, secret handling |
| C9 | `CM/200_policy.d/laws.d/modeling.d/git.law.md` | Commit subject build marker, commit-cheap, default-branch work (§MOD.GIT.4) — governs every edit push |

## §SIGMA-REVIEW-SIGNAL-READING — Step 2 — Signal-Triggered Mandatory Reading

Collect signals from the review subject (changed paths, changed hunks, target
repo root). In the target checkout:

```bash
git diff --name-only <base>...<head>            # or the files of the supplied patch
git diff <base>...<head> | rg -o '#\[(table|reducer|view|procedure)|SpacetimeType|derive\(.*Boilerplate|boilerplate::|@genType|%raw|%identity|DO NOT EDIT|GENERATED' | sort -u
ls spacetime.json stdb-codegen.toml rescript.json bsconfig.json Cargo.toml 2>/dev/null
rg -l 'boilerplate' --glob 'Cargo.toml'
```

Every matching row is mandatory; rows are cumulative. Paths are under
`CM/300_library.d/skills.d/` unless they start with `CM/`.

| Signal | Mandatory files |
|---|---|
| any `.rs`, `Cargo.toml`, `Cargo.lock`, `.cargo/` | `rust.d/rust.skill.md`, `cargo-guard.d/cargo-guard.skill.md` |
| `boilerplate` dependency, `#[derive(Boilerplate)]`, `boilerplate!`, `templates/*.template` | `rust.d/boilerplate-rust.skill.md` |
| `spacetime.json`, `spacetimedb/`, `#[table]`/`#[reducer]`/`#[view]`/`#[procedure]`, `SpacetimeType`, `Stdb*` bindings, `stdb-codegen.toml` | `spacetimedb.d/spacetimedb.skill.md`, `CM/400_knowledge.d/gotchas.d/index.md`, `CM/600_products.d/spacetimedb.d/product.surface.md` |
| Rust **and** ReScript in scope, or schema/codegen/wire-format/transport change | `spacetimedb.d/rust-rescript-spacetimedb.skill.md`, `rust.d/sig-rust-rescript.skill.md` |
| `.res`/`.resi` with `rescript.json` at v12+ | `rescript.d/rescript.skill.md` |
| `.re`/`.rei`, `bsconfig.json`, or ReScript < v12 | `rescript.d/reasonml-rescript.skill.md` |
| `@genType`, genType output, TS consuming ReScript | `rescript.d/rescript-gentype.skill.md` |
| `__generated__/`, `module_bindings/`, "DO NOT EDIT"/"GENERATED" headers, codegen crates, templates, emitters | `generated-files-guard.d/generated-files-guard.skill.md`, `generated-files-guard.d/sig-codegen-gates.skill.md` |
| fork repository (`org/forks/*`) | every file in `CM/700_forks.d/patched.d/<fork>.d/` |
| commit/branch/history shape is part of the review | `dev.d/clean-diff.skill.md` (C9 already covers git law) |
| secrets, env files, auth, tokens, CI variables | every `*.law.md` in `CM/200_policy.d/laws.d/security.d/` |
| `.domain.md` | registry skill `domain-plan` |
| `.plan.md`, `.adr.md`, `.ops.md`, `.env.md`, backlog | registry skill `project-plan` |
| CDSE consumer or `__generated__/cdse/` | `cdse.d/sig-cdse.skill.md` + matching `cdse.d/*.skill.md` |
| UI components / visual behavior | matching `design.d/*.skill.md` (select by each skill's description) |

## §SIGMA-REVIEW-DISCOVERY — Step 3 — Discovery of Additional Mandatory Reading

Steps 1–2 are the floor. Run every sub-step; each hit that governs the touched
area becomes mandatory. "Governs" = states a rule, contract, decision, or known
defect for a path, type, crate, table, product, or technology in the review
subject.

### 3a — Skill registry (authoritative activation list)

Read `$SIGMA/agents/ron/skills-registry.ron` fully. For every `enabled: true`
entry test its `description` against the signals: a matching "Activate
for/when …" clause makes the skill mandatory; a matching "Do NOT load …" clause
excludes it. Resolve the file:

```bash
find "$CM" -name '<skill-name>.skill.md' -not -path '*/990_archive.d/*'
```

When two paths resolve, read both and diff them; a difference is itself a
finding (SSOT drift).

### 3b — Product and fork knowledge

Take the target's GitLab `path_with_namespace`, map it to a product via the
table in `CM/600_products.d/README.md` (read it). In
`CM/600_products.d/<product>.d/` read `README.md`, every `*.vision.md`, every
`*.domain.prompt.md` / `*.surface.md`, and every `knowledge.d/` file whose name
or content mentions a touched path, crate, module, table, or feature
(`rg -l '<term>' knowledge.d/`).

### 3c — Decisions and prior findings

For each term T in {repo slug, touched crate/module/table/type names, feature
name from the task or commit subject}:

```bash
rg -l -i 'T' "$CM/500_work.d" --glob '*.adr.md' --glob '*decision*' --glob '*.epic.md' \
  --glob '*.story.md' --glob '*.audit.md' --glob '*findings*.md' --glob '*.plan.md'
```

Mandatory: every ADR / decision-register entry for the touched area (a ratified
decision is a review criterion) and every prior audit/findings file for the same
area (a re-introduced known defect is a finding). For dsl-core, CDSE, or vr-next
also read `CM/500_work.d/products-main-plan.d/00-MAIN-PLAN.md`.

### 3d — Knowledge and references

- `rg -n -i '<technology or crate>' "$CM/400_knowledge.d/gotchas.d/"` — every
  matching gotcha is mandatory.
- Read `CM/400_knowledge.d/references.d/index.registry.md`; read the reference
  for the touched technology (`references.d/<tech>.d/`).
- `CM/400_knowledge.d/references.d/model-to-completion.reference.md` is
  mandatory whenever a §MOD.1d/§MOD.1e/§MOD.1f finding is reported.

### 3e — Module index

`CM/index.generated.json` lists every context module (`module_ref`, `tags`,
`products`, `depends_on`, `path`):

```bash
python3 -c "import json,sys;m=json.load(open('$CM/index.generated.json'))['modules'];t=sys.argv[1].lower()
[print(x['module_ref'],x['path']) for x in m if t in (' '.join(x['tags']+x['products'])+x['module_ref']).lower() and x['lifecycle']=='active']" <term>
```

`path` values use `/home/fg/...` — apply the Step 0 mapping. An atom under a
`atoms.d/` directory is contained in its rendered `*.skill.md` / `*.law.md`; if
that rendered file is in the ledger, the atom is covered. Every other active
module returned whose body states a rule for the touched area is mandatory;
follow its `depends_on` one level.

### 3f — Transitive references

Every § id or path a mandatory file names as the owner of a rule you intend to
apply is mandatory before you cite it. Resolve a § id to its defining file:

```bash
rg -n '^#+ §<ID>\b' "$CM/200_policy.d" "$CM/300_library.d" "$CM/600_products.d" "$CM/700_forks.d" \
  --glob '!**/__generated__/**' --glob '!**/atoms.d/**'
```

`__generated__/` governance-layer files and `atoms.d/` sources are projections
of the same text; read and cite the defining `*.law.md` / `*.skill.md`.

### 3g — Target repository

Read when present: `CLAUDE.md`, `AGENTS.md`, `README.md`, `docs.d/` index and
`ops.md`, `spacetime.json`, `stdb-codegen.toml`, `rescript.json`, root and
touched-crate `Cargo.toml` (lints, workspace deps), `clippy.toml`,
`rustfmt.toml`, `.gitlab-ci.yml` (which jobs gate which checks — needed for
§SIGMA-REVIEW-EXECUTION), pre-commit hooks.
These are conventions: advisory against HARD laws, binding otherwise.

### Excluded sources (never authority)

`CM/990_archive.d/`, `$SIGMA/__mirror__/`, `$SIGMA/.artifacts/`,
`CM/900_state.d/`, `CM/750_reality.d/`. Readable as incident evidence, never
cited as the rule.

## §SIGMA-REVIEW-LEDGER — Step 4 — Reading Ledger

The ledger is the first section of the report. It is the proof of Steps 1–3;
do not describe the reading in prose instead.

```text
Reading Ledger
Sigma HEAD: <sha>   Target: <path_with_namespace>@<base>..<head>
Signals: <comma-separated signals from Step 2>

| File | Lines read / total | Trigger (C#, Step-2 row, or 3a–3g + matched term) |
|---|---|---|
| CM/200_policy.d/laws.d/modeling.d/authoring.law.md | 1–857 / 857 | C2 |

Discovery queries run: <3a matches> | <3b product> | <3c terms> | <3d terms> | <3e terms>
Excluded after check: <file — reason>
```

A row with lines read < total is a ledger defect: finish reading, then update
the row.

## §SIGMA-REVIEW-REVIEW — Step 5 — Review

- Scope and false-positive rules: C7 (`audit.skill.md`). Report every gate
  blocker; non-gate issues only at confidence ≥ 80.
- Apply §MOD.10 (all items) to the change, plus the self-check list of every
  loaded stack skill (`§RUST-SELF-CHECK`, `§STDB-SELF-CHECK`, `§RRS-SELF-CHECK`,
  `§BOILERPLATE-RUST-SELF-CHECK`, …), with build/lint items mapped to the
  runtime's gate per §SIGMA-REVIEW-EXECUTION.
- Every finding cites the exact § id and its ledger file, carries file/line
  evidence from the review subject, and names the source-truth fix at the
  correct layer (§MOD.CODEGEN.OWNERSHIP, §MOD.QUALITY.ARCH-LEVEL).
- Hard-gate violations are stated as invalid, never as "tradeoff" or
  "consider" (§MOD.QUALITY.GATE-LANGUAGE).
- Edit code only when the task asks for fixes; then follow
  §SIGMA-REVIEW-EDIT-DESTINATION.

## §SIGMA-REVIEW-REPORT-DESTINATION — Step 6 — Report Destination

For a persistent review report explicitly required for a Sigma-owned review, use
`context-modules.d/500_work.d/audits.d/<target-slug>-sigma-review-<YYYY-MM-DD>.audit.md`.
A second report for the same target and day may use `-2`, `-3`; multipart reports
may use a sibling directory. Include the review range and a reading ledger.

For other tasks, use the caller's contracted destination. Scratch and local
working evidence are valid when the contract requires them; they are not
published report destinations. Never infer a publication command from
review completion, report location, or this skill. Publication requires direct
current authorization under workspace policy; publication from `ai-ag/` is
forbidden. Where publication is authorized, preserve existing work and use the
normal non-force publication flow. Do not switch branches, rebase, or push as an
automatic response to a rejection; report the blocker for authorization or
resolution.

## §SIGMA-REVIEW-EDIT-DESTINATION — Step 7 — Edits Land in the Owning GitLab Repository

- **Owner:** the GitLab repository whose `path_with_namespace` owns the file
  (§MOD.17 ownership classification). A fix whose owner is another repository
  (generator, shared crate, fork) is made there, not patched in the consumer
  (§MOD.CODEGEN.OWNERSHIP, §MOD.5f).
- **Branch and lifecycle:** follow the task contract and current workspace
  publication authority. Do not switch branches, rebase, or publish
  automatically; commit per §MOD.GIT.COMMIT-CHEAP. Preserve existing and
  foreign work.
- **Verification:** use the applicable gate from §SIGMA-REVIEW-EXECUTION.
  Record its result when the task contract requires it. A verification gate
  does not itself authorize publication.
- **Generated files:** never hand-edit; fix the generator/template/schema and
  let the pipeline or the repository's documented regen step reproduce the
  output (§MOD.5f, §MOD.CODEGEN.REGEN-WORKFLOW). If regeneration requires a
  local tool run, state that as an open item with the exact command for Faruk.
- **ai-ag repositories:** publication from `ai-ag/` is forbidden (§MOD.GIT.3).
- A direct publication authorization applies only to its stated scope. Without
  it, keep edits local and report any external gate as unverified.
