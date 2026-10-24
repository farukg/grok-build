# Dependent edit sequence (continuation)

1. Add occurrence-keyed strict completion projection to ChatState and existing session persistence/storage. Receipt is metadata on the SAME chat JSONL record, not a separately committed reported-resource file. Existing summary keeps a derived receipt index across compaction/history replacement; a retry returning an existing projection reconciles actor memory from authoritative history, including lost persistence acknowledgements.
2. Carry coordinator AttemptId through completion summaries and typed parent wake identity. Retire subagent task-ID ledger use and Tools-side pre-projection marking; Shell safe-point owner becomes the one projector.
3. Persist typed terminal completion/outbox at existing session summary/update owner before live wake publication; use existing terminal TurnCompleted to settle recovery obligations. Recovery of a projected-but-not-terminal parent resumes without projecting the occurrence twice. Reuse current session load/heal entrypoints; no new store/registry.
4. Add owner-driven durable append/ack-loss/restart/attempt-reuse regressions and actual idle/active sampler integrations. Extend ADR/current result only after wiring the complete path. No local build/test/check/formatter commands.

Scope does not include pruning or Pager/sig-orch. The cross-owner source inventory required more than the original targeted-call guideline; continuation calls are necessary to edit and statically reconcile the actual transaction and replay owners, not a verification substitute.
