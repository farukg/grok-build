use super::{ACP_SESSION_UPDATE_METHOD, SessionUpdate, XAI_SESSION_UPDATE_METHOD};
use crate::extensions::notification::SessionNotification;
use serde::de::DeserializeOwned;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateMethod {
    Legacy,
    Acp,
    Xai,
    Unknown,
    Invalid,
}

impl UpdateMethod {
    fn classify(method: Option<&str>) -> Self {
        match method {
            None => Self::Legacy,
            Some(ACP_SESSION_UPDATE_METHOD) => Self::Acp,
            Some(XAI_SESSION_UPDATE_METHOD) => Self::Xai,
            Some(_) => Self::Unknown,
        }
    }
}

pub struct DecodedUpdate {
    pub update: SessionUpdate,
    pub method: UpdateMethod,
    pub timestamp_valid: bool,
}

enum Payload<'a> {
    Raw(&'a str),
    Value(serde_json::Value),
}

impl Payload<'_> {
    fn decode<T: DeserializeOwned>(self) -> Result<T, serde_json::Error> {
        match self {
            Self::Raw(raw) => serde_json::from_str(raw),
            Self::Value(value) => serde_json::from_value(value),
        }
    }
}

fn decode_payload(method: UpdateMethod, payload: Payload<'_>) -> Result<SessionUpdate, serde_json::Error> {
    match method {
        UpdateMethod::Xai => payload.decode::<SessionNotification>().map(|notification| SessionUpdate::Xai(Box::new(notification))),
        UpdateMethod::Legacy | UpdateMethod::Acp | UpdateMethod::Unknown | UpdateMethod::Invalid => payload.decode::<agent_client_protocol::SessionNotification>().map(|notification| SessionUpdate::Acp(Box::new(notification))),
    }
}

pub(super) fn decode_value(value: serde_json::Value) -> Result<SessionUpdate, serde_json::Error> {
    if value.get("method").is_some() {
        let envelope: super::SessionUpdateEnvelope = serde_json::from_value(value)?;
        envelope.into_update()
    } else {
        decode_payload(UpdateMethod::Legacy, Payload::Value(value))
    }
}

pub(super) fn decode_envelope(envelope: super::SessionUpdateEnvelope) -> Result<SessionUpdate, serde_json::Error> {
    decode_payload(UpdateMethod::classify(Some(&envelope.method)), Payload::Value(envelope.params))
}

pub fn decode_update(line: &str) -> Result<DecodedUpdate, serde_json::Error> {
    decode_line::<true>(line)
}

pub(super) fn decode_for_load(line: &str) -> Result<SessionUpdate, serde_json::Error> {
    decode_line::<false>(line).map(|decoded| decoded.update)
}

fn present_raw<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<&'de serde_json::value::RawValue>, D::Error> {
    <&serde_json::value::RawValue as serde::Deserialize>::deserialize(deserializer).map(Some)
}

fn decode_line<const REPORT: bool>(line: &str) -> Result<DecodedUpdate, serde_json::Error> {
    #[derive(serde::Deserialize)]
    #[serde(transparent)]
    struct BorrowedMethod<'a>(#[serde(borrow)] std::borrow::Cow<'a, str>);
    #[derive(serde::Deserialize)]
    struct BorrowedEnvelope<'a> {
        #[serde(default, borrow, deserialize_with = "present_raw")]
        method: Option<&'a serde_json::value::RawValue>,
        #[serde(default, borrow, deserialize_with = "present_raw")]
        params: Option<&'a serde_json::value::RawValue>,
        #[serde(default, borrow, deserialize_with = "present_raw")]
        timestamp: Option<&'a serde_json::value::RawValue>,
    }
    if let Ok(envelope) = serde_json::from_str::<BorrowedEnvelope<'_>>(line) {
        let params = envelope.params.map(|raw| raw.get());
        let (method, payload) = match envelope.method {
            None => (UpdateMethod::Legacy, params.unwrap_or(line)),
            Some(raw) => match serde_json::from_str::<Option<BorrowedMethod<'_>>>(raw.get()) {
                Ok(Some(method)) => {
                    let method = UpdateMethod::classify(Some(&method.0));
                    let method = if params.is_none() {
                        match method {
                            UpdateMethod::Acp | UpdateMethod::Xai => UpdateMethod::Invalid,
                            UpdateMethod::Legacy | UpdateMethod::Unknown | UpdateMethod::Invalid => method,
                        }
                    } else { method };
                    (method, params.unwrap_or(line))
                }
                Ok(None) => (UpdateMethod::Invalid, params.unwrap_or(line)),
                Err(_) => (UpdateMethod::Invalid, line),
            },
        };
        let timestamp_valid = !REPORT || (envelope.method.is_none() && params.is_none()) || envelope.timestamp.is_none_or(|raw| serde_json::from_str::<u64>(raw.get()).is_ok());
        return Ok(DecodedUpdate { update: decode_payload(method, Payload::Raw(payload))?, method, timestamp_valid });
    }
    Ok(DecodedUpdate {
        update: decode_payload(UpdateMethod::Legacy, Payload::Raw(line))?,
        method: UpdateMethod::Invalid,
        timestamp_valid: true,
    })
}
