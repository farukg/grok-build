use super::*;
use xai_grok_sampling_types::{ContextCategory, ContextSwitch};

fn edit(store: &mut ContextPolicyStore, id: &str, mutation: ContextPolicyMutation) {
    store.load().unwrap();
    let expected = store.selection(id, "actor").unwrap().version;
    store.edit(id, "actor", &ContextPolicyEdit { expected, mutation }).unwrap();
}

fn category(category: ContextCategory, value: ContextSwitch, target: ContextScope) -> ContextPolicyMutation {
    ContextPolicyMutation::SetCategories { categories: vec![category].try_into().unwrap(), value, target }
}

#[test]
fn scopes_restore_dormant_overrides_without_copying_global_values() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ContextPolicyStore::new(dir.path().to_owned());
    edit(&mut store, "a", category(ContextCategory::Reasoning, ContextSwitch::Excluded, ContextScope::Session));
    edit(&mut store, "a", ContextPolicyMutation::SelectScope { scope: ContextScope::Global });
    assert!(store.effective("a").unwrap().includes(ContextCategory::Reasoning));
    edit(&mut store, "b", category(ContextCategory::Memory, ContextSwitch::Excluded, ContextScope::Global));
    assert!(!store.effective("future").unwrap().includes(ContextCategory::Memory));
    edit(&mut store, "a", ContextPolicyMutation::SelectScope { scope: ContextScope::Session });
    let restored = store.effective("a").unwrap();
    assert!(!restored.includes(ContextCategory::Reasoning));
    assert!(!restored.includes(ContextCategory::Memory));
    let mut restarted = ContextPolicyStore::new(dir.path().to_owned());
    restarted.load().unwrap();
    assert_eq!(restarted.effective("a").unwrap(), restored);
    assert_eq!(restarted.selection("a", "new-actor").unwrap().scope, ContextScope::Session);
}

#[test]
fn session_alternative_changes_only_the_bulk_target_and_stale_confirmation_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ContextPolicyStore::new(dir.path().to_owned());
    edit(&mut store, "a", category(ContextCategory::Reasoning, ContextSwitch::Excluded, ContextScope::Session));
    edit(&mut store, "a", ContextPolicyMutation::SelectScope { scope: ContextScope::Global });
    let stale = store.selection("a", "actor").unwrap().version;
    edit(&mut store, "a", ContextPolicyMutation::SetCategories {
        categories: vec![ContextCategory::Memory, ContextCategory::ToolDefinitions].try_into().unwrap(),
        value: ContextSwitch::Excluded, target: ContextScope::Session,
    });
    let before = std::fs::read(dir.path().join("context_policy.toml")).unwrap();
    let mutation = ContextPolicyMutation::SelectScope { scope: ContextScope::Global };
    assert!(matches!(store.edit("a", "actor", &ContextPolicyEdit { expected: stale, mutation: mutation.clone() }), Err(ContextPolicyError::Stale)));
    let expected = store.selection("a", "actor").unwrap().version;
    assert!(matches!(store.edit("a", "different-actor", &ContextPolicyEdit { expected, mutation }), Err(ContextPolicyError::OriginChanged)));
    assert_eq!(std::fs::read(dir.path().join("context_policy.toml")).unwrap(), before);
    let policy = store.effective("a").unwrap();
    assert!(!policy.includes(ContextCategory::Reasoning));
    assert!(!policy.includes(ContextCategory::Memory));
    assert!(!policy.includes(ContextCategory::ToolDefinitions));
    assert!(store.effective("b").unwrap().is_unrestricted());
}

#[test]
fn legacy_json_is_migrated_as_explicit_session_choices_and_never_modified() {
    let root = tempfile::tempdir().unwrap();
    let session = tempfile::tempdir().unwrap();
    let legacy = serde_json::to_vec(&ContextPolicy::default()).unwrap();
    std::fs::write(session.path().join("context_policy.json"), &legacy).unwrap();
    let mut store = ContextPolicyStore::new(root.path().to_owned());
    store.ensure_session("legacy", session.path()).unwrap();
    edit(&mut store, "new", category(ContextCategory::Reasoning, ContextSwitch::Excluded, ContextScope::Global));
    assert!(store.effective("legacy").unwrap().includes(ContextCategory::Reasoning));
    assert!(!store.effective("new").unwrap().includes(ContextCategory::Reasoning));
    assert_eq!(store.selection("legacy", "actor").unwrap().scope, ContextScope::Session);
    assert_eq!(std::fs::read(session.path().join("context_policy.json")).unwrap(), legacy);
    let mut restarted = ContextPolicyStore::new(root.path().to_owned());
    restarted.ensure_session("legacy", session.path()).unwrap();
    assert!(restarted.effective("legacy").unwrap().includes(ContextCategory::Reasoning));
    assert!(root.path().join("context_policy.toml").is_file());
}

#[test]
fn malformed_legacy_and_owned_toml_never_become_included_defaults() {
    let root = tempfile::tempdir().unwrap();
    let session = tempfile::tempdir().unwrap();
    std::fs::write(session.path().join("context_policy.json"), "not-json").unwrap();
    let mut store = ContextPolicyStore::new(root.path().to_owned());
    assert!(matches!(store.ensure_session("a", session.path()), Err(ContextPolicyError::Legacy(_))));
    assert!(!root.path().join("context_policy.toml").exists());
    std::fs::write(root.path().join("context_policy.toml"), "not = [toml").unwrap();
    let mut restarted = ContextPolicyStore::new(root.path().to_owned());
    assert!(matches!(restarted.load(), Err(ContextPolicyError::Decode(_))));
}

#[test]
fn unreadable_owned_state_keeps_actual_cached_policy_and_typed_io_cause() {
    let root = tempfile::tempdir().unwrap();
    let mut store = ContextPolicyStore::new(root.path().to_owned());
    store.load().unwrap();
    let expected = store.selection("a", "actor").unwrap().version;
    std::fs::create_dir(root.path().join("context_policy.toml")).unwrap();
    let error = store.edit("a", "actor", &ContextPolicyEdit {
        expected: expected.clone(),
        mutation: category(ContextCategory::Reasoning, ContextSwitch::Excluded, ContextScope::Global),
    }).unwrap_err();
    assert!(matches!(error, ContextPolicyError::Io(_)));
    assert_eq!(store.selection("a", "actor").unwrap().version, expected);
    assert!(store.effective("a").unwrap().is_unrestricted());
    assert!(root.path().join("context_policy.toml").is_dir());
}
