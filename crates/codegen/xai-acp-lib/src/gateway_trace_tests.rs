//! Canary oracles for the gateway trace formatter: marker payloads must never
//! reach a trace line for payload-carrying extension calls, while safe methods
//! keep their full payload diagnostics.

use super::*;

const MARKER: &str = "TRACE-CANARY-MARKER";

const PAYLOAD_FREE_METHODS: [&str; 2] = [
    "x.ai/session/request_snapshot",
    "x.ai/session/request_preview",
];

fn ext_args(method: &str) -> AcpArgs<acp::ExtRequest> {
    let (response_tx, _) = oneshot::channel();
    let params = serde_json::value::to_raw_value(&serde_json::json!({
        "sessionId": "00000000-0000-0000-0000-000000000000",
        "body": MARKER
    }))
    .expect("canary params");
    AcpArgs {
        request: acp::ExtRequest::new(method, params.into()),
        response_tx,
    }
}

#[test]
fn payload_free_methods_withhold_marker_payload_in_every_phase() {
    for method in PAYLOAD_FREE_METHODS {
        let disclosure = before_request(&ext_args(method), true).expect("trace plan");
        assert!(
            matches!(&disclosure, TraceDisclosure::EnvelopeOnly { method: label } if label == method),
            "{method}: classified envelope-only"
        );
        for phase in [
            TracePhase::SendingRequest,
            TracePhase::ReceivedRequest,
            TracePhase::ReceivedResponse,
            TracePhase::ReceivedError,
        ] {
            let line = phase.line(&disclosure, || MARKER.to_owned());
            assert!(line.contains(method), "{method}: envelope stays attributable: {line}");
            assert!(!line.contains(MARKER), "{method}: marker leaked: {line}");
        }
    }
    assert!(
        matches!(before_request(&ext_args(PAYLOAD_FREE_METHODS[0]), false), None),
        "tracing off produces no plan"
    );
}

#[test]
fn safe_methods_keep_payload_traces() {
    let disclosure = before_request(&ext_args("x.ai/test"), true).expect("trace plan");
    assert!(
        matches!(&disclosure, TraceDisclosure::Payload { method } if method == "ext_method"),
        "unlisted extension methods keep the envelope method label"
    );
    for phase in [
        TracePhase::SendingRequest,
        TracePhase::ReceivedRequest,
        TracePhase::ReceivedResponse,
    ] {
        let line = phase.line(&disclosure, || MARKER.to_owned());
        assert!(line.contains(MARKER), "safe payload diagnostics unchanged: {line}");
    }
}

#[test]
fn error_text_is_visible_for_safe_methods_and_withheld_for_payload_free_ones() {
    let error = acp::Error::new(acp::ErrorCode::InternalError.into(), MARKER);
    let safe = TraceDisclosure::Payload { method: "ext_method".to_owned() };
    assert!(TracePhase::ReceivedError.line(&safe, || error.to_string()).contains(MARKER));
    for method in PAYLOAD_FREE_METHODS {
        let sensitive = TraceDisclosure::EnvelopeOnly { method: method.to_owned() };
        let line = TracePhase::ReceivedError.line(&sensitive, || error.to_string());
        assert!(line.contains(method), "{method}: error stays attributable: {line}");
        assert!(!line.contains(MARKER), "{method}: error marker leaked: {line}");
    }
}
