# F1-core — context provenance for every category

Branch: create `grb/f1core` from `main` (the name is free on the fork).

## Goal
Every context category in `ContextCategory` (`ST/context_policy.rs`) can be switched off per
session from F7 and really changes the next model request (Faruk decision (b): everything is
switchable, only warnings). The current prompt and its attachments are always sent.
Design: `docs/fork/design/f6-f7.md` §1, §2, §7 and waves A1/A2/B1.

## Already on main
- `ContextCategory`, `RuntimeNotice`, `ContextSwitch`, `ContextPolicy`, `all_categories()`.
- `ConversationItem::context_category()` (`ST/context_provenance.rs`): exhaustive, derived from the
  variant and `SyntheticReason`; never stored.
- M5 per-item removal (`ContextItemRef`, replay persistence) for turns and tool exchanges.

## Next steps
1. Producers for the reserved origins: `docs/fork/briefs/f1core-reserved-origins.md`
   (SessionPrefix, DirectBash, GoalSetup).
2. Mixed-content sections: only items that truly carry several categories (system prompt fragments,
   the startup prefix if it bundles project rules / skills / MCP catalog / memory). Model as
   `Sections::{Whole, Split(Vec<ContextSection>)}` with serde default `Whole` and
   `skip_serializing_if` so old sessions load unchanged; decision in
   `docs/fork/reviews/f1core-sections-decision.md`; earlier code on `archive/f1core-wip-sections`.
3. A2 request projection in `CS/actor/request_builder.rs::build_conversation_request`: apply the
   session's `ContextPolicy` in the same single pass as pruning (no extra scan per request), keep
   provider-required tool call/result pairs and reasoning envelopes valid, tool definitions switchable.
4. B1 shell policy API + persistence (session state, replay, compaction must not resurrect excluded
   content, child inheritance).
5. Tests: every category OFF through the request builder with an independent absence/pairing
   oracle, then ON again; current prompt always present.
