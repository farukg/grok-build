use super::update_decoder::{UpdateMethod, decode_update};
use super::{SessionUpdate, SessionUpdateEnvelope};

const LEGACY: &str = r#"{"sessionId":"s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"kept"}}}"#;
const ACP: &str = r#"{"timestamp":1,"method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"kept"}}}}"#;
const XAI: &str = r#"{"timestamp":2,"method":"_x.ai/session/update","params":{"sessionId":"s","update":{"sessionUpdate":"memory_flush_started"}}}"#;

#[test]
fn canonical_update_ingresses_share_native_payload_dispatch() {
    for (input, method) in [
        (LEGACY, UpdateMethod::Legacy),
        (ACP, UpdateMethod::Acp),
        (XAI, UpdateMethod::Xai),
    ] {
        let decoded = decode_update(input).unwrap();
        assert_eq!(decoded.method, method);
        assert!(decoded.timestamp_valid);
        let loaded = SessionUpdateEnvelope::from_str(input).unwrap();
        let value_loaded =
            SessionUpdateEnvelope::from_value(serde_json::from_str(input).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(&decoded.update).unwrap(),
            serde_json::to_value(&loaded).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&loaded).unwrap(),
            serde_json::to_value(value_loaded).unwrap()
        );
        match method {
            UpdateMethod::Legacy | UpdateMethod::Acp => {
                assert!(matches!(loaded, SessionUpdate::Acp(_)))
            }
            UpdateMethod::Xai => assert!(matches!(loaded, SessionUpdate::Xai(_))),
            UpdateMethod::Unknown | UpdateMethod::Invalid => panic!("known independent fixture"),
        }
    }
}

#[test]
fn audit_reports_ignored_envelope_metadata_without_changing_tolerant_loading() {
    for timestamp in ["null", "-1", "\"private\"", "{}"] {
        let input = ACP.replace("\"timestamp\":1", &format!("\"timestamp\":{timestamp}"));
        let decoded = decode_update(&input).unwrap();
        assert!(!decoded.timestamp_valid);
        assert!(SessionUpdateEnvelope::from_str(&input).is_ok());
    }
    let escaped = XAI.replace("_x.ai/session/update", "_x.ai/session\\u002fupdate");
    assert_eq!(decode_update(&escaped).unwrap().method, UpdateMethod::Xai);
    let unknown = ACP.replace("session/update", "future/update");
    assert_eq!(
        decode_update(&unknown).unwrap().method,
        UpdateMethod::Unknown
    );
    assert!(SessionUpdateEnvelope::from_str(&unknown).is_ok());
    for (method, expected) in [
        ("future/update", UpdateMethod::Unknown),
        ("session/update", UpdateMethod::Invalid),
    ] {
        let missing_params = LEGACY.replacen('{', &format!("{{\"method\":\"{method}\","), 1);
        assert_eq!(decode_update(&missing_params).unwrap().method, expected);
        assert!(SessionUpdateEnvelope::from_str(&missing_params).is_ok());
    }
    let null_method = ACP.replace("\"method\":\"session/update\"", "\"method\":null");
    assert_eq!(
        decode_update(&null_method).unwrap().method,
        UpdateMethod::Invalid
    );
    assert!(SessionUpdateEnvelope::from_str(&null_method).is_ok());
    let unknown_variant = XAI.replace("memory_flush_started", "future_private");
    assert!(
        matches!(decode_update(&unknown_variant).unwrap().update, SessionUpdate::Xai(notification) if matches!(notification.update, crate::extensions::notification::SessionUpdate::Unknown))
    );
    for input in [
        r#"{"method":"session/update","params":{}}"#,
        r#"{"sessionId":"s","update":42}"#,
    ] {
        assert!(decode_update(input).is_err());
        assert!(SessionUpdateEnvelope::from_str(input).is_err());
    }
}
