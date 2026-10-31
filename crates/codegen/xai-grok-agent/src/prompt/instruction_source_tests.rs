use std::error::Error;
use std::fs;
use std::path::Path;

use super::*;

fn spec(dir: &Path, name: &str, text: &[u8], role: InstructionRole) -> InstructionFileSpec {
    let path = dir.join(name);
    fs::write(&path, text).unwrap();
    InstructionFileSpec {
        id: InstructionSourceId::try_from(name.to_owned()).unwrap(),
        path,
        origin: InstructionOrigin::ConfiguredFile,
        format: InstructionFormat::PlainText,
        policy: InstructionPolicy { role },
        digest: InstructionDigest::of_bytes(text),
    }
}

fn start(sources: Vec<InstructionFileSpec>) -> InstructionSnapshot {
    InstructionSnapshot::start(load_instruction_snapshot(&InstructionManifest::V1(sources)).unwrap())
}

#[test]
fn sha256_is_exact_bytes_not_trimmed_or_newline_normalized() {
    let known: InstructionDigest =
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            .parse().unwrap();
    assert_eq!(InstructionDigest::of_bytes(b"abc"), known);
    assert_ne!(InstructionDigest::of_bytes(b"abc\n"), known);
    assert_ne!(InstructionDigest::of_bytes(b"abc\r\n"), known);
    assert!("BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
        .parse::<InstructionDigest>().is_err());
}

#[test]
fn disk_edit_and_catalog_load_cannot_change_active_session_without_activation() {
    let dir = tempfile::tempdir().unwrap();
    let old_spec = spec(dir.path(), "rules.md", b"old\n", InstructionRole::User);
    let active = start(vec![old_spec.clone()]);
    let saved = serde_json::to_vec(&active).unwrap();
    let new_spec = spec(dir.path(), "rules.md", b"new\n", InstructionRole::User);
    let loaded = load_instruction_snapshot(&InstructionManifest::V1(vec![new_spec])).unwrap();
    assert_eq!(serde_json::to_vec(&active).unwrap(), saved);
    let plan = prepare_instruction_activation(&active, loaded);
    assert_eq!(active.sources()[0].raw_text(), "old\n");
    assert_eq!(plan.candidate().sources()[0].raw_text(), "new\n");
    assert_eq!(plan.expected_prefix_change(), ExpectedPrefixChange::Changed);
    let next = plan.activate(&active).unwrap();
    assert_eq!(next.sources()[0].raw_text(), "new\n");
    assert_eq!(active.sources()[0].raw_text(), "old\n");
}

#[test]
fn role_only_reload_exposes_both_versions_without_changing_category_or_origin() {
    let dir = tempfile::tempdir().unwrap();
    let old_spec = spec(dir.path(), "prompt.md", b"same", InstructionRole::System);
    let active = start(vec![old_spec.clone()]);
    let mut new_spec = old_spec;
    new_spec.policy.role = InstructionRole::Developer;
    let loaded = load_instruction_snapshot(&InstructionManifest::V1(vec![new_spec])).unwrap();
    let plan = prepare_instruction_activation(&active, loaded);
    assert_eq!(plan.expected_prefix_change(), ExpectedPrefixChange::Changed);
    let changes = plan.changes();
    let [InstructionSourceChange::Changed { old, new, old_index, new_index }] = changes.as_slice() else {
        panic!("role change must be visible");
    };
    assert_eq!((*old_index, *new_index), (0, 0));
    assert_eq!(old.policy().role, InstructionRole::System);
    assert_eq!(new.policy().role, InstructionRole::Developer);
    assert_eq!(old.digest(), new.digest());
    assert_eq!(old.provenance(), new.provenance());
    assert_ne!(plan.previous().digest(), plan.candidate().digest());
}

#[test]
fn complete_manifest_rejects_mixed_revisions_and_preserves_old_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let first = spec(dir.path(), "first.md", b"first old", InstructionRole::System);
    let second = spec(dir.path(), "second.md", b"second old", InstructionRole::User);
    let active = start(vec![first.clone(), second.clone()]);
    let saved = serde_json::to_vec(&active).unwrap();
    let first_new = spec(dir.path(), "first.md", b"first new", InstructionRole::System);
    let mut second_new = second.clone();
    second_new.digest = InstructionDigest::of_bytes(b"second new");
    let manifest = InstructionManifest::V1(vec![first_new, second_new]);
    let error = load_instruction_snapshot(&manifest).unwrap_err();
    assert!(matches!(error, InstructionLoadError::DigestMismatch { ref path, .. } if path == &second.path));
    assert_eq!(serde_json::to_vec(&active).unwrap(), saved);
    fs::remove_file(&second.path).unwrap();
    let error = load_instruction_snapshot(&manifest).unwrap_err();
    assert!(matches!(&error, InstructionLoadError::Missing { path, source, .. }
        if path == &second.path && source.kind() == std::io::ErrorKind::NotFound));
    assert!(error.source().is_some());
    assert_eq!(serde_json::to_vec(&active).unwrap(), saved);
}

#[test]
fn order_is_manifest_order_and_reordering_is_a_visible_prefix_change() {
    let dir = tempfile::tempdir().unwrap();
    let z = spec(dir.path(), "z.md", b"z", InstructionRole::User);
    let a = spec(dir.path(), "a.md", b"a", InstructionRole::System);
    let active = start(vec![z.clone(), a.clone()]);
    let same = load_instruction_snapshot(&InstructionManifest::V1(vec![z.clone(), a.clone()])).unwrap();
    let same_plan = prepare_instruction_activation(&active, same);
    assert_eq!(same_plan.expected_prefix_change(), ExpectedPrefixChange::Unchanged);
    assert!(same_plan.changes().is_empty());
    assert_eq!(active.sources().iter().map(|s| s.id().as_str()).collect::<Vec<_>>(), ["z.md", "a.md"]);
    let reordered = load_instruction_snapshot(&InstructionManifest::V1(vec![a, z])).unwrap();
    let plan = prepare_instruction_activation(&active, reordered);
    assert_eq!(plan.expected_prefix_change(), ExpectedPrefixChange::Changed);
    assert_eq!(plan.changes().len(), 2);
    assert_eq!(plan.candidate().sources()[0].id().as_str(), "a.md");
}

#[test]
fn exact_text_and_canonical_paths_survive_restore_without_any_disk_sources() {
    let dir = tempfile::tempdir().unwrap();
    let text = "\u{feff}  Grüße\r\n\n$({not a command})  ";
    let file = spec(dir.path(), "text.md", text.as_bytes(), InstructionRole::Developer);
    let canonical = dunce::canonicalize(&file.path).unwrap();
    let active = start(vec![file]);
    assert_eq!(active.sources()[0].raw_text().as_bytes(), text.as_bytes());
    assert_eq!(active.sources()[0].instruction_text(), text);
    assert_eq!(active.sources()[0].provenance().canonical_path(), canonical);
    let saved = serde_json::to_vec(&active).unwrap();
    drop(dir);
    let restored: InstructionSnapshot = serde_json::from_slice(&saved).unwrap();
    assert_eq!(restored, active);
    assert_eq!(restored.digest(), active.digest());
    assert_eq!(serde_json::to_vec(&restored).unwrap(), saved);
}

#[test]
fn equal_bytes_at_different_paths_remain_distinct_sources() {
    let dir = tempfile::tempdir().unwrap();
    let a = spec(dir.path(), "a.md", b"same", InstructionRole::System);
    let b = spec(dir.path(), "b.md", b"same", InstructionRole::System);
    let snapshot = start(vec![a, b]);
    assert_eq!(snapshot.sources()[0].digest(), snapshot.sources()[1].digest());
    assert_ne!(snapshot.sources()[0].id(), snapshot.sources()[1].id());
    assert_ne!(snapshot.sources()[0].provenance(), snapshot.sources()[1].provenance());
}

#[test]
fn fresh_and_old_sessions_using_same_files_are_isolated_and_stale_plan_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let old = spec(dir.path(), "shared.md", b"old", InstructionRole::User);
    let old_session = start(vec![old]);
    let new = spec(dir.path(), "shared.md", b"new", InstructionRole::System);
    let fresh_session = start(vec![new.clone()]);
    let loaded = load_instruction_snapshot(&InstructionManifest::V1(vec![new])).unwrap();
    let plan = prepare_instruction_activation(&old_session, loaded);
    assert!(plan.activate(&fresh_session).is_err());
    assert_eq!(old_session.sources()[0].raw_text(), "old");
    assert_eq!(old_session.sources()[0].policy().role, InstructionRole::User);
    assert_eq!(fresh_session.sources()[0].raw_text(), "new");
    assert_eq!(fresh_session.sources()[0].policy().role, InstructionRole::System);
}

#[test]
fn required_runtime_asset_missing_has_no_embedded_fallback_and_defaults_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let assets = RuntimeInstructionAssets::in_grok_home(dir.path());
    assert_eq!(assets.root(), dir.path().join("bundled/instructions"));
    for template in [RuntimeInstructionTemplate::Base, RuntimeInstructionTemplate::ApplyPatch, RuntimeInstructionTemplate::Subagent] {
        let spec = assets.template_spec(template, InstructionDigest::of_bytes(b"unused"));
        assert_eq!(spec.policy.role, InstructionRole::System);
        let error = load_instruction_snapshot(&InstructionManifest::V1(vec![spec])).unwrap_err();
        assert!(matches!(error, InstructionLoadError::Missing { .. }));
    }
    let error = load_instruction_manifest(&assets.manifest_path()).unwrap_err();
    assert!(matches!(error, InstructionLoadError::Missing { .. }));
}

#[test]
fn runtime_template_bytes_come_from_existing_plaintext_source_not_a_new_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let assets = RuntimeInstructionAssets::at_root(dir.path().to_path_buf());
    fs::create_dir(dir.path().join("templates")).unwrap();
    let bytes = include_bytes!("../../templates/prompt.md");
    fs::write(assets.template_path(RuntimeInstructionTemplate::Base), bytes).unwrap();
    let file = assets.template_spec(RuntimeInstructionTemplate::Base, InstructionDigest::of_bytes(bytes));
    let snapshot = start(vec![file]);
    assert_eq!(snapshot.sources()[0].raw_text().as_bytes(), bytes);
    fs::write(assets.template_path(RuntimeInstructionTemplate::Base), b"edited runtime asset").unwrap();
    assert_eq!(snapshot.sources()[0].raw_text().as_bytes(), bytes);
}

#[test]
fn rule_input_keeps_existing_discovery_scope_and_existing_frontmatter_parser() {
    let dir = tempfile::tempdir().unwrap();
    let raw = "---\ndescription: fixture\n---\nKeep this body.\n";
    let path = dir.path().join("rule.md");
    fs::write(&path, raw).unwrap();
    let discovered = AgentConfigFile {
        file_name: "rule.md".to_owned(),
        file_path: path.to_str().unwrap().to_owned(),
        content: "Keep this body.".to_owned(),
        source: InstructionSource::Configured,
    };
    let file = InstructionFileSpec::discovered_rule(
        &discovered, InstructionFormat::RuleMarkdown, InstructionDigest::of_bytes(raw.as_bytes()),
    ).unwrap();
    assert_eq!(file.policy.role, InstructionRole::User);
    let snapshot = start(vec![file]);
    assert_eq!(snapshot.sources()[0].raw_text(), raw);
    assert_eq!(snapshot.sources()[0].instruction_text().trim(), "Keep this body.");
    assert_eq!(snapshot.sources()[0].provenance().origin(),
        &InstructionOrigin::DiscoveredRule(InstructionSource::Configured));
}

#[test]
fn disk_manifest_parses_real_roles_and_resolves_paths_against_manifest_directory() {
    let dir = tempfile::tempdir().unwrap();
    let file = spec(dir.path(), "prompt.md", b"abc", InstructionRole::Developer);
    let mut relative = file;
    relative.path = "prompt.md".into();
    let path = dir.path().join("instructions.json");
    fs::write(&path, serde_json::to_vec(&InstructionManifest::V1(vec![relative])).unwrap()).unwrap();
    let loaded = load_instruction_manifest(&path).unwrap();
    assert_eq!(loaded.snapshot().sources()[0].policy().role, InstructionRole::Developer);
    assert_eq!(loaded.snapshot().sources()[0].raw_text(), "abc");
    let valid = fs::read_to_string(&path).unwrap();
    fs::write(&path, valid.replace("developer", "assistant")).unwrap();
    let error = load_instruction_manifest(&path).unwrap_err();
    assert!(matches!(&error, InstructionLoadError::InvalidManifest { .. }));
    assert!(error.source().is_some());
}

#[test]
fn invalid_utf8_duplicate_identity_duplicate_canonical_file_and_directory_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let invalid = spec(dir.path(), "invalid.md", &[0xff], InstructionRole::User);
    let error = load_instruction_snapshot(&InstructionManifest::V1(vec![invalid])).unwrap_err();
    assert!(matches!(&error, InstructionLoadError::InvalidUtf8 { .. }));
    assert!(error.source().is_some());
    let file = spec(dir.path(), "one.md", b"one", InstructionRole::User);
    let duplicate_id = InstructionManifest::V1(vec![file.clone(), file.clone()]);
    assert!(matches!(load_instruction_snapshot(&duplicate_id),
        Err(InstructionLoadError::InvalidSource(InvalidInstructionSnapshot::DuplicateIdentity { .. }))));
    let mut duplicate_file = file.clone();
    duplicate_file.id = InstructionSourceId::try_from("other".to_owned()).unwrap();
    let manifest = InstructionManifest::V1(vec![file.clone(), duplicate_file]);
    assert!(matches!(load_instruction_snapshot(&manifest),
        Err(InstructionLoadError::InvalidSource(InvalidInstructionSnapshot::DuplicateFile { .. }))));
    let mut directory = file;
    directory.path = dir.path().to_path_buf();
    assert!(matches!(load_instruction_snapshot(&InstructionManifest::V1(vec![directory])),
        Err(InstructionLoadError::NotRegularFile { .. })));
}

#[test]
fn snapshot_restore_validates_source_identity_instead_of_bypassing_construction() {
    let dir = tempfile::tempdir().unwrap();
    let snapshot = start(vec![spec(dir.path(), "one.md", b"one", InstructionRole::User)]);
    let saved = serde_json::to_string(&snapshot).unwrap();
    let invalid = saved.replace("\"id\":\"one.md\"", "\"id\":\" \"");
    assert_ne!(saved, invalid);
    assert!(serde_json::from_str::<InstructionSnapshot>(&invalid).is_err());
}

#[test]
fn composed_runtime_projection_preserves_verified_atom_provenance_after_sources_are_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let atom = spec(dir.path(), "workspace.atom.md", b"canonical atom fixture", InstructionRole::User);
    let mut projection = spec(dir.path(), "runtime.md", b"composed fixture", InstructionRole::System);
    projection.origin = InstructionOrigin::ComposedProjection(vec![InstructionProjectionInput {
        kind: CanonicalInstructionInputKind::SigmaAtom,
        path: atom.path.clone(),
        digest: atom.digest,
    }]);
    let active = start(vec![projection.clone()]);
    let InstructionOrigin::ComposedProjection(inputs) = active.sources()[0].provenance().origin() else {
        panic!("projection lineage must survive loading");
    };
    assert_eq!(inputs[0].path, dunce::canonicalize(&atom.path).unwrap());
    assert_eq!(inputs[0].digest, atom.digest);
    fs::write(&atom.path, b"changed canonical input").unwrap();
    assert!(matches!(load_instruction_snapshot(&InstructionManifest::V1(vec![projection])),
        Err(InstructionLoadError::DigestMismatch { .. })));
    let saved = serde_json::to_vec(&active).unwrap();
    drop(dir);
    let restored: InstructionSnapshot = serde_json::from_slice(&saved).unwrap();
    assert_eq!(restored, active);
    assert_eq!(restored.sources()[0].raw_text(), "composed fixture");
}

#[test]
fn added_and_removed_sources_are_reported_with_their_original_order_and_paths() {
    let dir = tempfile::tempdir().unwrap();
    let old = spec(dir.path(), "old.md", b"old", InstructionRole::System);
    let active = start(vec![old.clone()]);
    let new = spec(dir.path(), "new.md", b"new", InstructionRole::User);
    let loaded = load_instruction_snapshot(&InstructionManifest::V1(vec![new.clone()])).unwrap();
    let plan = prepare_instruction_activation(&active, loaded);
    let changes = plan.changes();
    let [InstructionSourceChange::Removed { old_index, old: removed },
        InstructionSourceChange::Added { new_index, new: added }] = changes.as_slice() else {
        panic!("source replacement must report removal and addition");
    };
    assert_eq!((*old_index, *new_index), (0, 0));
    assert_eq!(removed.provenance().requested_path(), old.path);
    assert_eq!(added.provenance().requested_path(), new.path);
}

#[cfg(unix)]
#[test]
fn io_cause_and_invalid_source_shapes_remain_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let file = spec(dir.path(), "regular.md", b"data", InstructionRole::User);
    let mut invalid_parent = file.clone();
    invalid_parent.path = file.path.join("child.md");
    let error = load_instruction_snapshot(&InstructionManifest::V1(vec![invalid_parent])).unwrap_err();
    assert!(matches!(&error, InstructionLoadError::Io { source, .. }
        if source.kind() == std::io::ErrorKind::NotADirectory));
    assert!(error.source().is_some());
    let mut relative = file.clone();
    relative.path = "regular.md".into();
    assert!(matches!(load_instruction_snapshot(&InstructionManifest::V1(vec![relative])),
        Err(InstructionLoadError::InvalidSource(InvalidInstructionSnapshot::RelativePath { .. }))));
    let mut projection = file;
    projection.origin = InstructionOrigin::ComposedProjection(Vec::new());
    assert!(matches!(load_instruction_snapshot(&InstructionManifest::V1(vec![projection])),
        Err(InstructionLoadError::InvalidSource(InvalidInstructionSnapshot::EmptyProjection))));
}

#[cfg(unix)]
#[test]
fn symlink_aliases_cannot_inject_the_same_canonical_file_twice() {
    let dir = tempfile::tempdir().unwrap();
    let file = spec(dir.path(), "one.md", b"one", InstructionRole::System);
    let alias = dir.path().join("alias.md");
    std::os::unix::fs::symlink(&file.path, &alias).unwrap();
    let mut other = file.clone();
    other.id = InstructionSourceId::try_from("alias".to_owned()).unwrap();
    other.path = alias;
    assert!(matches!(load_instruction_snapshot(&InstructionManifest::V1(vec![file, other])),
        Err(InstructionLoadError::InvalidSource(InvalidInstructionSnapshot::DuplicateFile { .. }))));
}
