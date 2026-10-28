use agent_client_protocol as acp;
use crate::session::current_actions::{CurrentActionsSnapshot, SnapshotBinding};

pub const LOAD_BINDING_META_KEY: &str = "x.ai/currentActionsBinding";
pub const LOAD_SNAPSHOT_META_KEY: &str = "x.ai/currentActions";
pub const DELIVERY_META_KEY: &str = "x.ai/currentActionsDelivery";
pub const LOAD_CUT_META_KEY: &str = "x.ai/currentActionsCut";
pub const GENERATION_END_METHOD: &str = "x.ai/current_actions/generation_end";
pub const ORDERED_REPLAY_CAPABILITY: &str = "x.ai/current_actions/ordered_replay";
pub const DISCARD_METHOD: &str = "x.ai/current_actions/discard";
pub use crate::session::replay_events::SessionNotification as DiscardedNotification;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscardedRecord {
    pub role: DeliveryRole,
    pub notification: DiscardedNotification,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicationContext {
    pub binding: SnapshotBinding,
    pub incarnation: uuid::Uuid,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "role", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum DeliveryRole {
    #[default]
    Unavailable,
    Live { context: PublicationContext },
    LiveUnbound { session_id: String, incarnation: uuid::Uuid },
    Historical { binding: SnapshotBinding },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CutUnavailable {
    UnsupportedTransport,
    SourceUnavailable,
    ProducerLost,
    ReplayFlushFailed,
    PublicationIncomplete,
    ReceiverFailed,
    Superseded,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationEnd {
    #[serde(deserialize_with = "deserialize_end_snapshot")]
    pub snapshot: crate::session::current_actions::CurrentActions,
}

fn deserialize_end_snapshot<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<crate::session::current_actions::CurrentActions, D::Error> {
    let snapshot = <crate::session::current_actions::CurrentActions as serde::Deserialize>::deserialize(deserializer)?;
    CurrentActionsSnapshot::Current(snapshot.clone()).validate().map_err(serde::de::Error::custom)?;
    Ok(snapshot)
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum LoadCut {
    Unavailable { reason: CutUnavailable },
    Settled,
}

pub fn stamp_delivery(meta: &mut acp::Meta, role: &DeliveryRole) -> Result<(), serde_json::Error> {
    meta.insert(DELIVERY_META_KEY.into(), serde_json::to_value(role)?);
    Ok(())
}

pub fn delivery_role(meta: Option<&acp::Meta>) -> Result<DeliveryRole, serde_json::Error> {
    meta.and_then(|meta| meta.get(DELIVERY_META_KEY))
        .map(|value| serde_json::from_value(value.clone())).transpose()
        .map(|role| role.unwrap_or_default())
}

pub fn stamp_load_cut(meta: &mut acp::Meta, cut: &LoadCut) -> Result<(), serde_json::Error> {
    meta.insert(LOAD_CUT_META_KEY.into(), serde_json::to_value(cut)?);
    Ok(())
}

pub fn load_cut(meta: Option<&acp::Meta>) -> Result<LoadCut, serde_json::Error> {
    match meta.and_then(|meta| meta.get(LOAD_CUT_META_KEY)) {
        Some(value) => serde_json::from_value(value.clone()),
        None => Ok(LoadCut::Unavailable { reason: CutUnavailable::UnsupportedTransport }),
    }
}

pub(crate) fn transition_notification(event: &crate::session::current_actions::CurrentActionsEvent, role: &DeliveryRole) -> Result<acp::ExtNotification, serde_json::Error> {
    #[derive(serde::Serialize)]
    struct Document<'a> {
        #[serde(flatten)]
        event: &'a crate::session::current_actions::CurrentActionsEvent,
        #[serde(rename = "_meta")]
        meta: acp::Meta,
    }
    let mut meta = acp::Meta::new();
    stamp_delivery(&mut meta, role)?;
    let params = serde_json::value::to_raw_value(&Document { event, meta })?;
    Ok(acp::ExtNotification::new(crate::session::current_actions::TRANSITION_METHOD, params.into()))
}

pub fn transition_delivery(notification: &acp::ExtNotification) -> Result<(crate::session::current_actions::CurrentActionsEvent, DeliveryRole), serde_json::Error> {
    #[derive(serde::Deserialize)]
    struct Document {
        #[serde(flatten)]
        event: crate::session::current_actions::CurrentActionsEvent,
        #[serde(rename = "_meta", default)]
        meta: Option<acp::Meta>,
    }
    let document: Document = serde_json::from_str(notification.params.get())?;
    document.event.validate().map_err(<serde_json::Error as serde::de::Error>::custom)?;
    let role = delivery_role(document.meta.as_ref())?;
    let matches_source = match &role {
        DeliveryRole::Unavailable => true,
        DeliveryRole::Live { context } => context.binding.session_id == document.event.session_id
            && context.incarnation == document.event.version.incarnation,
        DeliveryRole::LiveUnbound { session_id, incarnation } => *session_id == document.event.session_id
            && *incarnation == document.event.version.incarnation,
        DeliveryRole::Historical { binding } => binding.session_id == document.event.session_id,
    };
    if !matches_source {
        return Err(<serde_json::Error as serde::de::Error>::custom("current-actions delivery source mismatch"));
    }
    Ok((document.event, role))
}

pub fn stamp_load_binding(meta: &mut acp::Meta, binding: &SnapshotBinding) -> Result<(), serde_json::Error> {
    meta.insert(LOAD_BINDING_META_KEY.into(), serde_json::to_value(binding)?);
    Ok(())
}

pub fn load_binding(meta: Option<&acp::Meta>) -> Result<Option<SnapshotBinding>, serde_json::Error> {
    meta.and_then(|meta| meta.get(LOAD_BINDING_META_KEY))
        .map(|value| serde_json::from_value(value.clone())).transpose()
}

pub fn stamp_load_snapshot(meta: &mut acp::Meta, snapshot: &CurrentActionsSnapshot) -> Result<(), serde_json::Error> {
    meta.insert(LOAD_SNAPSHOT_META_KEY.into(), serde_json::to_value(snapshot)?);
    Ok(())
}

pub fn load_snapshot(meta: Option<&acp::Meta>) -> Result<CurrentActionsSnapshot, serde_json::Error> {
    match meta.and_then(|meta| meta.get(LOAD_SNAPSHOT_META_KEY)) {
        Some(value) => serde_json::from_value(value.clone()),
        None => Ok(CurrentActionsSnapshot::Unavailable),
    }
}

pub fn snapshot_response(snapshot: &CurrentActionsSnapshot) -> super::ExtResult {
    super::to_raw_response(snapshot)
}

pub(crate) fn handle(agent: &crate::agent::MvpAgent, args: &acp::ExtRequest) -> super::ExtResult {
    let binding: SnapshotBinding = serde_json::from_str(args.params.get())
        .map_err(|_| acp::Error::invalid_params().data("invalid current-actions binding"))?;
    snapshot_response(&agent.current_actions_snapshot(binding))
}
