use super::{build_minimal_agent_for_tests, run_local_for_bridge_test};
use crate::extensions::request_preview::{PublishedRequestPreview, PreviewUnavailable, RequestPreviewRead, REQUEST_PREVIEW_METHOD};
use agent_client_protocol as acp;

#[test]
fn cold_preview_does_not_load_an_actor_and_rpc_has_no_draft_parameter() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let id = uuid::Uuid::new_v4().to_string();
        let resident_before = agent.resident_count();
        let params = serde_json::value::to_raw_value(&RequestPreviewRead { session_id: id.clone(), ui_generation: 19 }).unwrap();
        let result = crate::extensions::request_preview::handle(&agent,
            &acp::ExtRequest::new(REQUEST_PREVIEW_METHOD, params.into())).await.unwrap();
        let result: PublishedRequestPreview = serde_json::from_str(result.0.get()).unwrap();
        assert!(matches!(result, PublishedRequestPreview::Unavailable {
            session_id, ui_generation: 19, cause: PreviewUnavailable::SessionNotResident,
        } if session_id == id));
        assert_eq!(agent.resident_count(), resident_before);
        let params = serde_json::value::to_raw_value(&serde_json::json!({
            "sessionId": id, "uiGeneration": 20, "input": {"kind": "draft", "text": "draft canary"},
        })).unwrap();
        let error = crate::extensions::request_preview::handle(&agent,
            &acp::ExtRequest::new(REQUEST_PREVIEW_METHOD, params.into())).await.unwrap_err();
        assert!(!format!("{error:?}").contains("draft canary"));
        assert_eq!(agent.resident_count(), resident_before);
    });
}
