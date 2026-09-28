# F1-core provenance — lead decision (binding)

Review of `c30d086e wip(context): add typed item provenance` — not integrable as is:

1. **Persisted-history break.** `SystemItem`/`UserItem` gained `context_category: ContextCategory` and `sections: Vec<ContextSection>` without serde defaults → every existing session JSONL item (which lacks both keys) fails to deserialize. Loading old sessions is a hard requirement.
2. **Same fact twice (SSOT).** For whole-item categories, `context_category` duplicates what `synthetic_reason` already says (the classifier is a pure function of it). Storing both lets them diverge.
3. **Wire weight.** Every persisted item now serializes a category and an empty `sections` array.

## Decision
- **Whole-item category is derived, never stored:** `ConversationItem::context_category(&self) -> ContextCategory` = the exhaustive classifier over `synthetic_reason` / variant you already wrote (move `classify_legacy_item` into that method; it is not "legacy", it is the classifier). Remove the `context_category` fields, the `tagged` builder and the JSONL load hook — nothing to classify at load when it is derived.
- **The only stored provenance is mixed-content sections**, and only where one item really carries several categories (system prompt fragments, the startup prefix with project rules/skills/MCP/memory). Model it as an explicit sum so an item cannot be "sectioned and unsectioned" at once, e.g. on `SystemItem`/`UserItem`:
  `#[serde(default, skip_serializing_if = "Sections::is_whole")] pub sections: Sections` with `enum Sections { Whole, Split(Vec<ContextSection>) }` (Default = Whole). Old items deserialize as `Whole`; `Whole` is not written. `context_category()` on a `Split` item is irrelevant — the request projection filters per section.
- Where `synthetic_reason` cannot express a whole-item origin today (design note: `SessionPrefix`, `DirectBash`, `GoalSetup` are reserved but emitted as `Human`), fix the PRODUCERS to set the right `SyntheticReason` (A1) rather than adding a parallel field.
- Constructors: `Sections::Whole` needs no argument, so the 54 struct-literal sites only change if they build mixed content (they get `..` or the new field via the constructor). Prefer adding the field with `Default` so literals using `..Default::default()` stay untouched; struct literals without it get `sections: Sections::Whole` — mechanical, allowed.

Keep the `context_policy.rs` cleanup (match instead of find_map/bool-match) — good.
