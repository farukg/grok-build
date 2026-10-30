//! Human prompts addressed to a child session id.

use agent_client_protocol as acp;
use tokio::sync::{mpsc, oneshot};
use xai_grok_tools::implementations::grok_build::task::types::{
    ActiveAgentMessageOperation, ActiveAgentMessageOutcome, AgentAddress,
};

use xai_grok_tools::implementations::grok_build::task::coordinator::child_submission::ChildSubmissionAuthority;

use super::{ChildHost, ChildReach, ChildResidence, MvpAgent, RunningChild, SessionHost};
use crate::agent::subagent::PromptTurnReceipt;
use crate::extensions::notification::SubagentDelivery;
use crate::extensions::subagent_deliver::DeliverSubagentOutcome;
use crate::extensions::subagent_message::{SendSubagentMessageOutcome, literal_text};
use crate::extensions::subagent_resume::ResumeSubagentOutcome;
use crate::session::SessionHandle;
use crate::session::commands::PromptTurnResult;

pub(super) enum ChildPromptAdmission {
    /// The prompt takes the root prompt path; its result settles a receipt so the parent still
    /// receives the child's last turn.
    Live {
        handle: Box<SessionHandle>,
        turn: ChildTurn,
    },
    /// The coordinator wakes the finished child under the same session id.
    Woken {
        parent_session_id: acp::SessionId,
        address: AgentAddress,
    },
    Refused(ChildPromptRefusal),
}

#[derive(Debug, thiserror::Error)]
pub(super) enum ChildPromptRefusal {
    #[error("workflow-owned child sessions do not accept prompts")]
    WorkflowOwned,
    #[error("the child session has too many prompts in flight")]
    Saturated,
}

impl ChildPromptAdmission {
    pub(super) fn of(child: ChildHost) -> Self {
        let ChildHost {
            parent_session_id,
            reach,
        } = child;
        match reach {
            ChildReach::Unaddressed => Self::Refused(ChildPromptRefusal::WorkflowOwned),
            ChildReach::Addressed {
                address,
                residence: ChildResidence::Finished,
            } => Self::Woken {
                parent_session_id,
                address,
            },
            ChildReach::Addressed {
                address,
                residence: ChildResidence::Running(running),
            } => {
                let RunningChild {
                    handle,
                    turns,
                    parent_prompt_index,
                    delivery: _,
                } = *running;
                match turns.try_reserve_owned() {
                    Ok(receipt) => Self::Live {
                        handle: Box::new(handle),
                        turn: ChildTurn {
                            receipt,
                            parent_session_id,
                            parent_prompt_index,
                        },
                    },
                    // The receipt drain closes when the child's run finalizes; from then on only a
                    // wake reaches it.
                    Err(mpsc::error::TrySendError::Closed(_)) => Self::Woken {
                        parent_session_id,
                        address,
                    },
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        Self::Refused(ChildPromptRefusal::Saturated)
                    }
                }
            }
        }
    }
}

/// A reserved slot in the child's receipt drain.
pub(super) struct ChildTurn {
    receipt: mpsc::OwnedPermit<PromptTurnReceipt>,
    parent_session_id: acp::SessionId,
    parent_prompt_index: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl ChildTurn {
    /// Registers the dispatched prompt with the drain; the returned sender settles it.
    pub(super) fn admit(
        self,
        prompt_id: String,
        operation: ActiveAgentMessageOperation,
    ) -> oneshot::Sender<PromptTurnResult> {
        let (completion, result) = oneshot::channel();
        self.receipt.send(PromptTurnReceipt {
            prompt_id,
            result,
            telemetry: crate::session::telemetry::ActiveAgentMessageAdmissionTelemetry::new(
                std::time::Instant::now(),
                crate::agent::subagent::parent_telemetry_ctx(
                    &self.parent_session_id.0,
                    &self.parent_prompt_index,
                ),
                operation,
                operation,
                None,
            ),
        });
        completion
    }
}

impl MvpAgent {
    pub(crate) fn deliver_child(
        &self,
        id: &acp::SessionId,
        requested: SubagentDelivery,
    ) -> DeliverSubagentOutcome {
        let Some(SessionHost::Child(child)) = self.session_host(id) else {
            return DeliverSubagentOutcome::NotRunning;
        };
        match child.reach {
            ChildReach::Addressed {
                residence: ChildResidence::Running(running),
                ..
            } => {
                let mut previous = None;
                running.delivery.send_if_modified(|delivery| {
                    if running.turns.is_closed() {
                        return false;
                    }
                    previous = Some(*delivery);
                    let changed = *delivery != requested;
                    *delivery = requested;
                    changed
                });
                let Some(previous) = previous else {
                    return DeliverSubagentOutcome::NotRunning;
                };
                match requested {
                    SubagentDelivery::Held => DeliverSubagentOutcome::Held,
                    SubagentDelivery::OnTurnEnd => match previous {
                        SubagentDelivery::Held => DeliverSubagentOutcome::Delivered,
                        SubagentDelivery::OnTurnEnd => DeliverSubagentOutcome::NotHeld,
                    },
                }
            }
            ChildReach::Addressed {
                residence: ChildResidence::Finished,
                ..
            }
            | ChildReach::Unaddressed => DeliverSubagentOutcome::NotRunning,
        }
    }

    async fn reserve_child_user_submission(
        &self,
        parent: &acp::SessionId,
        child: &acp::SessionId,
        prompt: &str,
    ) -> Result<ChildSubmissionAuthority, acp::Error> {
        let (respond_to, response) = oneshot::channel();
        self.subagent_event_tx.send(
            xai_grok_tools::implementations::grok_build::task::types::SubagentEvent::ReserveChildSubmission {
                parent_session_id: parent.0.to_string(), child_session_id: child.0.to_string(), prompt: prompt.to_owned(), respond_to,
            },
        ).map_err(|_| acp::Error::internal_error().data("child submission coordinator closed"))?;
        tokio::time::timeout(std::time::Duration::from_secs(30), response).await
            .map_err(|_| acp::Error::internal_error().data("child submission authority timed out"))?
            .map_err(|_| acp::Error::internal_error().data("child submission authority acknowledgement lost"))?
            .map_err(|outcome| refused_wake(SendSubagentMessageOutcome::from(outcome)))
    }

    async fn admit_child_user_submission(
        &self,
        parent: &acp::SessionId,
        child: &acp::SessionId,
        address: &AgentAddress,
    ) -> Result<(), acp::Error> {
        self.validate_child_submission_target(parent, child, address)?;
        let startup = self.cfg.borrow().leader_restart_recovery.startup();
        let handle = self.session_handle_waiting_for_load(child).await;
        self.validate_child_submission_target(parent, child, address)?;
        match handle {
            Some(handle) => handle.admit_new_user_message_after_restart().await?,
            None => match startup {
                xai_grok_tools::types::restart_recovery::RestartStartup::Continue => {},
                xai_grok_tools::types::restart_recovery::RestartStartup::Cancel { .. } => {
                    let session = self.management_session(child).await?;
                    self.validate_child_submission_target(parent, child, address)?;
                    session.handle.admit_new_user_message_after_restart().await?;
                }
            },
        }
        self.validate_child_submission_target(parent, child, address)
    }

    fn validate_child_submission_target(
        &self,
        parent: &acp::SessionId,
        child: &acp::SessionId,
        address: &AgentAddress,
    ) -> Result<(), acp::Error> {
        match self.session_host(child) {
            Some(SessionHost::Child(ChildHost {
                parent_session_id,
                reach: ChildReach::Addressed { address: current, .. },
            })) if parent_session_id == *parent && current == *address => Ok(()),
            Some(SessionHost::Child(_)) | Some(SessionHost::Root) | None => {
                Err(acp::Error::invalid_request().data("child submission target changed during admission"))
            }
        }
    }

    pub(super) async fn wake_child_with_prompt(
        &self,
        parent_session_id: &acp::SessionId,
        child: &acp::SessionId,
        address: AgentAddress,
        prompt: Vec<acp::ContentBlock>,
    ) -> Result<acp::PromptResponse, acp::Error> {
        self.wait_for_child_management(child).await;
        let text = prompt_text(prompt)?;
        self.validate_child_submission_target(parent_session_id, child, &address)?;
        let authority = self.reserve_child_user_submission(parent_session_id, child, &text).await?;
        self.admit_child_user_submission(parent_session_id, child, &address).await?;
        use xai_grok_tools::implementations::grok_build::task::{backend::{ChannelBackend, SubagentBackend}, types::ActiveAgentMessageRequest};
        let outcome = if self.cfg.borrow().is_feature_enabled(crate::agent::config::Feature::ActiveAgentMessages) {
            let request = ActiveAgentMessageRequest::try_new_from_human(address.as_str(), text.clone(), ActiveAgentMessageOperation::Steer)
                .map_err(|outcome| refused_wake(SendSubagentMessageOutcome::from(outcome)))?
                .with_child_submission(authority.clone());
            ChannelBackend::for_coordinator_session(self.subagent_event_tx.clone(), parent_session_id.0.to_string())
                .send_active_message(request).await
        } else {
            ActiveAgentMessageOutcome::Unsupported
        };
        match outcome {
            ActiveAgentMessageOutcome::Unsupported => {
                self.continue_admitted_child(parent_session_id, child, text, authority).await
            }
            outcome @ ActiveAgentMessageOutcome::Accepted { .. } => {
                let wake =
                    serde_json::json!({ "childWake": SendSubagentMessageOutcome::from(outcome) });
                Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)
                    .meta(wake.as_object().cloned()))
            }
            // `ActiveAgentMessageOutcome` is `#[non_exhaustive]`; the wire outcome names every refusal.
            outcome => Err(refused_wake(SendSubagentMessageOutcome::from(outcome))),
        }
    }
}

impl MvpAgent {
    /// Without active agent messages a finished child continues the way `x.ai/subagent/resume`
    /// does: as a new child that starts from the finished one's history.
    pub(super) async fn continue_child_with_prompt(
        &self,
        parent_session_id: &acp::SessionId,
        child: &acp::SessionId,
        text: String,
    ) -> Result<acp::PromptResponse, acp::Error> {
        match self.session_host(child) {
            Some(SessionHost::Child(ChildHost { reach: ChildReach::Addressed { address, .. }, .. })) => {
                self.validate_child_submission_target(parent_session_id, child, &address)?;
            }
            Some(SessionHost::Child(_)) => return Err(acp::Error::invalid_request().data("workflow-owned child sessions do not accept prompts")),
            Some(SessionHost::Root) | None => {
                let cwd = self.get_session_cwd(parent_session_id).ok_or_else(|| acp::Error::invalid_request().data("saved child parent is unavailable"))?;
                let source = crate::agent::subagent::durable_resume_source(child.0.as_ref(), parent_session_id.0.as_ref(), &cwd)
                    .map_err(|error| acp::Error::invalid_request().data(error.to_string()))?;
                if source.subagent_id != child.0.as_ref() || source.child_session_id != child.0.as_ref() {
                    return Err(acp::Error::invalid_request().data("saved child identity does not match the submission"));
                }
            }
        }
        let authority = self.reserve_child_user_submission(parent_session_id, child, &text).await?;
        match self.session_host(child) {
            Some(SessionHost::Child(ChildHost { reach: ChildReach::Addressed { address, .. }, .. })) => {
                self.admit_child_user_submission(parent_session_id, child, &address).await?;
            }
            Some(SessionHost::Child(_)) => return Err(acp::Error::invalid_request().data("workflow-owned child sessions do not accept prompts")),
            Some(SessionHost::Root) | None => {
                let session = self.management_disk_child_submission(parent_session_id, child).await?;
                session.handle.admit_new_user_message_after_restart().await?;
            }
        }
        self.continue_admitted_child(parent_session_id, child, text, authority).await
    }

    async fn continue_admitted_child(
        &self,
        parent_session_id: &acp::SessionId,
        child: &acp::SessionId,
        text: String,
        authority: ChildSubmissionAuthority,
    ) -> Result<acp::PromptResponse, acp::Error> {
        self.wait_for_child_management(child).await;
        self.drain_old_session_thread(child).await;
        if self.child_writer_is_active(child) {
            return Err(acp::Error::invalid_request().data("previous child actor still owns persistence"));
        }
        let outcome = ResumeSubagentOutcome::from(
            self.resume_subagent_submission(&parent_session_id.0, &child.0, text, authority).await,
        );
        let body = serde_json::json!({ "childResume": outcome });
        match outcome {
            ResumeSubagentOutcome::Queued { .. } | ResumeSubagentOutcome::Resumed { .. } => {
                Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)
                    .meta(body.as_object().cloned()))
            }
            ResumeSubagentOutcome::Refused { .. } => Err(acp::Error::invalid_request().data(body)),
        }
    }
}

pub(super) fn prompt_text(prompt: Vec<acp::ContentBlock>) -> Result<String, acp::Error> {
    literal_text(prompt).map_err(refused_wake)
}

/// The parent session a client names on a prompt to a child id this shell no longer knows
/// (`_meta.childOf`, stamped by clients that show child sessions).
pub(super) fn child_of(meta: Option<&acp::Meta>) -> Option<acp::SessionId> {
    meta?
        .get("childOf")?
        .as_str()
        .map(|parent| acp::SessionId::new(parent.to_owned()))
}

fn refused_wake(outcome: SendSubagentMessageOutcome) -> acp::Error {
    acp::Error::invalid_request().data(serde_json::json!({ "childWake": outcome }))
}

/// Why a prompt to an id this shell hosts neither live nor loading was refused.
/// `ReapedChild` replaces the old bare "unknown session id" for subagent children: the wire
/// data names the child, its parent when the durable record carries one, and the recovery path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UnknownPromptTarget {
    /// The id names a subagent child whose actor is gone (reaped or leader restart); its
    /// durable record survives, so the child is recoverable through its parent session.
    ReapedChild {
        child: String,
        parent_session_id: Option<String>,
    },
    /// No session with this id exists here or on disk.
    Unknown { session_id: String },
}

impl UnknownPromptTarget {
    /// The error's `data` payload: a bare string for unknown ids (byte-identical to the
    /// pre-typed refusal), an object naming the reaped child, its parent, and the recovery
    /// path for subagent children.
    pub(super) fn wire_payload(&self) -> serde_json::Value {
        match self {
            Self::Unknown { .. } => serde_json::json!("unknown session id"),
            Self::ReapedChild {
                child,
                parent_session_id,
            } => {
                let parent_clause = parent_session_id
                    .as_deref()
                    .map(|parent| format!(" of parent session '{parent}'"))
                    .unwrap_or_default();
                serde_json::json!({
                    "kind": "reapedChild",
                    "sessionId": child,
                    "parentSessionId": parent_session_id,
                    "message": format!(
                        "session '{child}' is a subagent session{parent_clause} that is no \
                         longer running; open its parent session and resume the child from \
                         there — prompts sent from the child's own view carry `_meta.childOf` \
                         and continue it automatically",
                    ),
                })
            }
        }
    }
}

impl From<UnknownPromptTarget> for acp::Error {
    fn from(target: UnknownPromptTarget) -> Self {
        acp::Error::invalid_params().data(target.wire_payload())
    }
}

/// Typed refusal for a prompt to an id with no live host and no `_meta.childOf`.
/// The durable subagent record (the child's own `summary.json`) decides between a reaped
/// child, recoverable through its parent, and a genuinely unknown id.
pub(super) async fn unknown_prompt_target_error(session_id: &str) -> acp::Error {
    let id = session_id.to_owned();
    let summary = tokio::task::spawn_blocking(move || {
        crate::session::persistence::find_summary_by_session_id(&id)
    })
    .await
    .ok()
    .flatten();
    match summary {
        Some(summary) if summary.is_subagent_kind() => UnknownPromptTarget::ReapedChild {
            child: session_id.to_owned(),
            parent_session_id: summary.parent_session_id.clone(),
        }
        .into(),
        Some(_) | None => UnknownPromptTarget::Unknown {
            session_id: session_id.to_owned(),
        }
        .into(),
    }
}
