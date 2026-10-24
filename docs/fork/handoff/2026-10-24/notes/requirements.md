# Goal requirements and baseline — 2026-10-22

## Scope
Plan criteria cover Grok release, Gateway Responses sequence fix, and original stream-A Codex API work. Historical envelope/governance/provider-resilience tasks remain outside this goal unless directly needed. User explicitly removed unchanged-codex-reference evidence requirement; no further status/provenance checking for that reference.

## Matrix
| Requirement | Baseline / owner | Current evidence / next gate |
|---|---|---|
| Grok release | main b519faf78d5e73f216028adcd4eb912ea72d5944 | Published; linux-test run39690775097, build+test monitor 01a160b6-f8d6-7412-a472-9bfa05bdbe9f; exact SHA CI pending |
| Grok stage | glue 7fe173e exact run+commit support | ten mock tests passed earlier; actual final artifact not staged |
| Responses sequence | gw-grok-features 849710bc + uncommitted wire patches | Chat/buffered/CG implemented, native synthetic failure incomplete; worker 01a160b1-1751-7c11-a5d7-8502ec9cf29d |
| Independent activation | gateway main0385942f has unrelated WIP | isolate integration worktree for committed required patch; no copying unrelated capability hunks |
| Responses Lite | gw-codex-lite7517a02f + WIP | tools projection present, base instructions/function response path needs contract check |
| Model flags | same | effort ladder and catalog parity need completion; worker 01a160b1-95bd-7680-aa9c-f680c67710a0 |
| Summary/tier/cache | same | summary omit, tier unavailable tests present; full original source contracts pending |
| WS | original stream A item8 | analysis only, no transport implementation required |
| Registry/redaction | shared config/secrets | fail-closed load error blocks gateway test; worker 01a160b7-a682-7c01-97c3-205cc323b7f9 investigates exact cause |

## Baseline work isolation
Main sigma-crates has unrelated orchestrator, secret-scan and generator changes; preserve them. Worktree gw-grok-features includes capability committed changes and unrelated created_at fix; extract sequence-only changes as identified by prior report. Worktree gw-codex-lite includes Cargo.lock change; inspect source identity before build. No local Grok or other Rust package build.
