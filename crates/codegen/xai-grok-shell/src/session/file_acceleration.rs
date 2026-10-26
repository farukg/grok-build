//! File acceleration for local sessions: the gate built from the feature and the process's sandbox state, the
//! session's file system choice, and the accelerator's notices as telemetry events.
//!
//! A session is accelerated only on a positive signal that nothing confines this process, because an accelerator
//! could read files the sandbox denies. An ACP session is never offered to the accelerator: the client owns its files.

use std::sync::Arc;

use tokio::runtime::Handle;
use xai_grok_config_types::{
    ENV_FILE_ACCELERATION_ROUTES, Feature, resolve_file_acceleration_routes,
};
use xai_grok_telemetry::events::{
    FileAccelerationArm, FileAccelerationSessionEnded, FileAccelerationSessionStarted,
    FileAccelerationUnavailableHit,
};
use xai_grok_tools::computer::{
    file_acceleration::{
        self, AcceleratedFs, AccelerationArm, AccelerationContext, AccelerationNotice,
        FileAccelerationGate, LATENCY_BUCKET_EDGES_MS,
    },
    types::AsyncFileSystem,
};

use crate::agent::config::Config;

/// The `file_acceleration` feature and route override a session spawns with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileAccelerationSettings {
    pub is_enabled: bool,
    pub routes: Option<Arc<str>>,
}

pub(crate) fn settings(config: &Config) -> FileAccelerationSettings {
    FileAccelerationSettings {
        is_enabled: config.is_feature_enabled(Feature::FileAcceleration),
        routes: resolve_file_acceleration_routes(
            std::env::var(ENV_FILE_ACCELERATION_ROUTES).ok().as_deref(),
            config.file_acceleration.routes.as_deref(),
            config
                .remote_settings
                .as_ref()
                .and_then(|remote| remote.file_acceleration_routes.as_deref()),
        ),
    }
}

/// The session's file system and the owner of its accelerator summary. `acp_fs` is the client-backed file system
/// when the client serves files; `None` means the local disk. Only the local disk reaches the accelerator, and
/// `gate` is not built at all for an ACP session.
pub fn select_session_fs(
    acp_fs: Option<Arc<dyn AsyncFileSystem>>,
    gate: impl FnOnce() -> FileAccelerationGate,
) -> AcceleratedFs {
    match acp_fs {
        Some(fs) => AcceleratedFs { fs, session: None },
        None => file_acceleration::local_fs(gate()),
    }
}

/// The gate for one local session, reading the process's sandbox state.
pub fn gate(settings: FileAccelerationSettings, session_id: &str) -> FileAccelerationGate {
    let decision = decide_gate(GateInputs {
        is_feature_enabled: settings.is_enabled,
        configured_profile: xai_grok_sandbox::configured_profile_name(),
        requested_confinement: xai_grok_sandbox::requested_confinement_profile(),
        is_inside_bwrap: xai_grok_sandbox::is_inside_bwrap(),
    });
    match decision {
        GateDecision::Enabled => {
            let session_id = session_id.to_owned();
            let runtime = Handle::try_current().ok();
            FileAccelerationGate::Enabled(AccelerationContext {
                routes: settings.routes,
                sink: Arc::new(move |notice| {
                    // Telemetry drops an event emitted off a runtime, and an accelerator may
                    // notify from its own thread.
                    let _entered = match (Handle::try_current(), &runtime) {
                        (Err(_), Some(runtime)) => Some(runtime.enter()),
                        (Ok(_), _) | (Err(_), None) => None,
                    };
                    log_notice(&session_id, notice);
                }),
            })
        }
        GateDecision::FeatureOff => FileAccelerationGate::Disabled,
        GateDecision::SandboxUnknown
        | GateDecision::SandboxRequested
        | GateDecision::InsideBwrap => {
            tracing::debug!(reason = ?decision, "file acceleration refused for this session");
            FileAccelerationGate::Disabled
        }
    }
}

/// What the gate reads, passed in so [`decide_gate`] depends on nothing else.
#[derive(Debug, Clone, Copy)]
pub struct GateInputs<'a> {
    pub is_feature_enabled: bool,
    /// The profile recorded at startup, `off` included; `None` when startup recorded none.
    pub configured_profile: Option<&'a str>,
    /// The confining profile this process was started with, if any.
    pub requested_confinement: Option<&'a str>,
    /// This process is the child of a sandbox's bwrap re-exec.
    pub is_inside_bwrap: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    Enabled,
    FeatureOff,
    /// No profile was recorded, so there is no positive signal that nothing confines this process.
    SandboxUnknown,
    SandboxRequested,
    InsideBwrap,
}

pub fn decide_gate(inputs: GateInputs<'_>) -> GateDecision {
    if !inputs.is_feature_enabled {
        return GateDecision::FeatureOff;
    }
    if inputs.is_inside_bwrap {
        return GateDecision::InsideBwrap;
    }
    match (inputs.configured_profile, inputs.requested_confinement) {
        (None, _) => GateDecision::SandboxUnknown,
        (Some(_), Some(_)) => GateDecision::SandboxRequested,
        (Some(_), None) => GateDecision::Enabled,
    }
}

/// Records the unconfined startup profile. The process lock accepts the first call only.
pub fn record_off_sandbox_profile() {
    xai_grok_sandbox::set_configured_profile("off");
}

/// The file system a local session gets after the sandbox gate. `has_session` is true only when
/// the process accelerator wrapped it.
pub async fn read_local_acceleration(
    feature_enabled: bool,
    session_id: &str,
    path: &std::path::Path,
) -> Result<(bool, Vec<u8>), String> {
    let settings = FileAccelerationSettings {
        is_enabled: feature_enabled,
        routes: None,
    };
    let accelerated = select_session_fs(None, || gate(settings, session_id));
    let has_session = accelerated.session.is_some();
    let bytes = accelerated
        .fs
        .read_file(path)
        .await
        .map_err(|error| error.to_string())?;
    Ok((has_session, bytes))
}

/// A client-owned file system never builds the gate and never wraps.
pub fn client_owned_files_skip_acceleration() -> bool {
    let owned: std::sync::Arc<dyn AsyncFileSystem> =
        std::sync::Arc::new(file_acceleration_client_fs());
    let mut gate_built = false;
    let selected = select_session_fs(Some(owned), || {
        gate_built = true;
        xai_grok_tools::computer::file_acceleration::FileAccelerationGate::Disabled
    });
    !gate_built && selected.session.is_none()
}

fn file_acceleration_client_fs() -> impl AsyncFileSystem {
    xai_grok_tools::computer::local::MockFs::new()
}

/// The preserving factory, on a fresh slot, emits a session-started notice and serves the inner bytes.
pub async fn preserving_factory_start_notice(path: &std::path::Path) -> Result<Vec<u8>, String> {
    use std::sync::Mutex;
    let notices = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&notices);
    let sink = Arc::new(move |notice| {
        notices_push(&recorded, notice);
    });
    let accelerated = xai_grok_tools::computer::file_acceleration::apply_accelerator(
        xai_grok_tools::computer::file_acceleration::preserving_file_accelerator(),
        xai_grok_tools::computer::file_acceleration::FileAccelerationGate::Enabled(
            xai_grok_tools::computer::file_acceleration::AccelerationContext { routes: None, sink },
        ),
    );
    let started = {
        let guard = notices.lock().unwrap_or_else(|poison| poison.into_inner());
        matches!(
            guard.first(),
            Some(
                xai_grok_tools::computer::file_acceleration::AccelerationNotice::SessionStarted { .. }
            )
        )
    };
    if !started || accelerated.session.is_none() {
        return Err("preserving factory did not start a session".to_string());
    }
    accelerated
        .fs
        .read_file(path)
        .await
        .map_err(|error| error.to_string())
}

fn notices_push(
    slot: &std::sync::Mutex<Vec<xai_grok_tools::computer::file_acceleration::AccelerationNotice>>,
    notice: xai_grok_tools::computer::file_acceleration::AccelerationNotice,
) {
    slot.lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .push(notice);
}

/// One notice as the event it is logged as.
#[derive(Debug, PartialEq, Eq)]
enum AccelerationEvent {
    Started(FileAccelerationSessionStarted),
    Ended(FileAccelerationSessionEnded),
    UnavailableHit(FileAccelerationUnavailableHit),
}

fn event_for(session_id: &str, notice: AccelerationNotice) -> AccelerationEvent {
    let session_id = session_id.to_owned();
    match notice {
        AccelerationNotice::SessionStarted { arm } => {
            AccelerationEvent::Started(FileAccelerationSessionStarted {
                session_id,
                arm: telemetry_arm(arm),
            })
        }
        AccelerationNotice::SessionEnded {
            arm,
            duration_ms,
            served,
            declined,
            failed,
            served_latency,
            saturated,
        } => AccelerationEvent::Ended(FileAccelerationSessionEnded {
            session_id,
            arm: telemetry_arm(arm),
            duration_ms,
            served,
            declined,
            failed,
            served_latency_buckets: served_latency.to_vec(),
            served_latency_bucket_edges_ms: LATENCY_BUCKET_EDGES_MS.to_vec(),
            saturated,
        }),
        AccelerationNotice::UnavailableHit { label, retry_in_ms } => {
            AccelerationEvent::UnavailableHit(FileAccelerationUnavailableHit {
                session_id,
                label,
                retry_in_ms,
            })
        }
    }
}

fn telemetry_arm(arm: AccelerationArm) -> FileAccelerationArm {
    match arm {
        AccelerationArm::Treatment => FileAccelerationArm::Treatment,
        AccelerationArm::Control => FileAccelerationArm::Control,
    }
}

/// The accelerator's sink; the tools crate cannot link telemetry.
fn log_notice(session_id: &str, notice: AccelerationNotice) {
    use xai_grok_telemetry::session_ctx::log_event;
    match event_for(session_id, notice) {
        AccelerationEvent::Started(event) => log_event(event),
        AccelerationEvent::Ended(event) => log_event(event),
        AccelerationEvent::UnavailableHit(event) => log_event(event),
    }
}

#[cfg(test)]
#[path = "file_acceleration_tests.rs"]
mod tests;
