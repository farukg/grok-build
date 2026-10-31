use super::*;
use serde_json::json;

fn attempt(n: u8) -> AttemptEvidence {
    AttemptEvidence::from_recorded(Some(&format!("at1.{n:032x}"))).unwrap()
}

fn journal(line: u64, event: &str) -> ChildEvidenceSource {
    ChildEvidenceSource::ParentJournal { line, byte_offset: line * 100, byte_len: 80, event_id: Some(event.to_owned()), digest: "a".repeat(64) }
}

fn record(attempt: AttemptEvidence, source: ChildEvidenceSource, value: ChildEvidence) -> ChildEvidenceRecord {
    ChildEvidenceRecord { subagent_id: "child".to_owned(), attempt, source, value }
}

fn summary(n: u8) -> ChildEvidenceRecord {
    record(attempt(n), ChildEvidenceSource::ChildSummary { child_session_id: "child".to_owned(), digest: "b".repeat(64) }, ChildEvidence::SummaryIdentity { child_session_id: "child".to_owned(), fork_parent_prompt_id: Some("prompt".to_owned()) })
}

fn terminal(n: u8, source: ChildEvidenceSource, state: RecordedTerminalState) -> ChildEvidenceRecord {
    record(attempt(n), source.clone(), ChildEvidence::Terminal { terminal: ChildTerminalEvidence::Recorded { result: RecordedSubagentTerminal {
        attempt: attempt(n), state, completed_at: None, duration_ms: Some(337393), tool_calls: Some(0), turns: Some(0), tokens_used: None, output: Some("worker output".into()), sources: vec![source],
    } } })
}

fn import(graph: &mut SpawnGraph, records: Vec<ChildEvidenceRecord>) -> ChildInventorySnapshot {
    let base = graph.begin_inventory("parent").unwrap();
    graph.import_inventory(ChildInventoryImport { parent_session_id: "parent".to_owned(), base, records, issues: vec![] }, &[]).unwrap()
}

fn inspection(snapshot: &ChildInventorySnapshot) -> &SubagentInspection {
    match &snapshot.entries[0] {
        ChildInventoryEntry::Child { inspection, .. } => inspection,
        entry => panic!("expected child, got {entry:?}"),
    }
}

#[test]
fn positive_summary_does_not_invent_metadata_or_activity() {
    let mut graph = SpawnGraph::default();
    let snapshot = import(&mut graph, vec![summary(1)]);
    let child = inspection(&snapshot);
    assert_eq!(child.child_session_id.as_deref(), Some("child"));
    assert_eq!(child.fork_parent_prompt_id.as_deref(), Some("prompt"));
    assert_eq!(child.description, None);
    assert_eq!(child.subagent_type, None);
    assert_eq!(child.recorded_owner, None);
    assert_eq!(child.recorded_spawner, None);
    assert!(matches!(child.observation, ChildObservation::IdentityOnly { .. }));
    assert!(child.snapshot().is_none());
    assert!(!graph.is_reachable_from("child", "prompt"));
}

#[test]
fn low_event_id_terminal_remains_terminal_in_both_input_orders() {
    let spawn = record(attempt(1), journal(20, "event.9000"), ChildEvidence::Nonterminal { phase: HistoricalChildPhase::Spawned });
    let finish = terminal(1, journal(21, "event.38"), RecordedTerminalState::Cancelled { cause: None, message: Some("restart text is not a cause".to_owned()) });
    for records in [vec![summary(1), spawn.clone(), finish.clone()], vec![finish.clone(), spawn.clone(), summary(1)]] {
        let mut graph = SpawnGraph::default();
        let snapshot = import(&mut graph, records);
        let child = inspection(&snapshot);
        assert!(matches!(&child.observation, ChildObservation::Terminal { terminal } if terminal.legacy_status() == "cancelled"));
        assert_eq!(child.observation.state(), None);
        assert_eq!(child.observation.duration_ms(), Some(337393));
        assert!(child.snapshot().is_none());
    }
}

#[test]
fn old_attempt_finish_after_new_spawn_cannot_close_new_attempt() {
    let mut graph = SpawnGraph::default();
    let snapshot = import(&mut graph, vec![summary(2),
        record(attempt(1), journal(1, "event.99"), ChildEvidence::Nonterminal { phase: HistoricalChildPhase::Spawned }),
        record(attempt(2), journal(2, "event.2"), ChildEvidence::Nonterminal { phase: HistoricalChildPhase::Spawned }),
        terminal(1, journal(3, "event.1"), RecordedTerminalState::Completed)]);
    let child = inspection(&snapshot);
    assert_eq!(child.attempt, attempt(2));
    assert!(matches!(child.observation, ChildObservation::Uncustodied { .. }));
}

#[test]
fn equally_ranked_conflicting_terminals_have_no_arbitrary_winner() {
    for reverse in [false, true] {
        let mut records = vec![summary(1), terminal(1, journal(2, "event.8"), RecordedTerminalState::Completed), terminal(1, journal(3, "event.1"), RecordedTerminalState::Cancelled { cause: None, message: None })];
        if reverse { records.reverse(); }
        let mut graph = SpawnGraph::default();
        let snapshot = import(&mut graph, records);
        assert!(matches!(&snapshot.entries[0], ChildInventoryEntry::Conflict { issues, .. } if issues.iter().any(|i| matches!(i, InventoryIssue::TerminalConflict { sources } if sources.len() >= 2))));
    }
}

#[test]
fn obsolete_import_base_is_rejected_without_partial_merge() {
    let mut graph = SpawnGraph::default();
    let old = graph.begin_inventory("parent").unwrap();
    let current = graph.begin_inventory("parent").unwrap();
    assert!(matches!(graph.import_inventory(ChildInventoryImport { parent_session_id: "parent".into(), base: old, records: vec![summary(1)], issues: vec![] }, &[]), Err(SubagentAccessError::InventorySuperseded)));
    let snapshot = graph.import_inventory(ChildInventoryImport { parent_session_id: "parent".into(), base: current, records: vec![], issues: vec![] }, &[]).unwrap();
    assert!(snapshot.entries.is_empty());
}

#[test]
fn live_transition_during_read_preserves_base_but_wins_attempt_selection() {
    let mut graph = SpawnGraph::default();
    let base = graph.begin_inventory("parent").unwrap();
    graph.record_runtime_evidence("parent", record(attempt(2), ChildEvidenceSource::Coordinator { version: base }, ChildEvidence::Nonterminal { phase: HistoricalChildPhase::Running })).unwrap();
    let current = graph.scopes["parent"].version();
    let mut runtime_identity = summary(2);
    runtime_identity.source = ChildEvidenceSource::Coordinator { version: current };
    runtime_identity.value = ChildEvidence::Identity { child_session_id: Some("child".into()), description: "actual worker".into(), subagent_type: "general".into(), started_at_epoch_ms: Some(1), persona: None, resumed_from: None, fork_parent_prompt_id: None, recorded_owner: None, recorded_spawner: None };
    graph.record_runtime_evidence("parent", runtime_identity).unwrap();
    let snapshot = graph.import_inventory(ChildInventoryImport { parent_session_id: "parent".into(), base, records: vec![summary(1), terminal(1, journal(20, "event.1"), RecordedTerminalState::Completed)], issues: vec![] }, &[]).unwrap();
    let child = inspection(&snapshot);
    assert_eq!(child.attempt, attempt(2));
    assert!(matches!(child.observation, ChildObservation::Uncustodied { .. }));
}

#[test]
fn failed_read_is_not_successful_empty_inventory_and_retry_is_explicit() {
    let mut graph = SpawnGraph::default();
    let base = graph.begin_inventory("parent").unwrap();
    let issue = InventoryIssue::Decode { source: InventoryReadSource::ParentJournal };
    let result = graph.import_inventory(ChildInventoryImport { parent_session_id: "parent".into(), base, records: vec![summary(1)], issues: vec![issue.clone()] }, &[]);
    assert!(matches!(result, Err(SubagentAccessError::InventoryUnreadable { issues }) if issues == vec![issue]));
    assert!(graph.contains("child"));
    assert!(matches!(graph.inventory_snapshot("parent", &[]), Err(SubagentAccessError::InventoryUnreadable { .. })));
    assert_eq!(import(&mut graph, vec![]).entries.len(), 1);
    let mut empty = SpawnGraph::default();
    assert!(import(&mut empty, vec![]).entries.is_empty());
}

#[test]
fn eviction_retains_identity_headers_but_not_output_and_release_fences_old_import() {
    let mut graph = SpawnGraph::default();
    import(&mut graph, vec![summary(1), terminal(1, journal(2, "event.1"), RecordedTerminalState::Completed)]);
    graph.remove("child");
    let snapshot = graph.inventory_snapshot("parent", &[]).unwrap();
    assert!(graph.contains("child"));
    assert!(matches!(&inspection(&snapshot).observation, ChildObservation::Terminal { terminal: ChildTerminalEvidence::Recorded { result } } if result.output.is_none() && result.duration_ms == Some(337393)));
    let old = graph.begin_inventory("parent").unwrap();
    let release = graph.begin_inventory_release("parent").unwrap();
    assert!(matches!(graph.begin_inventory("parent"), Err(SubagentAccessError::InventoryReleasing)));
    graph.release_inventory("parent", release).unwrap();
    assert!(!graph.contains("child"));
    let fresh = graph.begin_inventory("parent").unwrap();
    assert_ne!(fresh.incarnation, old.incarnation);
    assert!(matches!(graph.import_inventory(ChildInventoryImport { parent_session_id: "parent".into(), base: old, records: vec![summary(1)], issues: vec![] }, &[]), Err(SubagentAccessError::InventorySuperseded)));
}

#[test]
fn stale_proof_binds_attempt_and_existing_target_incarnation_not_global_sequence() {
    let mut graph = SpawnGraph::default();
    let snapshot = import(&mut graph, vec![summary(1)]);
    let target = InventoryTarget::Resident { target: ResidentChildTarget { attempt: attempt(1), incarnation: uuid::Uuid::new_v4() } };
    let proof = InventoryCancelProof { version: snapshot.version, target: target.clone() };
    let base = graph.begin_inventory("parent").unwrap();
    assert!(graph.matches_inventory_proof("parent", &proof, &target));
    let replacement = InventoryTarget::Resident { target: ResidentChildTarget { attempt: attempt(2), incarnation: uuid::Uuid::new_v4() } };
    assert!(!graph.matches_inventory_proof("parent", &proof, &replacement));
    let reincarnated = InventoryTarget::Resident { target: ResidentChildTarget { attempt: attempt(1), incarnation: uuid::Uuid::new_v4() } };
    assert!(!graph.matches_inventory_proof("parent", &proof, &reincarnated));
    let future = InventoryCancelProof { version: SourceVersion { sequence: base.sequence + 10, ..base }, target: target.clone() };
    assert!(!graph.matches_inventory_proof("parent", &future, &target));
}

#[test]
fn partial_summary_wire_requires_positive_matching_child_identity() {
    let value = serde_json::to_value(summary(1)).unwrap();
    assert_eq!(value["value"]["kind"], "summary_identity");
    assert!(value["value"].get("description").is_none());
    let mut bad = value;
    bad["value"]["childSessionId"] = json!("other");
    let record: ChildEvidenceRecord = serde_json::from_value(bad).unwrap();
    assert_eq!(record.validate(), Err(InventoryValidationError::IdentityMismatch));
}

fn identity(id: &str, spawner: Option<RecordedSubagentSpawner>) -> ChildEvidenceRecord {
    ChildEvidenceRecord {
        subagent_id: id.to_owned(), attempt: attempt(1),
        source: ChildEvidenceSource::ParentMeta { subagent_id: id.to_owned(), digest: "c".repeat(64) },
        value: ChildEvidence::Identity {
            child_session_id: Some(id.to_owned()), description: format!("description {id}"), subagent_type: "general".into(),
            started_at_epoch_ms: None, persona: None, resumed_from: None, fork_parent_prompt_id: None,
            recorded_owner: None, recorded_spawner: spawner,
        },
    }
}

fn child_spawner(id: &str) -> Option<RecordedSubagentSpawner> {
    Some(RecordedSubagentSpawner::Child { subagent_id: id.to_owned(), session_id: id.to_owned() })
}

#[test]
fn affirmative_imported_chains_bind_in_any_source_order_without_host_capabilities() {
    let records = vec![identity("a", Some(RecordedSubagentSpawner::Root)), identity("b", child_spawner("a")), identity("c", child_spawner("b")), identity("unknown", None), identity("leaf", child_spawner("unknown"))];
    for reverse in [false, true] {
        let mut records = records.clone();
        if reverse { records.reverse(); }
        let mut graph = SpawnGraph::default();
        import(&mut graph, records);
        assert!(graph.is_reachable_from("c", "parent"));
        assert!(graph.is_ancestor("c", "a"));
        assert!(graph.is_ancestor("c", "b"));
        assert!(!graph.is_ancestor("c", "c"));
        assert_eq!(graph.direct_spawner("c"), Some("b"));
        assert_eq!(graph.advertise_target("c"), None);
        assert_eq!(graph.surface_target("c"), None);
        assert!(!graph.is_ancestor("leaf", "unknown"));
        assert!(graph.is_reachable_from("leaf", "parent"));
    }
}

#[test]
fn invalid_imported_edges_are_atomic_in_both_orders() {
    let cases = vec![
        vec![identity("a", child_spawner("a"))],
        vec![identity("a", child_spawner("b")), identity("b", child_spawner("a"))],
        vec![identity("a", Some(RecordedSubagentSpawner::Root)), identity("b", Some(RecordedSubagentSpawner::Child { subagent_id: "a".into(), session_id: "wrong-session".into() }))],
        vec![identity("a", Some(RecordedSubagentSpawner::Root)), identity("b", Some(RecordedSubagentSpawner::Child { subagent_id: "a".into(), session_id: "parent".into() }))],
        vec![identity("parent", Some(RecordedSubagentSpawner::Root))],
    ];
    for records in cases {
        for reverse in [false, true] {
            let mut graph = SpawnGraph::default();
            import(&mut graph, vec![summary(1)]);
            let mut records = records.clone();
            records.push(identity("innocent", None));
            if reverse { records.reverse(); }
            let base = graph.begin_inventory("parent").unwrap();
            assert!(matches!(graph.import_inventory(ChildInventoryImport { parent_session_id: "parent".into(), base, records, issues: vec![] }, &[]), Err(SubagentAccessError::InvalidInventory { reason: InventoryValidationError::IdentityMismatch })));
            assert!(!graph.contains("innocent"));
            assert!(graph.contains("child"));
            assert_eq!(graph.inventory_source_version("parent").unwrap(), base);
        }
    }
}

#[test]
fn foreign_root_and_ancestor_session_aliases_cannot_create_audience() {
    let mut graph = SpawnGraph::default();
    graph.insert_root_child("foreign", "other-root");
    let base = graph.begin_inventory("parent").unwrap();
    let batch = ChildInventoryImport { parent_session_id: "parent".into(), base, records: vec![identity("leaf", child_spawner("foreign"))], issues: vec![] };
    assert!(matches!(graph.import_inventory(batch, &[]), Err(SubagentAccessError::InvalidInventory { .. })));
    assert!(!graph.contains("leaf"));
    let mut aliased = identity("b", child_spawner("a"));
    aliased.source = journal(2, "event.1");
    if let ChildEvidence::Identity { child_session_id, .. } = &mut aliased.value { *child_session_id = Some("a".into()); }
    let base = graph.begin_inventory("parent").unwrap();
    assert!(matches!(graph.import_inventory(ChildInventoryImport { parent_session_id: "parent".into(), base, records: vec![identity("a", Some(RecordedSubagentSpawner::Root)), aliased], issues: vec![] }, &[]), Err(SubagentAccessError::InvalidInventory { .. })));
    assert!(!graph.contains("a"));
}

#[test]
fn summary_and_full_identity_join_optional_fields_in_both_orders() {
    for reverse in [false, true] {
        let mut graph = SpawnGraph::default();
        let base = graph.begin_inventory("parent").unwrap();
        let mut full = identity("child", None);
        if let ChildEvidence::Identity { child_session_id, .. } = &mut full.value { *child_session_id = None; }
        if reverse {
            graph.import_inventory(ChildInventoryImport { parent_session_id: "parent".into(), base, records: vec![summary(1)], issues: vec![] }, &[]).unwrap();
        }
        full.source = ChildEvidenceSource::Coordinator { version: graph.inventory_source_version("parent").unwrap() };
        graph.record_runtime_evidence("parent", full).unwrap();
        let snapshot = if reverse {
            graph.inventory_snapshot("parent", &[]).unwrap()
        } else {
            graph.import_inventory(ChildInventoryImport { parent_session_id: "parent".into(), base, records: vec![summary(1)], issues: vec![] }, &[]).unwrap()
        };
        let child = inspection(&snapshot);
        assert_eq!(child.child_session_id.as_deref(), Some("child"));
        assert_eq!(child.fork_parent_prompt_id.as_deref(), Some("prompt"));
        assert_eq!(child.description.as_deref(), Some("description child"));
        assert_eq!(child.subagent_type.as_deref(), Some("general"));
        assert_eq!(child.recorded_owner, None);
        assert!(matches!(child.observation, ChildObservation::IdentityOnly { .. }));
    }
}

#[test]
fn full_summary_and_summary_summary_identity_contradictions_are_explicit() {
    let mut prompt_conflict = identity("child", None);
    if let ChildEvidence::Identity { fork_parent_prompt_id, .. } = &mut prompt_conflict.value { *fork_parent_prompt_id = Some("different-prompt".into()); }
    let mut child_conflict = identity("child", None);
    child_conflict.source = journal(3, "event.2");
    if let ChildEvidence::Identity { child_session_id, .. } = &mut child_conflict.value { *child_session_id = Some("different-session".into()); }
    let mut summary_conflict = summary(1);
    if let ChildEvidence::SummaryIdentity { fork_parent_prompt_id, .. } = &mut summary_conflict.value { *fork_parent_prompt_id = Some("other-prompt".into()); }
    for conflict in [prompt_conflict, child_conflict, summary_conflict] {
        for reverse in [false, true] {
            let mut records = vec![summary(1), conflict.clone()];
            if reverse { records.reverse(); }
            let snapshot = import(&mut SpawnGraph::default(), records);
            assert!(matches!(&snapshot.entries[0], ChildInventoryEntry::Conflict { issues, .. } if issues.iter().any(|issue| matches!(issue, InventoryIssue::IdentityMismatch { .. }))));
        }
    }
}

#[test]
fn recorded_terminal_fields_enrich_without_inventing_absent_values() {
    for legacy in [false, true] {
        for reverse in [false, true] {
            let mut a = terminal(1, journal(1, "event.9"), RecordedTerminalState::Cancelled { cause: None, message: Some(String::new()) });
            let mut b = terminal(1, journal(2, "event.1"), RecordedTerminalState::Cancelled { cause: None, message: None });
            if let ChildEvidence::Terminal { terminal: ChildTerminalEvidence::Recorded { result } } = &mut a.value { result.duration_ms = None; }
            if let ChildEvidence::Terminal { terminal: ChildTerminalEvidence::Recorded { result } } = &mut b.value { result.tokens_used = Some(0); }
            let mut identity = summary(1);
            if legacy {
                identity.attempt = AttemptEvidence::Legacy;
                a.attempt = AttemptEvidence::Legacy;
                b.attempt = AttemptEvidence::Legacy;
                for record in [&mut a, &mut b] {
                    if let ChildEvidence::Terminal { terminal: ChildTerminalEvidence::Recorded { result } } = &mut record.value { result.attempt = AttemptEvidence::Legacy; }
                }
            }
            let sources = [a.source.clone(), b.source.clone()];
            let mut records = vec![identity, a, b];
            if reverse { records.reverse(); }
            let snapshot = import(&mut SpawnGraph::default(), records);
            if legacy {
                assert!(matches!(&snapshot.entries[0], ChildInventoryEntry::Conflict { issues, .. } if issues.iter().any(|issue| matches!(issue, InventoryIssue::TerminalConflict { .. }))));
                continue;
            }
            match &inspection(&snapshot).observation {
                ChildObservation::Terminal { terminal: ChildTerminalEvidence::Recorded { result } } => {
                    assert_eq!(result.state, RecordedTerminalState::Cancelled { cause: None, message: Some(String::new()) });
                    assert_eq!(result.duration_ms, Some(337393));
                    assert_eq!(result.tokens_used, Some(0));
                    assert_eq!(result.output, None);
                    for source in sources { assert!(result.sources.contains(&source)); }
                }
                other => panic!("expected recorded cancellation, got {other:?}"),
            }
        }
    }
}

fn producer(n: u8, source: ChildEvidenceSource, recorded: Option<RecordedSubagentTerminal>) -> ChildEvidenceRecord {
    record(attempt(n), source.clone(), ChildEvidence::Terminal { terminal: ChildTerminalEvidence::ProducerFact {
        completion_id: xai_chat_state::SubagentCompletionId { subagent_id: "child".into(), attempt_id: attempt(n).known().unwrap().clone() },
        state: SubagentTerminalState::Completed, recorded, sources: vec![source],
    } })
}

fn recorded_result(record: &ChildEvidenceRecord) -> RecordedSubagentTerminal {
    match &record.value {
        ChildEvidence::Terminal { terminal: ChildTerminalEvidence::Recorded { result } } => result.clone(),
        other => panic!("expected recorded result, got {other:?}"),
    }
}

#[test]
fn producer_fact_keeps_all_compatible_provenance_and_real_headers_in_every_order() {
    let legacy = terminal(1, journal(1, "event.1"), RecordedTerminalState::Completed);
    let mut header = recorded_result(&legacy);
    header.duration_ms = None;
    header.tokens_used = Some(19);
    header.output = None;
    let sources = [journal(1, "event.1"), journal(2, "event.2"), journal(3, "event.3")];
    let records = [legacy, producer(1, sources[1].clone(), Some(header)), producer(1, sources[2].clone(), None)];
    for order in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
        let mut input = vec![summary(1)];
        input.extend(order.into_iter().map(|i| records[i].clone()));
        let snapshot = import(&mut SpawnGraph::default(), input);
        match &snapshot.entries[0] {
            ChildInventoryEntry::Child { inspection, issues, .. } => {
                assert!(issues.is_empty());
                match &inspection.observation {
                    ChildObservation::Terminal { terminal: ChildTerminalEvidence::ProducerFact { completion_id, state, recorded: Some(result), sources: actual } } => {
                        assert_eq!(completion_id.attempt_id, *attempt(1).known().unwrap());
                        assert_eq!(*state, SubagentTerminalState::Completed);
                        assert_eq!(result.duration_ms, Some(337393));
                        assert_eq!(result.tokens_used, Some(19));
                        assert_eq!(result.tool_calls, Some(0));
                        assert_eq!(result.output, None);
                        for source in &sources { assert!(actual.contains(source)); }
                    }
                    other => panic!("expected producer fact, got {other:?}"),
                }
            }
            other => panic!("expected producer child, got {other:?}"),
        }
    }
}

#[test]
fn contradictory_legacy_cannot_erase_the_original_producer_fact_or_headers() {
    let original = terminal(1, journal(1, "event.1"), RecordedTerminalState::Completed);
    let header = recorded_result(&original);
    let fact = producer(1, journal(2, "event.2"), Some(header));
    let mut conflicting_header = original.clone();
    if let ChildEvidence::Terminal { terminal: ChildTerminalEvidence::Recorded { result } } = &mut conflicting_header.value { result.duration_ms = Some(7); }
    for contradiction in [terminal(1, journal(3, "event.3"), RecordedTerminalState::Cancelled { cause: None, message: None }), conflicting_header] {
        for reverse in [false, true] {
            let mut records = vec![summary(1), fact.clone(), contradiction.clone()];
            if reverse { records.reverse(); }
            let snapshot = import(&mut SpawnGraph::default(), records);
            match &snapshot.entries[0] {
                ChildInventoryEntry::Child { inspection, issues, .. } => {
                    assert!(issues.iter().any(|issue| matches!(issue, InventoryIssue::TerminalConflict { .. })));
                    assert!(matches!(&inspection.observation, ChildObservation::Terminal { terminal: ChildTerminalEvidence::ProducerFact { state: SubagentTerminalState::Completed, recorded: Some(result), .. } } if result.duration_ms == Some(337393) && result.tool_calls == Some(0)));
                }
                other => panic!("producer claim was erased: {other:?}"),
            }
        }
    }
}

#[test]
fn contradictory_producers_conflict_and_other_attempts_never_enrich_current_fact() {
    let a = producer(1, journal(1, "event.1"), None);
    let mut b = producer(1, journal(2, "event.2"), None);
    if let ChildEvidence::Terminal { terminal: ChildTerminalEvidence::ProducerFact { state, .. } } = &mut b.value { *state = SubagentTerminalState::Failed { message: "real failure".into() }; }
    for records in [vec![summary(1), a.clone(), b.clone()], vec![b, a, summary(1)]] {
        let snapshot = import(&mut SpawnGraph::default(), records);
        assert!(matches!(&snapshot.entries[0], ChildInventoryEntry::Conflict { issues, .. } if issues.iter().any(|issue| matches!(issue, InventoryIssue::TerminalConflict { .. }))));
    }
    let snapshot = import(&mut SpawnGraph::default(), vec![summary(2), producer(2, journal(4, "event.4"), None), terminal(1, journal(5, "event.5"), RecordedTerminalState::Completed)]);
    assert!(matches!(&inspection(&snapshot).observation, ChildObservation::Terminal { terminal: ChildTerminalEvidence::ProducerFact { recorded: None, sources, .. } } if sources == &vec![journal(4, "event.4")]));
}

#[test]
fn old_lineage_requires_real_runtime_capture_instead_of_empty_conflict() {
    let snapshot = import(&mut SpawnGraph::default(), vec![summary(1)]);
    let mut runtime = RuntimeInventory { inspection: inspection(&snapshot).clone(), control_owner: None };
    runtime.inspection.observation = ChildObservation::Resident {
        status: super::super::super::super::types::SubagentSnapshotStatus::Initializing,
        duration_ms: 29,
        target: ResidentChildTarget { attempt: attempt(1), incarnation: uuid::Uuid::new_v4() },
    };
    let mut graph = SpawnGraph::default();
    graph.insert_root_child("child", "parent");
    let base = graph.begin_inventory("parent").unwrap();
    graph.import_inventory(ChildInventoryImport { parent_session_id: "parent".into(), base, records: vec![], issues: vec![] }, std::slice::from_ref(&runtime)).unwrap();
    let expected = vec![InventoryIssue::RuntimeCaptureUnavailable { subagent_id: "child".into() }];
    assert!(matches!(graph.inventory_snapshot("parent", &[]), Err(SubagentAccessError::InventoryUnreadable { issues }) if issues == expected));
    assert!(matches!(graph.inventory_entry("parent", "child", None), Err(SubagentAccessError::InventoryUnreadable { issues }) if issues == expected));
    let entry = graph.inventory_entry("parent", "child", Some(&runtime)).unwrap().unwrap();
    assert!(matches!(entry, ChildInventoryEntry::Child { inspection, .. } if inspection == runtime.inspection));
    assert_eq!(graph.inventory_snapshot("parent", &[runtime]).unwrap().entries.len(), 1);
}

#[test]
fn nested_release_fences_drain_and_retains_root_siblings_and_their_evidence() {
    let mut graph = SpawnGraph::default();
    let before = import(&mut graph, vec![identity("a", Some(RecordedSubagentSpawner::Root)), identity("b", child_spawner("a")), identity("c", child_spawner("b")), identity("sibling", Some(RecordedSubagentSpawner::Root)), identity("unknown", None)]);
    let target = InventoryTarget::Historical { attempt: attempt(1), source: journal(1, "event.1") };
    let proof = InventoryCancelProof { version: before.version, target: target.clone() };
    let drain = graph.begin_inventory_subscope_release("parent", "a").unwrap();
    assert!(!graph.matches_inventory_proof("parent", &proof, &target));
    graph.insert_root_child("late-root-child", "parent");
    assert!(!graph.contains("late-root-child"));
    assert!(graph.insert_nested("late-nested-child", "parent", super::super::NestedSpawner { child_id: "a".into(), session_id: "a".into(), surface_completion: true }).is_err());
    assert!(!graph.contains("late-nested-child"));
    assert!(graph.contains("b"));
    assert!(graph.contains("c"));
    assert!(matches!(graph.begin_inventory("parent"), Err(SubagentAccessError::InventoryReleasing)));
    assert!(matches!(graph.release_inventory("parent", drain), Err(SubagentAccessError::InventorySuperseded)));
    assert!(matches!(graph.release_inventory_subscope("parent", "b", drain), Err(SubagentAccessError::InventorySuperseded)));
    assert!(matches!(graph.release_inventory_subscope("parent", "a", before.version), Err(SubagentAccessError::InventorySuperseded)));
    graph.release_inventory_subscope("parent", "a", drain).unwrap();
    let after = graph.inventory_snapshot("parent", &[]).unwrap();
    assert_ne!(before.version.incarnation, after.version.incarnation);
    assert_eq!(after.entries, before.entries.into_iter().filter(|entry| matches!(entry.subagent_id(), "a" | "sibling" | "unknown")).collect::<Vec<_>>());
    assert!(!graph.contains("b"));
    assert!(!graph.contains("c"));
    assert!(!graph.matches_inventory_proof("parent", &proof, &target));
    let base = graph.begin_inventory("parent").unwrap();
    assert!(matches!(graph.import_inventory(ChildInventoryImport { parent_session_id: "parent".into(), base: before.version, records: vec![identity("b", child_spawner("a"))], issues: vec![] }, &[]), Err(SubagentAccessError::InventorySuperseded)));
    assert_eq!(graph.inventory_source_version("parent").unwrap(), base);
}

#[test]
fn sequence_overflow_mints_fresh_incarnation_and_invalidates_old_proofs_and_reads() {
    let mut graph = SpawnGraph::default();
    let snapshot = import(&mut graph, vec![summary(1)]);
    let old = SourceVersion { incarnation: snapshot.version.incarnation, sequence: u64::MAX };
    graph.scopes.insert("parent".into(), InventoryScope::Ready { version: old });
    let target = InventoryTarget::Historical { attempt: attempt(1), source: journal(1, "event.1") };
    let proof = InventoryCancelProof { version: old, target: target.clone() };
    let fresh = graph.begin_inventory("parent").unwrap();
    assert_ne!(fresh.incarnation, old.incarnation);
    assert_eq!(fresh.sequence, 0);
    assert!(!graph.matches_inventory_proof("parent", &proof, &target));
    assert!(matches!(graph.import_inventory(ChildInventoryImport { parent_session_id: "parent".into(), base: old, records: vec![], issues: vec![] }, &[]), Err(SubagentAccessError::InventorySuperseded)));
    assert!(graph.contains("child"));
}

#[test]
fn cause_and_message_enrich_independently_but_real_contradictions_conflict() {
    use super::super::super::super::types::InterruptionCause;
    let sparse = terminal(1, journal(1, "event.1"), RecordedTerminalState::Cancelled { cause: None, message: Some("actual message".into()) });
    let cause = terminal(1, journal(2, "event.2"), RecordedTerminalState::Cancelled { cause: Some(InterruptionCause::ProcessRestart), message: None });
    for records in [vec![summary(1), sparse.clone(), cause.clone()], vec![cause.clone(), sparse, summary(1)]] {
        let snapshot = import(&mut SpawnGraph::default(), records);
        assert!(matches!(&inspection(&snapshot).observation, ChildObservation::Terminal { terminal: ChildTerminalEvidence::Recorded { result } } if result.state == RecordedTerminalState::Cancelled { cause: Some(InterruptionCause::ProcessRestart), message: Some("actual message".into()) }));
    }
    let conflicts = [
        terminal(1, journal(3, "event.3"), RecordedTerminalState::Cancelled { cause: Some(InterruptionCause::LiveParentOrphan), message: None }),
        terminal(1, journal(4, "event.4"), RecordedTerminalState::Failed { message: Some("failure".into()) }),
    ];
    for conflict in conflicts {
        for records in [vec![summary(1), cause.clone(), conflict.clone()], vec![conflict, cause.clone(), summary(1)]] {
            let snapshot = import(&mut SpawnGraph::default(), records);
            assert!(matches!(&snapshot.entries[0], ChildInventoryEntry::Conflict { issues, .. } if issues.iter().any(|issue| matches!(issue, InventoryIssue::TerminalConflict { .. }))));
        }
    }
    let a = terminal(1, journal(5, "event.5"), RecordedTerminalState::Failed { message: Some("a".into()) });
    let b = terminal(1, journal(6, "event.6"), RecordedTerminalState::Failed { message: Some("b".into()) });
    for records in [vec![summary(1), a.clone(), b.clone()], vec![b, a, summary(1)]] {
        let snapshot = import(&mut SpawnGraph::default(), records);
        assert!(matches!(snapshot.entries[0], ChildInventoryEntry::Conflict { .. }));
    }
}

fn full_producer(source: ChildEvidenceSource, output: &str, tokens_used: u64) -> ChildEvidenceRecord {
    record(attempt(1), source.clone(), ChildEvidence::Terminal { terminal: ChildTerminalEvidence::Producer {
        completion_id: xai_chat_state::SubagentCompletionId { subagent_id: "child".into(), attempt_id: attempt(1).known().unwrap().clone() },
        result: super::super::super::super::terminal::SubagentTerminal {
            attempt_id: attempt(1).known().unwrap().as_str().to_owned(), state: SubagentTerminalState::Completed,
            output: output.into(), tool_calls: 3, turns: 2, duration_ms: 81, tokens_used,
        }, sources: vec![source],
    } })
}

#[test]
fn full_producer_headers_survive_compaction_and_payload_disagreements_remain_conflicts() {
    let a = full_producer(journal(1, "event.1"), "real complete output", 47);
    let b = full_producer(journal(2, "event.2"), "real complete output", 47);
    for records in [vec![summary(1), a.clone(), b.clone()], vec![b, a.clone(), summary(1)]] {
        let mut graph = SpawnGraph::default();
        let snapshot = import(&mut graph, records);
        assert!(matches!(&inspection(&snapshot).observation, ChildObservation::Terminal { terminal: ChildTerminalEvidence::ProducerFact { recorded: Some(result), sources, .. } } if result.tool_calls == Some(3) && result.turns == Some(2) && result.duration_ms == Some(81) && result.tokens_used == Some(47) && result.output.is_none() && sources.contains(&journal(1, "event.1")) && sources.contains(&journal(2, "event.2"))));
        graph.remove("child");
        assert!(graph.contains("child"));
        assert!(graph.inventory_entry("parent", "child", None).unwrap().is_some());
    }
    for contradiction in [full_producer(journal(2, "event.2"), "different output", 47), full_producer(journal(2, "event.2"), "real complete output", 48)] {
        for records in [vec![summary(1), a.clone(), contradiction.clone()], vec![contradiction, a.clone(), summary(1)]] {
            let snapshot = import(&mut SpawnGraph::default(), records);
            assert!(matches!(&snapshot.entries[0], ChildInventoryEntry::Conflict { issues, .. } if issues.iter().any(|issue| matches!(issue, InventoryIssue::TerminalConflict { .. }))));
        }
    }
}

#[test]
fn unknown_attempt_never_enriches_a_known_producer() {
    let mut unknown = terminal(1, journal(1, "event.1"), RecordedTerminalState::Completed);
    unknown.attempt = AttemptEvidence::Legacy;
    if let ChildEvidence::Terminal { terminal: ChildTerminalEvidence::Recorded { result } } = &mut unknown.value { result.attempt = AttemptEvidence::Legacy; }
    for records in [vec![summary(1), producer(1, journal(2, "event.2"), None), unknown.clone()], vec![unknown, producer(1, journal(2, "event.2"), None), summary(1)]] {
        let snapshot = import(&mut SpawnGraph::default(), records);
        assert!(matches!(&inspection(&snapshot).observation, ChildObservation::Terminal { terminal: ChildTerminalEvidence::ProducerFact { recorded: None, sources, .. } } if sources == &vec![journal(2, "event.2")]));
    }
}

#[test]
fn missing_ancestor_evidence_stays_root_only_until_real_chain_arrives() {
    let mut graph = SpawnGraph::default();
    import(&mut graph, vec![identity("leaf", child_spawner("middle"))]);
    assert!(graph.is_reachable_from("leaf", "parent"));
    assert!(!graph.is_reachable_from("leaf", "middle"));
    import(&mut graph, vec![identity("middle", Some(RecordedSubagentSpawner::Root))]);
    assert_eq!(graph.direct_spawner("leaf"), Some("middle"));
    assert!(graph.is_reachable_from("leaf", "middle"));
    assert_eq!(graph.advertise_target("leaf"), None);
}

#[test]
fn interrupted_producer_does_not_invent_a_recorded_cause_and_keeps_conflicting_legacy_claims() {
    use super::super::super::super::types::InterruptionCause;
    let actual = InterruptionCause::Error { message: "producer error".into() };
    let mut fact = producer(1, journal(1, "event.1"), None);
    if let ChildEvidence::Terminal { terminal: ChildTerminalEvidence::ProducerFact { state, .. } } = &mut fact.value { *state = SubagentTerminalState::Interrupted { cause: actual.clone() }; }
    let partial = terminal(1, journal(2, "event.2"), RecordedTerminalState::Cancelled { cause: None, message: Some("stored text".into()) });
    for records in [vec![summary(1), fact.clone(), partial.clone()], vec![partial, fact.clone(), summary(1)]] {
        let snapshot = import(&mut SpawnGraph::default(), records);
        match &inspection(&snapshot).observation {
            ChildObservation::Terminal { terminal: ChildTerminalEvidence::ProducerFact { state, recorded: Some(result), sources, .. } } => {
                assert_eq!(*state, SubagentTerminalState::Interrupted { cause: actual.clone() });
                assert_eq!(result.state, RecordedTerminalState::Cancelled { cause: None, message: Some("stored text".into()) });
                assert_eq!(sources, &vec![journal(1, "event.1"), journal(2, "event.2")]);
            }
            other => panic!("expected interrupted producer with partial recorded fields, got {other:?}"),
        }
    }
    let contradiction = terminal(1, journal(3, "event.3"), RecordedTerminalState::Cancelled { cause: Some(InterruptionCause::ProcessRestart), message: None });
    for records in [vec![summary(1), fact.clone(), contradiction.clone()], vec![contradiction, fact, summary(1)]] {
        let mut graph = SpawnGraph::default();
        let snapshot = import(&mut graph, records);
        assert!(matches!(&snapshot.entries[0], ChildInventoryEntry::Child { inspection, issues, .. } if matches!(&inspection.observation, ChildObservation::Terminal { terminal: ChildTerminalEvidence::ProducerFact { state: SubagentTerminalState::Interrupted { cause }, recorded: None, .. } } if cause == &actual) && issues.iter().any(|issue| matches!(issue, InventoryIssue::TerminalConflict { .. }))));
        assert_eq!(graph.nodes["child"].records.len(), 3);
    }
}

#[test]
fn runtime_overflow_during_loading_supersedes_the_original_import_base() {
    let mut graph = SpawnGraph::default();
    let base = SourceVersion { incarnation: uuid::Uuid::new_v4(), sequence: u64::MAX - 1 };
    let current = SourceVersion { sequence: u64::MAX, ..base };
    graph.scopes.insert("parent".into(), InventoryScope::Loading { base, version: current });
    let fresh = graph.record_runtime_evidence("parent", record(attempt(2), ChildEvidenceSource::Coordinator { version: current }, ChildEvidence::Nonterminal { phase: HistoricalChildPhase::Running })).unwrap();
    assert_eq!(fresh.sequence, 0);
    assert_ne!(fresh.incarnation, base.incarnation);
    assert!(matches!(graph.import_inventory(ChildInventoryImport { parent_session_id: "parent".into(), base, records: vec![summary(1)], issues: vec![] }, &[]), Err(SubagentAccessError::InventorySuperseded)));
    assert_eq!(graph.nodes["child"].records.len(), 1);
}

#[test]
fn coordinator_source_cannot_be_smuggled_through_a_historical_import() {
    let mut graph = SpawnGraph::default();
    let base = graph.begin_inventory("parent").unwrap();
    let mut forged = identity("child", Some(RecordedSubagentSpawner::Root));
    forged.source = ChildEvidenceSource::Coordinator { version: base };
    assert!(matches!(graph.import_inventory(ChildInventoryImport { parent_session_id: "parent".into(), base, records: vec![forged], issues: vec![] }, &[]), Err(SubagentAccessError::InvalidInventory { reason: InventoryValidationError::InvalidSource })));
    assert!(!graph.contains("child"));
    assert_eq!(graph.inventory_source_version("parent").unwrap(), base);
}

#[test]
fn absent_failed_message_enriches_to_a_real_empty_message_in_both_orders() {
    let a = terminal(1, journal(1, "event.1"), RecordedTerminalState::Failed { message: None });
    let b = terminal(1, journal(2, "event.2"), RecordedTerminalState::Failed { message: Some(String::new()) });
    for records in [vec![summary(1), a.clone(), b.clone()], vec![b, a, summary(1)]] {
        let snapshot = import(&mut SpawnGraph::default(), records);
        assert!(matches!(&inspection(&snapshot).observation, ChildObservation::Terminal { terminal: ChildTerminalEvidence::Recorded { result } } if result.state == RecordedTerminalState::Failed { message: Some(String::new()) }));
    }
}

#[test]
fn conflicting_weak_headers_do_not_select_an_arbitrary_producer_enrichment() {
    let a = terminal(1, journal(1, "event.1"), RecordedTerminalState::Completed);
    let mut b = terminal(1, journal(2, "event.2"), RecordedTerminalState::Completed);
    if let ChildEvidence::Terminal { terminal: ChildTerminalEvidence::Recorded { result } } = &mut b.value { result.duration_ms = Some(99); }
    for records in [vec![summary(1), a.clone(), producer(1, journal(3, "event.3"), None), b.clone()], vec![b, producer(1, journal(3, "event.3"), None), a, summary(1)]] {
        let snapshot = import(&mut SpawnGraph::default(), records);
        assert!(matches!(&snapshot.entries[0], ChildInventoryEntry::Child { inspection, issues, .. } if matches!(&inspection.observation, ChildObservation::Terminal { terminal: ChildTerminalEvidence::ProducerFact { recorded: None, .. } }) && issues.iter().any(|issue| matches!(issue, InventoryIssue::TerminalConflict { .. }))));
    }
}
