use super::{build_minimal_agent_for_tests, run_local_for_bridge_test, workspace_session_count};
use crate::session::request_capture::{BodyAvailability, RequestBodyRead, RequestOrigin, RequestSnapshotId};
use crate::session::storage::request_capture::{SnapshotRead, SnapshotReadResult};
use crate::session::storage::{StorageAdapter, jsonl::JsonlStorageAdapter};
use agent_client_protocol as acp;

async fn request(agent: &super::MvpAgent, session_id: &str, action: SnapshotRead) -> SnapshotReadResult {
    let params = serde_json::value::to_raw_value(&crate::extensions::request_snapshot::RequestSnapshotRequest {
        session_id: session_id.into(), action,
    }).unwrap();
    let result = crate::extensions::request_snapshot::handle(agent, &acp::ExtRequest::new(
        "x.ai/session/request_snapshot", params.into(),
    )).await.unwrap();
    serde_json::from_str(result.0.get()).unwrap()
}

#[test]
#[serial_test::serial]
fn passive_cold_get_list_and_body_never_load_start_resume_or_create_session_channels() {
    let home = tempfile::tempdir().unwrap();
    let _env = xai_grok_test_support::EnvGuard::set("GROK_HOME", home.path());
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let id = acp::SessionId::new(uuid::Uuid::new_v4().to_string());
        let info = crate::session::info::Info { id: id.clone(), cwd: "/cold/read-only-project".into() };
        let storage = JsonlStorageAdapter::default();
        storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
        let dir = crate::session::persistence::session_dir(&info);
        let snapshot_id = RequestSnapshotId(uuid::Uuid::new_v4());
        crate::session::storage::request_capture::begin(&dir, &RequestOrigin {
            session_id: id.to_string(), request: snapshot_id, turn: 13, generation: 17,
            prompt_id: None, conversation_id: None, logical_request_id: None,
        }).unwrap();
        let summary_before = std::fs::read(dir.join(crate::session::storage::SUMMARY_FILE)).unwrap();
        let history_before = std::fs::read(dir.join(crate::session::storage::CHAT_HISTORY_FILE)).ok();
        let workspaces_before = workspace_session_count(&agent);
        let attaching_before = agent.session_registry.attaching_count();
        let SnapshotReadResult::List { page } = request(&agent, id.0.as_ref(), SnapshotRead::List { after: 0, limit: 10 }).await else { panic!("passive cold index"); };
        assert_eq!(page.requests.len(), 1);
        assert_eq!(page.requests[0].request, snapshot_id);
        let SnapshotReadResult::Snapshot { snapshot } = request(&agent, id.0.as_ref(), SnapshotRead::Get { request: snapshot_id }).await else { panic!("cold snapshot"); };
        assert_eq!(snapshot.origin.turn, 13);
        assert!(snapshot.observations.is_empty(), "begin/prepared absence cannot manufacture an old request payload or outcome");
        let SnapshotReadResult::Body { body: RequestBodyRead::Unavailable { availability: BodyAvailability::Unknown } } = request(
            &agent, id.0.as_ref(), SnapshotRead::Body { request: snapshot_id, attempt: xai_grok_sampler::request_capture::RequestAttemptId(1), selection: Default::default() },
        ).await else { panic!("honest uncaptured body"); };
        assert!(agent.resident_handle(&id).is_none());
        assert!(agent.session_registry.live(&id).is_none());
        assert_eq!(agent.session_registry.attaching_count(), attaching_before);
        assert_eq!(workspace_session_count(&agent), workspaces_before);
        assert_eq!(std::fs::read(dir.join(crate::session::storage::SUMMARY_FILE)).unwrap(), summary_before);
        assert_eq!(std::fs::read(dir.join(crate::session::storage::CHAT_HISTORY_FILE)).ok(), history_before);
        let absent = uuid::Uuid::new_v4().to_string();
        let SnapshotReadResult::Capabilities { scope } = request(&agent, &absent, SnapshotRead::Capabilities).await else { panic!("capture scope is available without a resident session or disk store"); };
        let scope = serde_json::to_value(scope).unwrap();
        assert_eq!(scope["coverage"], "main_turn_only");
        assert_eq!(scope["unobserved"].as_array().unwrap().len(), 11);
        assert_eq!(workspace_session_count(&agent), workspaces_before);
        assert_eq!(agent.session_registry.attaching_count(), attaching_before);
        assert!(matches!(request(&agent, &absent, SnapshotRead::List { after: 0, limit: 10 }).await, SnapshotReadResult::Absent));
        assert!(agent.resident_handle(&acp::SessionId::new(absent)).is_none());
        let invalid = acp::ExtRequest::new("x.ai/session/request_snapshot", serde_json::value::to_raw_value(&serde_json::json!({
            "sessionId": id.0.as_ref(), "action": "get", "request": "../../arbitrary-path",
        })).unwrap().into());
        assert!(crate::extensions::request_snapshot::handle(&agent, &invalid).await.is_err());
    });
}
