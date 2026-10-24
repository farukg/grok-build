use serde::{Deserialize, Serialize};
use xai_chat_state::{CompletionReceipt, SubagentCompletionId};
use xai_grok_tools::implementations::grok_build::task::types::SubagentState;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionEventPolicy {
    Automatic,
    BoundaryOnly,
    Silent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompletionArtifacts {
    None,
    Metadata {
        metadata: Box<crate::agent::subagent::SubagentMeta>,
    },
}

#[derive(Debug)]
pub struct ProducerCommitAck {
    pub event: SubagentCompletionEvent,
    pub summary_path: std::path::PathBuf,
}

#[derive(Clone)]
pub(crate) struct PreparedProducerOutcome {
    pub event: std::sync::Arc<SubagentCompletionEvent>,
    pub summary_path: std::path::PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionOffer {
    Automatic,
    Boundary,
    None,
}
impl CompletionOffer {
    pub(crate) fn pending(policy: CompletionEventPolicy) -> Self {
        match policy {
            CompletionEventPolicy::Automatic => Self::Automatic,
            CompletionEventPolicy::BoundaryOnly => Self::Boundary,
            CompletionEventPolicy::Silent => Self::None,
        }
    }
}
#[derive(Debug)]
pub struct PublishedCompletion {
    pub offer: CompletionOffer,
    pub notification: crate::extensions::notification::SessionNotification,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CompletionResultReference {
    Prepared {
        path: std::path::PathBuf,
    },
    Published {
        path: std::path::PathBuf,
        event_id: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubagentCompletionFact {
    #[serde(with = "xai_chat_state::completion::completion_id_serde")]
    pub id: SubagentCompletionId,
    pub state: SubagentState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CompletionPayload {
    pub text: String,
    pub policy: CompletionEventPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<CompletionResultReference>,
}

impl CompletionPayload {
    pub(crate) fn model_text(&self, id: &SubagentCompletionId) -> String {
        match &self.reference {
            Some(reference) => format!(
                "{}\n\n{}",
                self.text,
                completion_reference_text(id, reference)
            ),
            None => self.text.clone(),
        }
    }
}

pub(crate) fn completion_reference_text(
    id: &SubagentCompletionId,
    source: &CompletionResultReference,
) -> String {
    let location = match source {
        CompletionResultReference::Prepared { path } => {
            format!("{} (prepared outcome)", path.display())
        }
        CompletionResultReference::Published { path, event_id } => {
            format!("{} (event {event_id})", path.display())
        }
    };
    let note = xai_grok_tools::reminders::task_completion::neutralize_reminder_tags(&format!(
        "Exact durable result for completion {}/{}: {location}.",
        id.subagent_id, id.attempt_id
    ));
    xai_grok_tools::reminders::wrap_reminder(&note)
}

pub(crate) fn completion_text_with_reference(
    event: &SubagentCompletionEvent,
    source: &CompletionResultReference,
) -> String {
    format!(
        "{}\n\n{}",
        event.text,
        completion_reference_text(&event.id, source)
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubagentCompletionEvent {
    #[serde(with = "xai_chat_state::completion::completion_id_serde")]
    pub id: SubagentCompletionId,
    pub state: SubagentState,
    pub text: String,
    pub output: String,
    pub child_session_id: String,
    pub tool_calls: u32,
    pub turns: u32,
    pub duration_ms: u64,
    pub tokens_used: u64,
    pub completed_at: chrono::DateTime<chrono::Utc>,
    pub policy: CompletionEventPolicy,
    pub artifacts: CompletionArtifacts,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "stage", content = "receipt", rename_all = "snake_case")]
pub(crate) enum CompletionAdmission {
    Pending,
    Projected(CompletionReceipt),
    Settled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub(crate) enum CompletionObligation {
    Preparing {
        event: SubagentCompletionEvent,
        admission: CompletionAdmission,
    },
    Pending {
        #[serde(with = "xai_chat_state::completion::completion_id_serde")]
        id: SubagentCompletionId,
        payload: CompletionPayload,
    },
    Projected {
        receipt: CompletionReceipt,
        payload: CompletionPayload,
    },
}

impl CompletionObligation {
    pub(crate) fn id(&self) -> &SubagentCompletionId {
        match self {
            Self::Preparing { event, .. } => &event.id,
            Self::Pending { id, .. } => id,
            Self::Projected { receipt, .. } => &receipt.completion,
        }
    }
    pub(crate) fn receipt(&self) -> Option<&CompletionReceipt> {
        match self {
            Self::Preparing {
                admission: CompletionAdmission::Projected(receipt),
                ..
            }
            | Self::Projected { receipt, .. } => Some(receipt),
            Self::Preparing {
                admission: CompletionAdmission::Pending | CompletionAdmission::Settled,
                ..
            }
            | Self::Pending { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CompletionDeliveryState {
    #[serde(default)]
    pub(crate) receiver: super::delivery_receiver::DeliveryReceiverState,
    #[serde(default)]
    pub(crate) wake_policy: CompletionWakePolicy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) obligations: Vec<CompletionObligation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) receipts: Vec<CompletionReceipt>,
}

#[derive(Clone, Copy)]
enum ProjectionBinding {
    CurrentTurn,
    Historical,
}

impl CompletionDeliveryState {
    fn record_projection(&mut self, receipt: &CompletionReceipt, binding: ProjectionBinding) {
        if !self
            .receipts
            .iter()
            .any(|existing| existing.completion == receipt.completion)
        {
            self.receipts.push(receipt.clone());
        }
        let Some(obligation) = self
            .obligations
            .iter_mut()
            .find(|obligation| obligation.id() == &receipt.completion)
        else {
            return;
        };
        match obligation {
            CompletionObligation::Preparing { admission, .. } => match admission {
                CompletionAdmission::Pending => {
                    *admission = CompletionAdmission::Projected(receipt.clone())
                }
                CompletionAdmission::Projected(existing) => match binding {
                    ProjectionBinding::CurrentTurn => *existing = receipt.clone(),
                    ProjectionBinding::Historical => {}
                },
                CompletionAdmission::Settled => {}
            },
            CompletionObligation::Pending { payload, .. } => {
                *obligation = CompletionObligation::Projected {
                    receipt: receipt.clone(),
                    payload: payload.clone(),
                }
            }
            CompletionObligation::Projected {
                receipt: existing, ..
            } => match binding {
                ProjectionBinding::CurrentTurn => *existing = receipt.clone(),
                ProjectionBinding::Historical => {}
            },
        }
    }

    pub(crate) fn result_reference(
        &self,
        id: &SubagentCompletionId,
        prepared_path: &std::path::Path,
    ) -> Option<CompletionResultReference> {
        match self
            .obligations
            .iter()
            .find(|obligation| obligation.id() == id)?
        {
            CompletionObligation::Preparing { .. } => Some(CompletionResultReference::Prepared {
                path: prepared_path.to_path_buf(),
            }),
            CompletionObligation::Pending { payload, .. }
            | CompletionObligation::Projected { payload, .. } => payload.reference.clone(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.receiver.records.is_empty()
            && self.obligations.is_empty()
            && self.receipts.is_empty()
            && self.wake_policy == CompletionWakePolicy::Automatic
    }
    pub(crate) fn apply(&mut self, mutation: &CompletionMutation) {
        match mutation {
            CompletionMutation::WakePolicy(policy) => self.wake_policy = *policy,
            CompletionMutation::Ready(event) => {
                if !self.obligations.iter().any(|o| o.id() == &event.id) {
                    let admission = match self.receipts.iter().find(|r| r.completion == event.id) {
                        Some(_) => CompletionAdmission::Settled,
                        None => CompletionAdmission::Pending,
                    };
                    self.obligations.push(CompletionObligation::Preparing {
                        event: event.clone(),
                        admission,
                    });
                }
            }
            CompletionMutation::PublicationFinished { id, reference } => {
                self.obligations.retain_mut(|obligation| {
                    if obligation.id() != id {
                        return true;
                    }
                    let replacement = match obligation {
                        CompletionObligation::Preparing { event, admission } => {
                            match event.policy {
                                CompletionEventPolicy::Silent => None,
                                CompletionEventPolicy::Automatic
                                | CompletionEventPolicy::BoundaryOnly => match admission {
                                    CompletionAdmission::Pending => {
                                        Some(CompletionObligation::Pending {
                                            id: event.id.clone(),
                                            payload: CompletionPayload {
                                                text: event.text.clone(),
                                                policy: event.policy,
                                                reference: Some(reference.clone()),
                                            },
                                        })
                                    }
                                    CompletionAdmission::Projected(receipt) => {
                                        Some(CompletionObligation::Projected {
                                            receipt: receipt.clone(),
                                            payload: CompletionPayload {
                                                text: event.text.clone(),
                                                policy: event.policy,
                                                reference: Some(reference.clone()),
                                            },
                                        })
                                    }
                                    CompletionAdmission::Settled => None,
                                },
                            }
                        }
                        CompletionObligation::Pending { .. }
                        | CompletionObligation::Projected { .. } => return true,
                    };
                    match replacement {
                        Some(replacement) => {
                            *obligation = replacement;
                            true
                        }
                        None => false,
                    }
                });
            }
            CompletionMutation::Projected(receipts) => {
                for receipt in receipts {
                    self.record_projection(receipt, ProjectionBinding::CurrentTurn);
                }
            }
            CompletionMutation::RetainReceipts(receipts) => {
                for receipt in receipts {
                    self.record_projection(receipt, ProjectionBinding::Historical);
                }
            }
            CompletionMutation::TurnEnded {
                prompt_id,
                parent_task,
            } => {
                self.obligations.retain_mut(|obligation| match obligation {
                    CompletionObligation::Preparing { admission, .. } => {
                        if matches!(admission, CompletionAdmission::Projected(receipt) if &receipt.prompt_id == prompt_id && &receipt.parent_task == parent_task) { *admission = CompletionAdmission::Settled; }
                        true
                    }
                    CompletionObligation::Pending { .. } => true,
                    CompletionObligation::Projected { receipt, .. } => &receipt.prompt_id != prompt_id || &receipt.parent_task != parent_task,
                });
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum CompletionMutation {
    WakePolicy(CompletionWakePolicy),
    Ready(SubagentCompletionEvent),
    PublicationFinished {
        id: SubagentCompletionId,
        reference: CompletionResultReference,
    },
    Projected(Vec<CompletionReceipt>),
    RetainReceipts(Vec<CompletionReceipt>),
    TurnEnded {
        prompt_id: String,
        parent_task: xai_chat_state::ParentTask,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionWakePolicy {
    #[default]
    Automatic,
    HumanStopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionWakeKind {
    Fresh,
    Recovery,
    Boundary,
    BoundaryRecovery,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionWake {
    pub id: SubagentCompletionId,
    pub kind: CompletionWakeKind,
}

pub(crate) fn completion_prompt_id(id: &SubagentCompletionId, kind: CompletionWakeKind) -> String {
    let prefix = match kind {
        CompletionWakeKind::Fresh => "subagent-completed-",
        CompletionWakeKind::Recovery => "subagent-recovered-",
        CompletionWakeKind::Boundary => "subagent-boundary-fresh-",
        CompletionWakeKind::BoundaryRecovery => "subagent-boundary-",
    };
    format!(
        "{prefix}{}/{}:{}",
        id.subagent_id,
        id.attempt_id,
        uuid::Uuid::now_v7()
    )
}

pub(crate) fn completion_from_prompt_id(prompt_id: &str) -> Option<CompletionWake> {
    let (key, kind) = if let Some(key) = prompt_id.strip_prefix("subagent-completed-") {
        (key, CompletionWakeKind::Fresh)
    } else if let Some(key) = prompt_id.strip_prefix("subagent-recovered-") {
        (key, CompletionWakeKind::Recovery)
    } else if let Some(key) = prompt_id.strip_prefix("subagent-boundary-fresh-") {
        (key, CompletionWakeKind::Boundary)
    } else {
        (
            prompt_id.strip_prefix("subagent-boundary-")?,
            CompletionWakeKind::BoundaryRecovery,
        )
    };
    let key = key.rsplit_once(':').map_or(key, |(identity, _)| identity);
    let (subagent_id, attempt) = key.rsplit_once('/')?;
    Some(CompletionWake {
        id: SubagentCompletionId {
            subagent_id: subagent_id.to_owned(),
            attempt_id: xai_message_delivery_core::AttemptId::parse(attempt)?,
        },
        kind,
    })
}
