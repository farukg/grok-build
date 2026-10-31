use super::*;
use crate::session::storage::prepare_replay_lines;

const CHILD: &str = "01a184d2-c4ef-7213-aec5-9b435d59bfa2";
const ATTEMPT: &str = "at1.62d39fc4b9924949b0ed8a0f259d47ec";

fn finish(extra: &str) -> String {
    format!(r#"{{"sessionId":"parent","update":{{"sessionUpdate":"subagent_finished","subagent_id":"{CHILD}","attempt_id":"{ATTEMPT}","child_session_id":"{CHILD}","status":"cancelled","error":"interrupted by process restart","tool_calls":0,"turns":0,"duration_ms":337393{extra}}},"_meta":{{"eventId":"parent-38"}}}}"#)
}

#[test]
fn legacy_finish_preserves_absence_zero_and_original_locator_before_cursor() {
    let line = finish("");
    let contents = format!("\n{line}\r\n");
    let prepared = prepare_replay_lines(&contents, Some("parent-38"));
    assert!(prepared.lines.is_empty());
    assert!(prepared.child_evidence_issues.is_empty());
    let record = &prepared.child_evidence[0];
    assert!(matches!(&record.source, ChildEvidenceSource::ParentJournal {
        line: 2, byte_offset: 1, byte_len, event_id: Some(event_id), ..
    } if *byte_len == line.len() as u64 && event_id == "parent-38"));
    let ChildEvidence::Terminal { terminal: ChildTerminalEvidence::Recorded { result } } = &record.value else {
        panic!("legacy finish must remain recorded");
    };
    assert!(matches!(&result.state, RecordedTerminalState::Cancelled { cause: None, .. }));
    assert_eq!(result.duration_ms, Some(337393));
    assert_eq!(result.tool_calls, Some(0));
    assert_eq!(result.turns, Some(0));
    assert_eq!(result.tokens_used, None);
    assert_eq!(result.output, None);
    assert_eq!(prepared.max_event_seq, Some(38));
}

#[test]
fn explicit_empty_output_and_zero_tokens_are_reported_values() {
    let contents = finish(r#", "tokens_used":0,"output":"""#);
    let prepared = prepare_replay_lines(&contents, None);
    let ChildEvidence::Terminal { terminal: ChildTerminalEvidence::Recorded { result } } = &prepared.child_evidence[0].value else {
        panic!("recorded terminal");
    };
    assert_eq!(result.tokens_used, Some(0));
    assert_eq!(result.output.as_deref(), Some(""));
}

#[tokio::test]
async fn point_read_rejects_changed_bytes_without_mutating_source() {
    let dir = tempfile::tempdir().unwrap();
    let contents = finish("");
    let path = dir.path().join("updates.jsonl");
    std::fs::write(&path, &contents).unwrap();
    let prepared = prepare_replay_lines(&contents, None);
    let record = &prepared.child_evidence[0];
    let terminal = load_inventory_terminal(dir.path(), "parent", CHILD, &record.attempt, &record.source).await.unwrap();
    assert!(matches!(terminal, Some(ChildTerminalEvidence::Recorded { .. })));
    let changed = contents.replace("337393", "337394");
    std::fs::write(&path, &changed).unwrap();
    assert!(matches!(load_inventory_terminal(dir.path(), "parent", CHILD, &record.attempt, &record.source).await,
        Err(SubagentAccessError::InventoryChanged)));
    assert_eq!(std::fs::read_to_string(path).unwrap(), changed);
}

#[tokio::test]
async fn missing_directory_is_empty_but_broken_metadata_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let base = SourceVersion { incarnation: uuid::Uuid::new_v4(), sequence: 0 };
    let empty = read_parent_inventory(dir.path(), "parent", base, Vec::new(), Vec::new()).await;
    assert!(empty.records.is_empty());
    assert!(empty.issues.is_empty());
    let child = dir.path().join("subagents").join(CHILD);
    std::fs::create_dir_all(&child).unwrap();
    std::fs::write(child.join("meta.json"), b"{").unwrap();
    let broken = read_parent_inventory(dir.path(), "parent", base, Vec::new(), Vec::new()).await;
    assert!(broken.records.is_empty());
    assert!(matches!(&broken.issues[..], [InventoryIssue::Decode { source: InventoryReadSource::ParentMeta { subagent_id } }] if subagent_id == CHILD));
    assert_eq!(std::fs::read(child.join("meta.json")).unwrap(), b"{");
}

#[test]
fn producer_fact_needs_original_complete_fields_to_become_full() {
    let fact = format!(r#""subagentCompletion":{{"id":{{"subagent_id":"{CHILD}","attempt_id":"{ATTEMPT}"}},"state":{{"status":"interrupted","cause":{{"kind":"error","message":"producer error"}}}}}},"eventId":"#);
    let partial = finish("").replace(r#""eventId":"#, &fact);
    let prepared = prepare_replay_lines(&partial, None);
    assert!(prepared.child_evidence_issues.is_empty());
    assert!(matches!(&prepared.child_evidence[0].value, ChildEvidence::Terminal {
        terminal: ChildTerminalEvidence::ProducerFact { recorded: Some(recorded), .. }
    } if recorded.tokens_used.is_none() && recorded.output.is_none()));
    let full = finish(r#", "tokens_used":0,"output":"""#).replace(r#""eventId":"#, &fact);
    let prepared = prepare_replay_lines(&full, None);
    assert!(prepared.child_evidence_issues.is_empty());
    assert!(matches!(&prepared.child_evidence[0].value, ChildEvidence::Terminal {
        terminal: ChildTerminalEvidence::Producer { result, .. }
    } if result.output.is_empty() && result.tokens_used == 0 && result.duration_ms == 337393));
}

#[test]
fn malformed_explicit_attempt_is_not_legacy() {
    let contents = finish("").replace(ATTEMPT, "broken");
    let prepared = prepare_replay_lines(&contents, None);
    assert!(prepared.child_evidence.is_empty());
    assert!(matches!(&prepared.child_evidence_issues[..], [InventoryIssue::Decode { source: InventoryReadSource::ParentJournal }]));
}

fn summary_bytes(obligations: &str) -> Vec<u8> {
    format!(r#"{{"info":{{"id":"parent","cwd":"/fixture"}},"completion_delivery":{{"obligations":[{obligations}]}}}}"#).into_bytes()
}

fn preparing_event() -> String {
    format!(r#"{{ "id":{{"subagent_id":"{CHILD}","attempt_id":"{ATTEMPT}"}},"state":{{"status":"completed"}},"text":"notice","output":"worker\noutput","child_session_id":"{CHILD}","tool_calls":5,"turns":1,"duration_ms":120590,"tokens_used":17,"completed_at":"2026-10-27T15:57:19Z","policy":"automatic","artifacts":{{"kind":"none"}} }}"#)
}

fn preparing_source(event: &str) -> ChildEvidenceSource {
    ChildEvidenceSource::ParentSummary {
        digest: digest(event.as_bytes()),
        completion_id: xai_chat_state::SubagentCompletionId {
            subagent_id: CHILD.into(),
            attempt_id: xai_message_delivery_core::AttemptId::parse(ATTEMPT).unwrap(),
        },
    }
}

#[test]
fn preparing_point_read_decodes_only_selected_payload_and_hashes_embedded_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let event = preparing_event();
    let source = preparing_source(&event);
    let attempt = AttemptEvidence::from_recorded(Some(ATTEMPT)).unwrap();
    // Valid JSON, but not a decodable completion event.
    let unrelated = r#"{"phase":"preparing","event":{"output":{"not":"a string"}},"admission":{"stage":"pending"}}"#;
    let selected = format!(r#"{{"admission":{{"stage":"pending"}},"event":{event},"phase":"preparing"}}"#);
    let bytes = summary_bytes(&format!("{unrelated},{selected}"));
    let path = dir.path().join("summary.json");
    std::fs::write(&path, &bytes).unwrap();
    assert!(parent_summary_records(&bytes, "parent").is_err());
    let terminal = load_inventory_terminal_at(dir.path(), "parent", CHILD, &attempt, &source).unwrap();
    assert!(matches!(terminal, Some(ChildTerminalEvidence::Producer { result, sources, .. })
        if result.output.as_ref() == "worker\noutput" && result.tokens_used == 17
        && result.tool_calls == 5 && result.duration_ms == 120590 && sources == vec![source.clone()]));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    // Enclosing changes do not change the embedded event digest.
    let changed = String::from_utf8(bytes).unwrap().replace("/fixture", "/elsewhere");
    std::fs::write(&path, &changed).unwrap();
    assert!(load_inventory_terminal_at(dir.path(), "parent", CHILD, &attempt, &source).unwrap().is_some());
    let changed_event = changed.replace("worker\\noutput", "changed output");
    std::fs::write(&path, &changed_event).unwrap();
    assert!(matches!(load_inventory_terminal_at(dir.path(), "parent", CHILD, &attempt, &source),
        Err(SubagentAccessError::InventoryChanged)));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), changed_event);
}

#[test]
fn preparing_point_read_preserves_selected_decode_identity_and_attempt_errors() {
    let dir = tempfile::tempdir().unwrap();
    let event = preparing_event();
    let source = preparing_source(&event);
    let attempt = AttemptEvidence::from_recorded(Some(ATTEMPT)).unwrap();
    let write = |event: &str| {
        std::fs::write(dir.path().join("summary.json"), summary_bytes(&format!(
            r#"{{"phase":"preparing","event":{event},"admission":{{"stage":"pending"}}}}"#))).unwrap();
    };
    write(&event);
    assert!(matches!(load_inventory_terminal_at(dir.path(), "foreign", CHILD, &attempt, &source),
        Err(SubagentAccessError::InvalidInventory { reason: InventoryValidationError::IdentityMismatch })));
    let other = AttemptEvidence::from_recorded(Some("at1.3472966983e64bab8708952207a6fc92")).unwrap();
    assert!(matches!(load_inventory_terminal_at(dir.path(), "parent", CHILD, &other, &source),
        Err(SubagentAccessError::InvalidInventory { reason: InventoryValidationError::AttemptMismatch })));
    let malformed = event.replace(ATTEMPT, "broken");
    write(&malformed);
    let mut malformed_source = source.clone();
    if let ChildEvidenceSource::ParentSummary { digest: value, .. } = &mut malformed_source {
        *value = digest(malformed.as_bytes());
    }
    assert!(matches!(load_inventory_terminal_at(dir.path(), "parent", CHILD, &attempt, &malformed_source),
        Err(SubagentAccessError::InvalidInventory { reason: InventoryValidationError::InvalidEvidence })));
    std::fs::write(dir.path().join("summary.json"), summary_bytes("")).unwrap();
    assert!(matches!(load_inventory_terminal_at(dir.path(), "parent", CHILD, &attempt, &source),
        Err(SubagentAccessError::InventoryChanged)));
}

#[test]
fn journal_point_read_rejects_malformed_stale_and_mismatched_locators() {
    let dir = tempfile::tempdir().unwrap();
    let contents = finish("");
    std::fs::write(dir.path().join("updates.jsonl"), &contents).unwrap();
    let prepared = prepare_replay_lines(&contents, None);
    let record = &prepared.child_evidence[0];
    let mut malformed = record.source.clone();
    if let ChildEvidenceSource::ParentJournal { byte_len, .. } = &mut malformed { *byte_len = 0; }
    assert!(matches!(load_inventory_terminal_at(dir.path(), "parent", CHILD, &record.attempt, &malformed),
        Err(SubagentAccessError::InvalidInventory { reason: InventoryValidationError::InvalidSource })));
    let mut stale = record.source.clone();
    if let ChildEvidenceSource::ParentJournal { byte_offset, .. } = &mut stale { *byte_offset += 1; }
    assert!(matches!(load_inventory_terminal_at(dir.path(), "parent", CHILD, &record.attempt, &stale),
        Err(SubagentAccessError::InventoryUnreadable { .. })));
    let mut wrong_event = record.source.clone();
    if let ChildEvidenceSource::ParentJournal { event_id, .. } = &mut wrong_event { *event_id = Some("parent-39".into()); }
    assert!(matches!(load_inventory_terminal_at(dir.path(), "parent", CHILD, &record.attempt, &wrong_event),
        Err(SubagentAccessError::InvalidInventory { reason: InventoryValidationError::InvalidSource })));
    let other = AttemptEvidence::from_recorded(Some("at1.3472966983e64bab8708952207a6fc92")).unwrap();
    assert!(matches!(load_inventory_terminal_at(dir.path(), "parent", CHILD, &other, &record.source),
        Err(SubagentAccessError::InvalidInventory { reason: InventoryValidationError::AttemptMismatch })));
    let mut bad_digest = record.source.clone();
    if let ChildEvidenceSource::ParentJournal { digest, .. } = &mut bad_digest { *digest = "z".repeat(64); }
    assert!(matches!(load_inventory_terminal_at(dir.path(), "parent", CHILD, &record.attempt, &bad_digest),
        Err(SubagentAccessError::InvalidInventory { reason: InventoryValidationError::InvalidSource })));
    assert_eq!(std::fs::read_to_string(dir.path().join("updates.jsonl")).unwrap(), contents);
}

#[tokio::test]
async fn no_replay_and_cursor_keep_full_evidence_and_highwater_at_import_hook() {
    let dir = tempfile::tempdir().unwrap();
    let highwater = r#"{"sessionId":"parent","update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"prompt"}},"_meta":{"eventId":"parent-16881851"}}"#;
    let contents = format!("{highwater}\n{}", finish(""));
    for cursor in [None, Some("parent-38")] {
        let mut prepared = crate::session::storage::prepare_parent_replay_lines(&contents, cursor, "parent");
        // noReplay drops delivery, not preparation or its import batch.
        prepared.lines.clear();
        let batch = prepare_parent_inventory(dir.path(), "parent",
            SourceVersion { incarnation: uuid::Uuid::new_v4(), sequence: 7 },
            &mut prepared, &summary_bytes("")).await;
        assert!(batch.issues.is_empty());
        assert_eq!(batch.records.len(), 1);
        assert_eq!(batch.records[0].subagent_id, CHILD);
        assert_eq!(prepared.max_event_seq, Some(16881851));
        assert!(prepared.child_evidence.is_empty());
        assert!(prepared.lines.is_empty());
    }
}

#[test]
fn rewind_discards_terminal_and_its_highwater_before_evidence_extraction() {
    let prompt = |text| format!(r#"{{"sessionId":"parent","update":{{"sessionUpdate":"user_message_chunk","content":{{"type":"text","text":"{text}"}}}}}}"#);
    let discarded = finish("").replace("parent-38", "parent-9999");
    let rewind = r#"{"method":"_x.ai/session/update","params":{"sessionId":"parent","update":{"sessionUpdate":"rewind_marker","target_prompt_index":1,"created_at":"2024-01-01"}}}"#;
    let kept = finish("");
    let reply = r#"{"sessionId":"parent","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"reply"}}}"#;
    let contents = format!("{}\n{reply}\n{}\n{discarded}\n{rewind}\n{kept}", prompt("kept"), prompt("discarded"));
    let prepared = crate::session::storage::prepare_parent_replay_lines(&contents, None, "parent");
    assert!(prepared.child_evidence_issues.is_empty());
    assert_eq!(prepared.child_evidence.len(), 1);
    assert_eq!(prepared.max_event_seq, Some(38));
    assert!(matches!(&prepared.child_evidence[0].source,
        ChildEvidenceSource::ParentJournal { line: 6, byte_offset, .. }
        if *byte_offset == contents.rfind(&kept).unwrap() as u64));
}

fn metadata_wire(child: &str, attempt: &str, parent: &str) -> serde_json::Value {
    serde_json::json!({
        "subagent_id": child, "child_session_id": child, "parent_session_id": parent,
        "attempt_id": attempt, "subagent_type": "general-purpose", "description": "fixture",
        "prompt": "redacted", "status": "cancelled", "started_at": "2026-10-27T15:55:18Z"
    })
}

#[tokio::test]
async fn four_historical_terminal_shapes_keep_exact_attempts_low_event_ids_and_unknown_owners() {
    const PARENT: &str = "01a17d8b-5982-77e2-8445-eb654bb66440";
    // Redacted contract fields; no output/token completeness is invented.
    let cases = [
        ("01a17e12-b8e1-7601-9816-4f32046b2f5b", "at1.3472966983e64bab8708952207a6fc92", 120590, 5, 1,
            1443, 15434334, 1498, 49, Some("permission manager unavailable for tool write")),
        (CHILD, ATTEMPT, 337393, 0, 0,
            8346, 16881851, 8407, 38, None),
        ("01a18cfe-a095-7660-ba59-cf9f15d73309", "at1.929852e1d174f8caeab3fd0ea34a6ce", 1107137, 54, 1,
            21972, 18194045, 33064, 96, Some("permission manager unavailable for tool write")),
        ("01a18d09-eb74-7661-b419-0cbe14af9d48", "at1.2e63b2dc61dc4a3b85b9e18e1c7f991e", 352623, 21, 1,
            22149, 18326411, 33063, 95, Some("permission manager unavailable for tool run_terminal_command")),
    ];
    let dir = tempfile::tempdir().unwrap();
    let mut lines = vec![String::new(); 33064];
    let mut metadata = Vec::new();
    let labels = ["Author exact Git operation lookup", "Repair GitLab backup path contract",
        "Repair Traefik rehearsal blockers", "Review observer identity repair"];
    for (&(child, attempt, duration, tools, turns, spawn_line, spawn_seq, finish_line, finish_seq, producer_error), label) in cases.iter().zip(labels) {
        let error = producer_error.unwrap_or("interrupted by process restart");
        let spawn = serde_json::json!({"sessionId": PARENT, "update": {
            "sessionUpdate": "subagent_spawned", "subagent_id": child, "child_session_id": child,
            "parent_session_id": PARENT, "attempt_id": attempt, "subagent_type": "general-purpose",
            "description": label
        }, "_meta": {"eventId": format!("{PARENT}-{spawn_seq}")}});
        let mut finish = serde_json::json!({"sessionId": PARENT, "update": {
            "sessionUpdate": "subagent_finished", "subagent_id": child, "child_session_id": child,
            "attempt_id": attempt, "status": "cancelled", "error": error,
            "duration_ms": duration, "tool_calls": tools, "turns": turns, "will_wake": false
        }, "_meta": {"eventId": format!("{PARENT}-{finish_seq}")}});
        if let Some(message) = producer_error {
            finish["_meta"]["subagentCompletion"] = serde_json::json!({
                "id": {"subagent_id": child, "attempt_id": attempt},
                "state": {"status": "interrupted", "cause": {"kind": "error", "message": message}}
            });
        }
        lines[spawn_line - 1] = spawn.to_string();
        lines[finish_line - 1] = finish.to_string();
        let mut meta = metadata_wire(child, attempt, PARENT);
        meta["description"] = label.into();
        meta["duration_ms"] = duration.into();
        meta["tool_calls"] = tools.into();
        meta["turns"] = turns.into();
        meta["error"] = error.into();
        let bytes = serde_json::to_vec(&meta).unwrap();
        let path = dir.path().join("subagents").join(child).join("meta.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        metadata.push((path, bytes));
    }
    let contents = lines.join("\n");
    std::fs::write(dir.path().join("updates.jsonl"), &contents).unwrap();
    let mut replay = crate::session::storage::prepare_parent_replay_lines(&contents, Some(&format!("{PARENT}-96")), PARENT);
    assert!(replay.child_evidence_issues.is_empty());
    assert_eq!(replay.max_event_seq, Some(18326411));
    let parent_summary = format!(r#"{{"info":{{"id":"{PARENT}","cwd":"/fixture"}}}}"#);
    let batch = prepare_parent_inventory(dir.path(), PARENT,
        SourceVersion { incarnation: uuid::Uuid::new_v4(), sequence: 0 },
        &mut replay, parent_summary.as_bytes()).await;
    assert!(batch.issues.is_empty());
    for &(child, attempt, duration, tools, turns, _, _, finish_line, finish_seq, producer_error) in &cases {
        let expected_attempt = AttemptEvidence::from_recorded(Some(attempt)).unwrap();
        let terminal_record = batch.records.iter().find(|record| record.subagent_id == child
            && matches!(&record.source, ChildEvidenceSource::ParentJournal { line, .. } if *line == finish_line as u64)).unwrap();
        assert_eq!(terminal_record.attempt, expected_attempt);
        assert!(matches!(&terminal_record.source, ChildEvidenceSource::ParentJournal { event_id: Some(id), .. }
            if id == &format!("{PARENT}-{finish_seq}")));
        let ChildEvidence::Terminal { terminal } = &terminal_record.value else { panic!("historical terminal"); };
        match terminal {
            ChildTerminalEvidence::ProducerFact { completion_id, state, recorded, .. } => {
                let message = producer_error.expect("producer fact in original shape");
                let recorded = recorded.as_ref().expect("original counters");
                assert_eq!(completion_id.subagent_id, child);
                assert_eq!(completion_id.attempt_id.as_str(), attempt);
                assert!(matches!(state, SubagentTerminalState::Interrupted { cause: xai_tool_types::InterruptionCause::Error { message: actual } } if actual == message));
                assert_eq!(recorded.output, None);
                assert_eq!(recorded.tokens_used, None);
            }
            ChildTerminalEvidence::Recorded { result } => {
                assert_eq!(producer_error, None);
                assert!(matches!(&result.state, RecordedTerminalState::Cancelled { cause: None, message: Some(message) }
                    if message == "interrupted by process restart"));
            }
            ChildTerminalEvidence::Producer { .. } => panic!("partial history is not a full producer"),
        }
        let recorded = terminal.recorded().unwrap();
        assert_eq!(recorded.duration_ms, Some(duration as u64));
        assert_eq!(recorded.tool_calls, Some(tools as u32));
        assert_eq!(recorded.turns, Some(turns as u32));
        let point = load_inventory_terminal(dir.path(), PARENT, child, &expected_attempt, &terminal_record.source).await.unwrap();
        assert_eq!(point.as_ref(), Some(terminal));
        let own_identity = batch.records.iter().find(|record| record.subagent_id == child
            && matches!(&record.source, ChildEvidenceSource::ParentMeta { .. })
            && matches!(&record.value, ChildEvidence::Identity { .. })).unwrap();
        assert!(matches!(&own_identity.value, ChildEvidence::Identity { recorded_owner: None, recorded_spawner: None, .. }));
    }
    assert_eq!(std::fs::read_to_string(dir.path().join("updates.jsonl")).unwrap(), contents);
    for (path, bytes) in metadata { assert_eq!(std::fs::read(path).unwrap(), bytes); }
}

#[test]
fn metadata_serde_forwards_exact_spawner_and_owner_to_canonical_lifecycle_and_artifacts() {
    use super::super::SubagentSessionMetadata;
    use crate::session::completion_delivery::CompletionArtifacts;
    for spawner in [serde_json::json!({"kind":"root"}), serde_json::json!({
        "kind":"child", "subagent_id":"direct-child", "session_id":"direct-session"
    })] {
        for owner in [serde_json::Value::Null, serde_json::json!({"kind":"task"}),
            serde_json::json!({"kind":"workflow","run_id":"workflow-17"})] {
            let dir = tempfile::tempdir().unwrap();
            let mut wire = metadata_wire(CHILD, ATTEMPT, "parent");
            wire["recorded_spawner"] = spawner.clone();
            wire["recorded_owner"] = owner.clone();
            let bytes = serde_json::to_vec(&wire).unwrap();
            let path = dir.path().join("subagents").join(CHILD).join("meta.json");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            let mut meta = metadata_store::read_owned(dir.path(), CHILD, "parent").unwrap();
            for status in [SubagentMetaStatus::Running, SubagentMetaStatus::Completed] {
                meta.status = status;
                let canonical = SubagentSessionMetadata::from_meta(&meta,
                    None, None, None, None, None, None, None, None, 0);
                let remote = serde_json::to_value(&canonical).unwrap();
                assert_eq!(remote["schemaVersion"], 1);
                assert_eq!(remote["recordedSpawner"], spawner);
                assert!(remote.get("recorded_spawner").is_none());
                if owner.is_null() { assert!(remote.get("recordedOwner").is_none()); }
                else { assert_eq!(remote["recordedOwner"], owner); }
                let decoded: SubagentSessionMetadata = serde_json::from_value(remote).unwrap();
                assert_eq!(decoded.recorded_spawner, meta.recorded_spawner);
                assert_eq!(decoded.recorded_owner, meta.recorded_owner);
                let artifacts = CompletionArtifacts::Metadata { metadata: Box::new(meta.clone()) };
                let restored: CompletionArtifacts = serde_json::from_value(serde_json::to_value(artifacts).unwrap()).unwrap();
                let CompletionArtifacts::Metadata { metadata } = restored else { panic!("metadata artifacts"); };
                let local = serde_json::to_value(metadata).unwrap();
                assert_eq!(local["recorded_spawner"], spawner);
                assert!(local.get("recordedSpawner").is_none());
                if owner.is_null() { assert!(local.get("recorded_owner").is_none()); }
                else { assert_eq!(local["recorded_owner"], owner); }
            }
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
    }
}

#[test]
fn missing_null_and_invalid_spawner_are_not_inferred_as_root() {
    use super::super::SubagentSessionMetadata;
    for extra in [None, Some(serde_json::Value::Null)] {
        let mut wire = metadata_wire(CHILD, ATTEMPT, "parent");
        if let Some(value) = extra { wire["recorded_spawner"] = value; }
        let meta: SubagentMeta = serde_json::from_value(wire).unwrap();
        assert_eq!(meta.recorded_spawner, None);
        assert!(serde_json::to_value(&meta).unwrap().get("recorded_spawner").is_none());
        let canonical = SubagentSessionMetadata::from_meta(&meta, None, None, None, None, None, None, None, None, 0);
        let mut remote = serde_json::to_value(&canonical).unwrap();
        assert!(remote.get("recordedSpawner").is_none());
        remote["recordedSpawner"] = serde_json::Value::Null;
        let decoded: SubagentSessionMetadata = serde_json::from_value(remote).unwrap();
        assert_eq!(decoded.recorded_spawner, None);
    }
    for invalid in [serde_json::json!({"kind":"future"}), serde_json::json!({"kind":"child","subagent_id":"missing-session"})] {
        let mut local = metadata_wire(CHILD, ATTEMPT, "parent");
        let meta: SubagentMeta = serde_json::from_value(local.clone()).unwrap();
        let canonical = SubagentSessionMetadata::from_meta(&meta, None, None, None, None, None, None, None, None, 0);
        let mut remote = serde_json::to_value(canonical).unwrap();
        local["recorded_spawner"] = invalid.clone();
        remote["recordedSpawner"] = invalid;
        assert!(serde_json::from_value::<SubagentMeta>(local).is_err());
        assert!(serde_json::from_value::<SubagentSessionMetadata>(remote).is_err());
    }
}
