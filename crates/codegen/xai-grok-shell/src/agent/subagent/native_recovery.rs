use super::*;
use xai_grok_tools::implementations::grok_build::task::native_recovery::{
    ArtifactCapabilityRef, ArtifactDirectPrincipal, ArtifactS3Principal, ArtifactProxyDestination, ArtifactProxyPrincipal, ArtifactProxyScope, MessageRecoveryState, PreparedMessageRecovery,
    SubagentMessageRecoveryRequest, InitialPublication, MessageRecoveryAction, MessageRecoveryAudience,
    NativeWakeCustody, NativeWakeLaunch, RecoveryPublication,
};
use crate::session::delivery_receiver::{DeliveryExecution, DeliveryProvenance, ParentMessageCustody};
use crate::session::repo_changes::UploadMethod;

fn stable_url(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some()
        || url.fragment().is_some() || !matches!(url.scheme(), "https" | "http" | "gs" | "s3")
    {
        return None;
    }
    Some(value.to_owned())
}

pub(crate) fn artifact_reference(
    bucket: Option<&str>, method: Option<&UploadMethod>, auth_manager: Option<&crate::auth::AuthManager>,
) -> Option<ArtifactCapabilityRef> {
    let Some(bucket) = bucket else { return Some(ArtifactCapabilityRef::Disabled); };
    match method? {
        UploadMethod::Direct { service_account_key } => {
            let principal = service_account_key.as_deref().and_then(|key| {
                let credentials: gcloud_storage::client::google_cloud_auth::credentials::CredentialsFile = serde_json::from_str(key).ok()?;
                if credentials.tp != "service_account" { return None; }
                let client_email = credentials.client_email?;
                if !client_email.contains('@') || client_email.chars().any(char::is_whitespace) { return None; }
                Some(ArtifactDirectPrincipal::ServiceAccount {
                    client_email, project_id: credentials.project_id,
                    token_audience_binding: crate::session::delivery_receiver::payload_digest(&stable_url(credentials.token_uri.as_deref()?)?),
                })
            }).unwrap_or(ArtifactDirectPrincipal::Unavailable);
            Some(ArtifactCapabilityRef::Direct { bucket_url: stable_url(bucket)?, principal })
        }
        UploadMethod::Proxy { proxy_base_url, deployment_key, alpha_test_key, .. } => {
            let principal = match (deployment_key, auth_manager.and_then(|manager| manager.current_wire_valid())) {
                (None, Some(auth)) if !auth.user_id.is_empty() => ArtifactProxyPrincipal::Login {
                    user_id: auth.user_id, principal_id: auth.principal_id, team_id: auth.team_id,
                    organization_id: auth.organization_id,
                    scope: match alpha_test_key { Some(_) => ArtifactProxyScope::Staging, None => ArtifactProxyScope::Standard },
                },
                (Some(_), Some(_) | None) | (None, None | Some(_)) => ArtifactProxyPrincipal::Unavailable,
            };
            let destination = match bucket {
                "proxy-managed" => ArtifactProxyDestination::Managed,
                bucket => ArtifactProxyDestination::Bucket { url: stable_url(bucket)? },
            };
            Some(ArtifactCapabilityRef::Proxy { destination, endpoint_binding: crate::session::delivery_receiver::payload_digest(&stable_url(proxy_base_url)?), principal })
        }
        UploadMethod::S3 { bucket: s3_bucket, region, endpoint_url, credentials_file, credentials_content } => Some(ArtifactCapabilityRef::S3 {
            bucket_url: stable_url(bucket)?, bucket: s3_bucket.clone(), region: region.clone(),
            principal: match (credentials_content, credentials_file) {
                (None, Some(path)) if !s3_bucket.is_empty() && !region.is_empty() && std::path::Path::new(path).is_absolute() && !path.chars().any(char::is_control) => ArtifactS3Principal::CredentialFile { binding: crate::session::delivery_receiver::payload_digest(path) },
                (Some(_), Some(_) | None) | (None, None | Some(_)) => ArtifactS3Principal::Unavailable,
            },
            endpoint_binding: match endpoint_url {
                Some(endpoint) => Some(crate::session::delivery_receiver::payload_digest(&stable_url(endpoint)?)),
                None => None,
            },
        }),
    }
}

pub(crate) enum ArtifactAdmission<'a> { New, Recovery(&'a ArtifactCapabilityRef) }

pub(crate) struct InitialArtifactGrant { state: InitialArtifactState }

enum InitialArtifactState {
    Disabled,
    Enabled { reference: ArtifactCapabilityRef, context: crate::upload::manifest::ArtifactUploadContext },
}

impl InitialArtifactGrant {
    pub(super) fn bind(
        bucket: Option<&str>, method: Option<&UploadMethod>, auth_manager: Option<&crate::auth::AuthManager>,
        prefix: String, admission: ArtifactAdmission<'_>,
    ) -> Option<Self> {
        let reference = artifact_reference(bucket, method, auth_manager)?;
        match admission {
            ArtifactAdmission::New => {}
            ArtifactAdmission::Recovery(expected) => {
                if &reference != expected { return None; }
                match &reference {
                    ArtifactCapabilityRef::Disabled
                    | ArtifactCapabilityRef::Proxy { principal: ArtifactProxyPrincipal::Login { .. }, .. }
                    | ArtifactCapabilityRef::Direct { principal: ArtifactDirectPrincipal::ServiceAccount { .. }, .. } => {}
                    ArtifactCapabilityRef::S3 { principal: ArtifactS3Principal::CredentialFile { .. }, .. } => {
                        let Some(UploadMethod::S3 { credentials_file: Some(path), .. }) = method else { return None; };
                        let file = std::fs::File::open(path).ok()?;
                        if !file.metadata().ok()?.is_file() { return None; }
                    }
                    ArtifactCapabilityRef::Proxy { principal: ArtifactProxyPrincipal::Unavailable, .. }
                    | ArtifactCapabilityRef::Direct { principal: ArtifactDirectPrincipal::Unavailable, .. }
                    | ArtifactCapabilityRef::S3 { principal: ArtifactS3Principal::Unavailable, .. } => return None,
                }
            }
        }
        let state = match reference {
            ArtifactCapabilityRef::Disabled => InitialArtifactState::Disabled,
            reference @ (ArtifactCapabilityRef::Direct { .. } | ArtifactCapabilityRef::Proxy { .. } | ArtifactCapabilityRef::S3 { .. }) => {
                let context = super::attempt_runner::artifact_upload_context(bucket, method, prefix)?;
                InitialArtifactState::Enabled { reference, context }
            }
        };
        Some(Self { state })
    }

    pub(crate) fn reference(&self) -> &ArtifactCapabilityRef {
        match &self.state {
            InitialArtifactState::Disabled => &ArtifactCapabilityRef::Disabled,
            InitialArtifactState::Enabled { reference, .. } => reference,
        }
    }

    pub(crate) fn into_context(self) -> Option<crate::upload::manifest::ArtifactUploadContext> {
        match self.state {
            InitialArtifactState::Disabled => None,
            InitialArtifactState::Enabled { context, .. } => Some(context),
        }
    }
}

pub(super) fn owned_mcp_references(servers: &[acp::McpServer]) -> Vec<String> {
    let mut names: Vec<_> = servers.iter().map(|server| crate::session::mcp_servers::mcp_server_name(server).to_owned()).collect();
    names.sort();
    names
}

fn publication_meta(launch: &NativeWakeLaunch, id: &str) -> std::io::Result<super::SubagentMeta> {
    let directory = launch.previous_completion.persisted_output_ref.as_deref().map(Path::new)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "original publication artifact reference unavailable"))?;
    let parent = directory.parent().and_then(Path::parent)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid publication artifact reference"))?;
    if parent.join("subagents").join(id) != directory {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "publication artifact identity mismatch"));
    }
    super::metadata_store::read_owned(parent, id, &launch.parent_session_id)
}

fn publication_presence_for_meta(meta: &super::SubagentMeta, launch: &NativeWakeLaunch, text: &str) -> std::io::Result<InitialPublication> {
    if meta.attempt_id.as_deref() == Some(launch.previous_completion.attempt_id.as_str())
        && meta.status == super::SubagentMetaStatus::of_state(launch.previous_completion.state())
    {
        return Ok(InitialPublication::Unpublished);
    }
    if meta.attempt_id.as_deref() == Some(launch.attempt_id.as_str()) && meta.status == super::SubagentMetaStatus::Running
        && meta.started_at == launch.publication_started_at && meta.prompt == text
    {
        return Ok(InitialPublication::Published);
    }
    Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "native publication artifact context changed"))
}

pub(crate) fn publication_presence(launch: &NativeWakeLaunch, id: &str, text: &str) -> std::io::Result<InitialPublication> {
    publication_presence_for_meta(&publication_meta(launch, id)?, launch, text)
}

pub(super) fn published_resume_source(launch: &NativeWakeLaunch, id: &str, text: &str) -> std::io::Result<ResumeSourceData> {
    let meta = publication_meta(launch, id)?;
    if publication_presence_for_meta(&meta, launch, text)? != InitialPublication::Published
        || meta.child_cwd.as_deref() != Some(launch.cwd.as_str())
    {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "published native context unavailable"));
    }
    Ok(super::resume_source_metadata(meta))
}

pub(super) fn publication_checkpoint(current: &crate::session::persistence::WakeSummaryState, delivery_id: &str, custody: &NativeWakeCustody) -> std::io::Result<Box<crate::session::delivery_receiver::ParentMessagePublication>> {
    let summary: crate::session::persistence::Summary = serde_json::from_slice(&current.summary_bytes).map_err(std::io::Error::other)?;
    match summary.completion_delivery.receiver.records.get(delivery_id).map(|record| &record.provenance) {
        Some(DeliveryProvenance::ParentMessage { admission }) => match &admission.custody {
            ParentMessageCustody::Initial { custody: stored, publication } if stored.as_ref() == custody => Ok(publication.clone()),
            ParentMessageCustody::Initial { .. } | ParentMessageCustody::Live { .. } | ParentMessageCustody::Legacy => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "native publication checkpoint conflicts with durable custody")),
        },
        Some(DeliveryProvenance::External) | None => Err(std::io::Error::new(std::io::ErrorKind::NotFound, "native publication checkpoint absent")),
    }
}

pub(super) async fn confirm_published(
    tx: &tokio::sync::mpsc::UnboundedSender<crate::session::persistence::PersistenceMsg>,
    launch: &NativeWakeLaunch,
    reservation: &std::sync::Arc<crate::session::persistence::ReservedParentMessageTurn>,
) -> bool {
    use crate::session::persistence::PersistenceMsg;
    let (respond_to, response) = tokio::sync::oneshot::channel();
    if tx.send(PersistenceMsg::ReconfirmPublishedParentMessage { reservation: reservation.clone(), launch: Box::new(launch.clone()), respond_to }).is_err() {
        return false;
    }
    matches!(response.await, Ok(Ok(())))
}

pub(super) fn prepare(
    request: &SubagentMessageRecoveryRequest,
    audience: &MessageRecoveryAudience,
    parent_cwd: &Path,
) -> PreparedMessageRecovery {
    let parent = audience.parent_session_id();
    let id = &request.reference.receiver.session_id;
    if xai_message_delivery_core::AgentId::from_uuid_v7(id.clone()).is_none() {
        return PreparedMessageRecovery::Refused;
    }
    let parent_info = SessionInfo { id: acp::SessionId::new(parent), cwd: parent_cwd.to_string_lossy().into_owned() };
    let parent_dir = session::persistence::session_dir(&parent_info);
    let meta = match super::metadata_store::read_owned(&parent_dir, id, parent) {
        Ok(meta) => meta,
        Err(_) => return PreparedMessageRecovery::Refused,
    };
    let Some(cwd) = meta.child_cwd.clone() else { return PreparedMessageRecovery::Refused; };
    let info = SessionInfo { id: acp::SessionId::new(id.clone()), cwd };
    let storage = crate::session::storage::JsonlStorageAdapter::with_root(crate::util::grok_home::grok_home());
    let Ok((summary, presence)) = storage.observe_receiver_custody(&info) else {
        return PreparedMessageRecovery::Unavailable { reference: request.reference.clone() };
    };
    if summary.hidden == Some(true) || summary.parent_session_id.as_deref() != Some(parent) {
        return PreparedMessageRecovery::Refused;
    }
    let Some(record) = summary.completion_delivery.receiver.records.get(&request.reference.delivery_id) else {
        return PreparedMessageRecovery::Unavailable { reference: request.reference.clone() };
    };
    let DeliveryProvenance::ParentMessage { admission } = &record.provenance else {
        return PreparedMessageRecovery::Refused;
    };
    use crate::session::delivery_receiver::{DeliveryProgress, ParentMessageSource};
    let source = match admission.source { ParentMessageSource::Agent => ActiveAgentMessageSource::Agent, ParentMessageSource::Human => ActiveAgentMessageSource::Human };
    if !audience.permits_sender(&admission.sender_session_id, source) || record.parent_recovery_ref().as_ref() != Some(&request.reference) {
        return PreparedMessageRecovery::Refused;
    }
    let state = match record.receipt(id, &request.reference.delivery_id, presence).progress {
        DeliveryProgress::Queued => MessageRecoveryState::Queued,
        DeliveryProgress::Processing { .. } => MessageRecoveryState::Processing,
        DeliveryProgress::Interrupted { .. } => MessageRecoveryState::Interrupted,
        DeliveryProgress::Succeeded => MessageRecoveryState::Succeeded,
        DeliveryProgress::Failed { .. } => MessageRecoveryState::Failed,
        DeliveryProgress::Cancelled { .. } => MessageRecoveryState::Cancelled,
    };
    let custody = match &admission.custody {
        ParentMessageCustody::Legacy => return PreparedMessageRecovery::Refused,
        ParentMessageCustody::Live { .. } => {
            return PreparedMessageRecovery::Observed { reference: request.reference.clone(), state };
        }
        ParentMessageCustody::Initial { custody, .. } => custody,
    };
    if custody.launch.parent_session_id != parent || custody.launch.cwd != info.cwd {
        return PreparedMessageRecovery::Refused;
    }
    match &record.execution {
        DeliveryExecution::Queued { payload, prompt_id } => {
            if prompt_id != &request.reference.prompt_id
                || crate::session::delivery_receiver::payload_digest(payload) != record.digest
            {
                return PreparedMessageRecovery::Refused;
            }
            match request.action {
                MessageRecoveryAction::Observe => {
                    return PreparedMessageRecovery::Observed { reference: request.reference.clone(), state };
                }
                MessageRecoveryAction::Adopt => {}
            }
            let output_dir = parent_dir.join("subagents").join(id);
            if custody.launch.previous_completion.parent_session_id() != parent
                || custody.launch.previous_completion.persisted_output_ref.as_deref().map(Path::new) != Some(output_dir.as_path())
            {
                return PreparedMessageRecovery::Unavailable { reference: request.reference.clone() };
            }
            let ParentMessageCustody::Initial { publication: checkpoint, .. } = &admission.custody else { return PreparedMessageRecovery::Refused; };
            let Ok(presence) = publication_presence_for_meta(&meta, &custody.launch, payload) else {
                return PreparedMessageRecovery::Unavailable { reference: request.reference.clone() };
            };
            let publication = match presence {
                InitialPublication::Published => {
                    if !crate::session::delivery_receiver::ParentMessagePublication::matches_started(&summary, &custody.launch) {
                        return PreparedMessageRecovery::Unavailable { reference: request.reference.clone() };
                    }
                    RecoveryPublication::Published
                }
                InitialPublication::Unpublished => {
                    let previous = &custody.launch.previous_completion;
                    let prior_matches = checkpoint.matches_prior(&summary)
                        || crate::session::delivery_receiver::ParentMessagePublication::matches_started(&summary, &custody.launch);
                    if previous.parent_session_id() != parent || !prior_matches
                        || previous.child_cwd != info.cwd || previous.snapshot_ref != meta.snapshot_ref
                        || previous.persisted_output_ref.as_deref().map(Path::new) != Some(output_dir.as_path())
                        || super::read_subagent_output(&output_dir).is_none()
                    {
                        return PreparedMessageRecovery::Unavailable { reference: request.reference.clone() };
                    }
                    RecoveryPublication::Unpublished { previous_prompt: meta.prompt }
                }
            };
            PreparedMessageRecovery::Initial {
                custody: custody.clone(), publication,
                message: ActiveAgentMessage { message_id: admission.message_id.clone(), sender_session_id: admission.sender_session_id.clone(), text: std::sync::Arc::from(payload.as_str()) },
                source,
            }
        }
        DeliveryExecution::Processing { .. } | DeliveryExecution::Interrupted { .. }
        | DeliveryExecution::Succeeded | DeliveryExecution::Failed { .. } | DeliveryExecution::Cancelled { .. } => {
            PreparedMessageRecovery::Observed { reference: request.reference.clone(), state }
        }
    }
}
