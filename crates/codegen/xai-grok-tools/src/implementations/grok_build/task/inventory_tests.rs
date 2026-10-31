use super::*;
use super::super::terminal::{
    ChildTerminalEvidence, RecordedSubagentTerminal, RecordedTerminalState, SubagentTerminal,
    SubagentTerminalState,
};
use super::super::types::{ChildObservation, SubagentCancelBinding, SubagentState};

const CHILD: &str = "01a184d2-c4ef-7213-aec5-9b435d59bfa2";
const PARENT: &str = "01a17d8b-5982-77e2-8445-eb654bb66440";
const ATTEMPT: &str = "at1.62d39fc4b9924949b0ed8a0f259d47ec";

fn attempt() -> AttemptEvidence {
    AttemptEvidence::from_recorded(Some(ATTEMPT)).unwrap()
}

fn source() -> ChildEvidenceSource {
    ChildEvidenceSource::ParentMeta {
        subagent_id: CHILD.into(),
        digest: "a".repeat(64),
    }
}

fn recorded() -> RecordedSubagentTerminal {
    RecordedSubagentTerminal {
        attempt: attempt(),
        state: RecordedTerminalState::Cancelled {
            cause: None,
            message: Some("interrupted by process restart".into()),
        },
        completed_at: None,
        duration_ms: Some(337393),
        tool_calls: Some(0),
        turns: Some(0),
        tokens_used: None,
        output: None,
        sources: vec![source()],
    }
}

fn inspection(observation: ChildObservation) -> SubagentInspection {
    SubagentInspection {
        subagent_id: CHILD.into(),
        description: Some("Repair GitLab backup path contract".into()),
        subagent_type: Some("general-purpose".into()),
        started_at_epoch_ms: Some(1793229767932),
        persona: None,
        parent_session_id: PARENT.into(),
        child_session_id: Some(CHILD.into()),
        fork_parent_prompt_id: None,
        resumed_from: None,
        attempt: attempt(),
        recorded_owner: None,
        recorded_spawner: None,
        observation,
    }
}

fn version() -> SourceVersion {
    SourceVersion {
        incarnation: uuid::Uuid::parse_str("01a17e12-b8e1-7601-9816-4f32046b2f5b").unwrap(),
        sequence: 17,
    }
}

#[test]
fn recorded_cancelled_keeps_unknown_cause_and_real_zero_counters() {
    let terminal = ChildTerminalEvidence::Recorded { result: recorded() };
    terminal.validate(CHILD, &attempt()).unwrap();
    let value = serde_json::to_value(&terminal).unwrap();
    let result = value.get("result").unwrap();
    assert_eq!(value.get("evidence"), Some(&serde_json::json!("recorded")));
    assert_eq!(result.get("state"), Some(&serde_json::json!({
        "status": "cancelled",
        "message": "interrupted by process restart",
    })));
    assert_eq!(result.get("toolCalls"), Some(&serde_json::json!(0)));
    assert_eq!(result.get("turns"), Some(&serde_json::json!(0)));
    assert!(result.get("tokensUsed").is_none());
    assert!(result.get("output").is_none());
    assert_eq!(terminal.state(), None);
    assert_eq!(terminal.duration_ms(), Some(337393));
    assert_eq!(terminal.legacy_status(), "cancelled");
    assert_eq!(serde_json::from_value::<ChildTerminalEvidence>(value).unwrap(), terminal);
}

#[test]
fn terminal_without_a_snapshot_stays_an_inspectable_child_with_unknown_owner() {
    let child = inspection(ChildObservation::Terminal {
        terminal: ChildTerminalEvidence::Recorded { result: recorded() },
    });
    child.validate().unwrap();
    assert!(child.snapshot().is_none());
    let value = serde_json::to_value(&child).unwrap();
    assert_eq!(value.get("recordedOwner"), Some(&serde_json::Value::Null));
    assert_eq!(value.get("childSessionId"), Some(&serde_json::json!(CHILD)));
    assert_eq!(serde_json::from_value::<SubagentInspection>(value).unwrap(), child);
}

#[test]
fn missing_and_invalid_attempts_are_different() {
    assert_eq!(AttemptEvidence::from_recorded(None).unwrap(), AttemptEvidence::Legacy);
    assert!(AttemptEvidence::from_recorded(Some("running")).is_err());
    assert!(serde_json::from_value::<AttemptEvidence>(serde_json::json!({
        "kind": "known", "attemptId": "running",
    })).is_err());
    assert_eq!(serde_json::to_value(attempt()).unwrap(), serde_json::json!({
        "kind": "known", "attemptId": ATTEMPT,
    }));
    assert_eq!(serde_json::to_value(AttemptEvidence::Legacy).unwrap(), serde_json::json!({"kind":"legacy"}));
}

#[test]
fn historical_nonterminal_has_neither_current_state_nor_elapsed_duration() {
    let child = inspection(ChildObservation::Uncustodied {
        attempt: attempt(),
        last: HistoricalChildPhase::Running,
        sources: vec![source()],
    });
    child.validate().unwrap();
    assert_eq!(child.observation.state(), None);
    assert_eq!(child.observation.duration_ms(), None);
    assert!(child.into_snapshot().is_none());
}

#[test]
fn producer_fact_does_not_supply_a_full_result() {
    assert!(SubagentTerminalState::from_state(&SubagentState::Running).is_none());
    let terminal = ChildTerminalEvidence::ProducerFact {
        completion_id: SubagentCompletionId {
            subagent_id: CHILD.into(),
            attempt_id: AttemptId::parse(ATTEMPT).unwrap(),
        },
        state: SubagentTerminalState::Completed,
        recorded: None,
        sources: vec![source()],
    };
    terminal.validate(CHILD, &attempt()).unwrap();
    assert_eq!(terminal.state(), Some(SubagentState::Completed));
    assert_eq!(terminal.duration_ms(), None);
    assert!(terminal.snapshot_status().is_none());
    let value = serde_json::to_value(&terminal).unwrap();
    assert!(value.get("result").is_none());
    assert!(value.get("recorded").is_none());
    assert_eq!(serde_json::from_value::<ChildTerminalEvidence>(value).unwrap(), terminal);
}

#[test]
fn full_producer_wire_remains_strict_and_attempt_exact() {
    let result = SubagentTerminal {
        attempt_id: ATTEMPT.into(),
        state: SubagentTerminalState::Completed,
        output: std::sync::Arc::from("actual worker output"),
        tool_calls: 5,
        turns: 1,
        duration_ms: 120590,
        tokens_used: 19,
    };
    let expected = serde_json::json!({
        "attemptId": ATTEMPT, "state": {"status": "completed"},
        "output": "actual worker output", "toolCalls": 5, "turns": 1,
        "durationMs": 120590, "tokensUsed": 19,
    });
    assert_eq!(serde_json::to_value(&result).unwrap(), expected);
    for field in ["attemptId", "state", "output", "toolCalls", "turns", "durationMs", "tokensUsed"] {
        let mut incomplete = expected.clone();
        incomplete.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<SubagentTerminal>(incomplete).is_err());
    }
    let mut terminal = ChildTerminalEvidence::Producer {
        completion_id: SubagentCompletionId {
            subagent_id: CHILD.into(),
            attempt_id: AttemptId::parse(ATTEMPT).unwrap(),
        },
        result,
        sources: vec![source()],
    };
    terminal.validate(CHILD, &attempt()).unwrap();
    if let ChildTerminalEvidence::Producer { result, .. } = &mut terminal {
        result.attempt_id = "at1.1".into();
    }
    assert_eq!(terminal.validate(CHILD, &attempt()), Err(InventoryValidationError::AttemptMismatch));
}

#[test]
fn owner_wire_shape_does_not_default_legacy_to_task() {
    for (owner, wire) in [
        (RecordedSubagentOwner::Task, serde_json::json!({"kind":"task"})),
        (RecordedSubagentOwner::Workflow { run_id: "workflow-42".into() },
            serde_json::json!({"kind":"workflow","run_id":"workflow-42"})),
    ] {
        assert_eq!(serde_json::to_value(&owner).unwrap(), wire);
        assert_eq!(serde_json::from_value::<RecordedSubagentOwner>(wire).unwrap(), owner);
    }
    assert_eq!(serde_json::from_value::<Option<RecordedSubagentOwner>>(serde_json::Value::Null).unwrap(), None);
    assert!(serde_json::from_value::<RecordedSubagentOwner>(serde_json::json!({"kind":"workflow"})).is_err());
    assert!(serde_json::from_value::<RecordedSubagentOwner>(serde_json::json!({"kind":"legacy_task"})).is_err());
}

#[test]
fn observed_cancel_without_proof_never_becomes_current() {
    let binding: SubagentCancelBinding = serde_json::from_value(serde_json::json!({
        "kind": "observed", "attempt_id": ATTEMPT,
    })).unwrap();
    assert_eq!(binding.selector(CHILD).unwrap(), ChildCancelSelector::UnconfirmedObservation { attempt: attempt() });
    assert_eq!(SubagentCancelBinding::Current.selector(CHILD).unwrap(), ChildCancelSelector::Current);
    let proof = InventoryCancelProof {
        version: version(),
        target: InventoryTarget::Resident {
            target: ResidentChildTarget {
                attempt: attempt(),
                incarnation: version().incarnation,
            },
        },
    };
    let binding = SubagentCancelBinding::Observed {
        attempt_id: Some(ATTEMPT.into()),
        proof: Some(proof.clone()),
    };
    assert_eq!(binding.selector(CHILD).unwrap(), ChildCancelSelector::Bound(proof));
    let mismatched = SubagentCancelBinding::Observed {
        attempt_id: Some("at1.1".into()),
        proof: match binding {
            SubagentCancelBinding::Observed { proof, .. } => proof,
            SubagentCancelBinding::Current => None,
        },
    };
    assert_eq!(mismatched.selector(CHILD), Err(InventoryValidationError::AttemptMismatch));
}

#[test]
fn import_cannot_assert_coordinator_custody_or_hide_another_attempt() {
    let mut batch = ChildInventoryImport {
        parent_session_id: PARENT.into(),
        base: version(),
        records: vec![ChildEvidenceRecord {
            subagent_id: CHILD.into(),
            attempt: attempt(),
            source: ChildEvidenceSource::Coordinator { version: version() },
            value: ChildEvidence::Nonterminal { phase: HistoricalChildPhase::Running },
        }],
        issues: vec![],
    };
    assert_eq!(batch.validate(), Err(InventoryValidationError::RuntimeSourceInImport));
    let record = batch.records.first_mut().unwrap();
    record.source = source();
    record.value = ChildEvidence::Terminal {
        terminal: ChildTerminalEvidence::Recorded { result: recorded() },
    };
    batch.validate().unwrap();
    if let ChildEvidence::Terminal {
        terminal: ChildTerminalEvidence::Recorded { result },
    } = &mut batch.records.first_mut().unwrap().value {
        result.sources = vec![ChildEvidenceSource::Coordinator { version: version() }];
    }
    assert_eq!(batch.validate(), Err(InventoryValidationError::RuntimeSourceInImport));
    if let ChildEvidence::Terminal {
        terminal: ChildTerminalEvidence::Recorded { result },
    } = &mut batch.records.first_mut().unwrap().value {
        result.sources = vec![source()];
    }
    batch.records.first_mut().unwrap().attempt = AttemptEvidence::Legacy;
    assert_eq!(batch.validate(), Err(InventoryValidationError::AttemptMismatch));
}

#[test]
fn journal_locators_preserve_low_event_ids_without_ordering_them() {
    let source = ChildEvidenceSource::ParentJournal {
        line: 8407,
        byte_offset: 100,
        byte_len: 321,
        event_id: Some(format!("{PARENT}-38")),
        digest: "b".repeat(64),
    };
    source.validate_for(CHILD, &attempt()).unwrap();
    let wire = serde_json::to_value(&source).unwrap();
    assert_eq!(wire.get("eventId"), Some(&serde_json::json!(format!("{PARENT}-38"))));
    assert_eq!(wire.get("byteOffset"), Some(&serde_json::json!(100)));
    assert_eq!(serde_json::from_value::<ChildEvidenceSource>(wire).unwrap(), source);
}

#[test]
fn a_real_initializing_record_keeps_its_target_without_a_child_session() {
    let mut child = inspection(ChildObservation::Resident {
        status: super::super::types::SubagentSnapshotStatus::Initializing,
        duration_ms: 0,
        target: ResidentChildTarget { attempt: attempt(), incarnation: version().incarnation },
    });
    child.child_session_id = None;
    child.validate().unwrap();
    assert_eq!(child.observation.state(), Some(SubagentState::Running));
    assert_eq!(child.snapshot().unwrap().duration_ms, 0);
    let value = serde_json::to_value(&child).unwrap();
    assert!(value.get("childSessionId").is_none());
    assert_eq!(serde_json::from_value::<SubagentInspection>(value).unwrap(), child);
}

#[test]
fn an_unreadable_meta_needs_no_invented_digest_or_terminal() {
    let batch = ChildInventoryImport {
        parent_session_id: PARENT.into(),
        base: version(),
        records: vec![],
        issues: vec![InventoryIssue::Io {
            source: InventoryReadSource::ParentMeta { subagent_id: CHILD.into() },
            error_kind: InventoryIoKind::PermissionDenied,
        }],
    };
    batch.validate().unwrap();
    assert_eq!(serde_json::to_value(&batch.issues).unwrap(), serde_json::json!([{
        "kind": "io", "source": {"kind": "parent_meta", "subagentId": CHILD},
        "errorKind": "permission_denied",
    }]));
    assert!(batch.records.is_empty());
}

#[test]
fn references_keep_each_attempt_paired_with_its_own_source() {
    let records: Vec<_> = [ATTEMPT, "at1.1"].into_iter().enumerate().map(|(index, token)| {
        ChildEvidenceRecord {
            subagent_id: CHILD.into(),
            attempt: AttemptEvidence::from_recorded(Some(token)).unwrap(),
            source: ChildEvidenceSource::ParentJournal {
                line: index as u64 + 1, byte_offset: index as u64 * 100, byte_len: 100,
                event_id: None, digest: "c".repeat(64),
            },
            value: ChildEvidence::Nonterminal { phase: HistoricalChildPhase::Spawned },
        }
    }).collect();
    let entry = ChildInventoryEntry::Reference {
        subagent_id: CHILD.into(), child_session_id: Some(CHILD.into()), records,
    };
    entry.validate().unwrap();
    let value = serde_json::to_value(&entry).unwrap();
    let roundtrip: ChildInventoryEntry = serde_json::from_value(value).unwrap();
    assert_eq!(roundtrip, entry);
}

#[test]
fn no_start_is_positive_runtime_evidence_not_a_missing_historical_attempt() {
    assert_eq!(AttemptEvidence::from_recorded(None).unwrap(), AttemptEvidence::Legacy);
    assert_eq!(serde_json::to_value(AttemptEvidence::NotStarted).unwrap(), serde_json::json!({"kind":"not_started"}));
    assert_ne!(AttemptEvidence::NotStarted, AttemptEvidence::Legacy);
    let sources = [
        source(),
        ChildEvidenceSource::ParentJournal { line: 1, byte_offset: 0, byte_len: 1, event_id: None, digest: "a".repeat(64) },
        ChildEvidenceSource::ParentSummary {
            digest: "a".repeat(64),
            completion_id: SubagentCompletionId { subagent_id: CHILD.into(), attempt_id: attempt().known().unwrap().clone() },
        },
        ChildEvidenceSource::ChildSummary { child_session_id: CHILD.into(), digest: "a".repeat(64) },
    ];
    for source in sources {
        assert_eq!(source.validate_for(CHILD, &AttemptEvidence::NotStarted), Err(InventoryValidationError::InvalidAttempt));
        source.validate_for(CHILD, &attempt()).unwrap();
    }
    ChildEvidenceSource::Coordinator { version: version() }.validate_for(CHILD, &AttemptEvidence::NotStarted).unwrap();
}

#[test]
fn only_an_explicit_no_start_proof_can_bind_a_queue_without_an_attempt_id() {
    let proof = InventoryCancelProof {
        version: version(),
        target: InventoryTarget::Resident { target: ResidentChildTarget {
            attempt: AttemptEvidence::NotStarted, incarnation: version().incarnation,
        } },
    };
    assert_eq!(SubagentCancelBinding::Observed {
        attempt_id: None, proof: Some(proof.clone()),
    }.selector(CHILD).unwrap(), ChildCancelSelector::Bound(proof.clone()));
    assert_eq!(SubagentCancelBinding::Observed {
        attempt_id: Some(ATTEMPT.into()), proof: Some(proof),
    }.selector(CHILD), Err(InventoryValidationError::AttemptMismatch));
    assert_eq!(SubagentCancelBinding::Observed {
        attempt_id: None, proof: None,
    }.selector(CHILD).unwrap(), ChildCancelSelector::UnconfirmedObservation { attempt: AttemptEvidence::Legacy });
}
