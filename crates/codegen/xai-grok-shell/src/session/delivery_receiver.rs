use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryAction {
    Admit,
    Observe,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryRequest {
    pub session_id: String,
    pub delivery_id: String,
    pub payload: String,
    pub action: DeliveryAction,
}

impl DeliveryRequest {
    pub fn digest(&self) -> String {
        payload_digest(&self.payload)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.session_id.is_empty() || self.delivery_id.is_empty() {
            return Err("sessionId and deliveryId must be nonempty");
        }
        let mut components = std::path::Path::new(&self.session_id).components();
        if !matches!(components.next(), Some(std::path::Component::Normal(_)))
            || components.next().is_some()
            || self.session_id.chars().any(char::is_control)
            || self.session_id.len() > 256
        {
            return Err("invalid target session identity");
        }
        if self.delivery_id.len() > 4096 || self.payload.len() > 16 * 1024 * 1024 {
            return Err("delivery exceeds receiver limits");
        }
        Ok(())
    }
}

pub fn payload_digest(payload: &str) -> String {
    format!("{:x}", Sha256::digest(payload.as_bytes()))
}

pub(crate) const RECOVERY_INTERRUPTION_REASON: &str =
    "receiver actor ended after input projection; execution will not be replayed";

pub(crate) fn terminal_progress(
    stop: &serde_json::Value,
    result: &serde_json::Value,
    trigger: Option<&str>,
) -> DeliveryProgress {
    if stop.as_str() == Some(crate::session::interrupted_turn::INTERRUPTED_STOP_REASON) {
        return DeliveryProgress::Interrupted {
            reason: RECOVERY_INTERRUPTION_REASON.into(),
        };
    }
    if matches!(stop.as_str(), Some("error" | "rate_limit")) {
        return DeliveryProgress::Failed {
            reason: result
                .as_str()
                .unwrap_or_else(|| stop.as_str().unwrap_or("receiver turn failed"))
                .to_owned(),
        };
    }
    match serde_json::from_value::<agent_client_protocol::StopReason>(stop.clone()) {
        Ok(agent_client_protocol::StopReason::Cancelled) => DeliveryProgress::Cancelled {
            reason: trigger.unwrap_or("receiver turn cancelled").to_owned(),
        },
        Ok(_) => DeliveryProgress::Succeeded,
        Err(_) => DeliveryProgress::Interrupted {
            reason: "unrecognized receiver turn outcome".into(),
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum DeliveryProgress {
    Queued,
    Processing {
        #[serde(rename = "turnId")]
        turn_id: String,
    },
    Succeeded,
    Failed {
        reason: String,
    },
    Cancelled {
        reason: String,
    },
    Interrupted {
        reason: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryReceipt {
    pub session_id: String,
    pub delivery_id: String,
    pub payload_digest: String,
    pub revision: u64,
    pub progress: DeliveryProgress,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum DeliveryResult {
    Absent,
    Admitted {
        receipt: DeliveryReceipt,
    },
    Conflict {
        #[serde(rename = "observedDigest")]
        observed_digest: String,
    },
    Rejected {
        reason: String,
    },
    Indeterminate {
        reason: String,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum ReceiverTurnOrigin {
    #[default]
    Other,
    Delivery {
        delivery_id: String,
        payload_digest: String,
    },
}
impl ReceiverTurnOrigin {
    pub(crate) fn from_prompt(origin: &super::PromptOrigin) -> Self {
        match origin {
            super::PromptOrigin::Delivery {
                delivery_id,
                payload_digest,
            } => Self::Delivery {
                delivery_id: delivery_id.clone(),
                payload_digest: payload_digest.clone(),
            },
            super::PromptOrigin::User
            | super::PromptOrigin::TaskCompleted { .. }
            | super::PromptOrigin::SubagentCompleted { .. }
            | super::PromptOrigin::ParentAgentMessage { .. }
            | super::PromptOrigin::ParentHumanMessage { .. }
            | super::PromptOrigin::WorkflowCompleted { .. }
            | super::PromptOrigin::NotificationDrain
            | super::PromptOrigin::GoalSummary
            | super::PromptOrigin::GoalClassifierNudge
            | super::PromptOrigin::SchedulerFired
            | super::PromptOrigin::PlanResume => Self::Other,
        }
    }
    pub(crate) fn from_meta(meta: Option<&serde_json::Value>) -> Result<Self, serde_json::Error> {
        match meta.and_then(|meta| meta.get("_receiverTurnOrigin")) {
            Some(value) => serde_json::from_value(value.clone()),
            None => Ok(Self::Other),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub(crate) enum DeliveryExecution {
    Queued {
        payload: String,
        prompt_id: String,
    },
    Processing {
        turn_id: String,
        #[serde(default)]
        parent_task: xai_chat_state::ParentTask,
        payload: String,
    },
    Interrupted {
        turn_id: String,
        #[serde(default)]
        parent_task: xai_chat_state::ParentTask,
        reason: String,
        payload: String,
    },
    Succeeded,
    Failed {
        reason: String,
    },
    Cancelled {
        reason: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct DeliveryRecord {
    pub digest: String,
    pub revision: u64,
    pub execution: DeliveryExecution,
}

impl DeliveryRecord {
    pub(crate) fn receipt(
        &self,
        session_id: &str,
        delivery_id: &str,
        presence: super::receiver_execution::ReceiverPresence,
    ) -> DeliveryReceipt {
        let (revision, progress) = match &self.execution {
            DeliveryExecution::Queued { .. } => (self.revision, DeliveryProgress::Queued),
            DeliveryExecution::Processing { turn_id, .. }
                if matches!(presence, super::receiver_execution::ReceiverPresence::Live) =>
            {
                (
                    self.revision,
                    DeliveryProgress::Processing {
                        turn_id: turn_id.clone(),
                    },
                )
            }
            DeliveryExecution::Processing { .. } => (
                self.revision.saturating_add(1),
                DeliveryProgress::Interrupted {
                    reason: RECOVERY_INTERRUPTION_REASON.into(),
                },
            ),
            DeliveryExecution::Interrupted { reason, .. } => (
                self.revision,
                DeliveryProgress::Interrupted {
                    reason: reason.clone(),
                },
            ),
            DeliveryExecution::Succeeded => (self.revision, DeliveryProgress::Succeeded),
            DeliveryExecution::Failed { reason } => (
                self.revision,
                DeliveryProgress::Failed {
                    reason: reason.clone(),
                },
            ),
            DeliveryExecution::Cancelled { reason } => (
                self.revision,
                DeliveryProgress::Cancelled {
                    reason: reason.clone(),
                },
            ),
        };
        DeliveryReceipt {
            session_id: session_id.to_owned(),
            delivery_id: delivery_id.to_owned(),
            payload_digest: self.digest.clone(),
            revision,
            progress,
        }
    }
}

pub enum ReceiverResume {
    CurrentQueue,
    Recovery { deliveries: Vec<String> },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct DeliveryReceiverState {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub records: BTreeMap<String, DeliveryRecord>,
}

impl DeliveryReceiverState {
    pub(crate) fn recoverable_ids(&self) -> Vec<String> {
        self.records
            .iter()
            .filter_map(|(id, record)| match record.execution {
                DeliveryExecution::Queued { .. }
                | DeliveryExecution::Processing { .. }
                | DeliveryExecution::Interrupted { .. } => Some(id.clone()),
                DeliveryExecution::Succeeded
                | DeliveryExecution::Failed { .. }
                | DeliveryExecution::Cancelled { .. } => None,
            })
            .collect()
    }

    pub(crate) fn observe(
        &self,
        request: &DeliveryRequest,
        presence: super::receiver_execution::ReceiverPresence,
    ) -> DeliveryResult {
        match self.records.get(&request.delivery_id) {
            Some(record) if record.digest != request.digest() => DeliveryResult::Conflict {
                observed_digest: record.digest.clone(),
            },
            Some(record) => DeliveryResult::Admitted {
                receipt: record.receipt(&request.session_id, &request.delivery_id, presence),
            },
            None => DeliveryResult::Absent,
        }
    }
    pub(crate) fn admit(&mut self, request: &DeliveryRequest) -> DeliveryResult {
        if !self.records.contains_key(&request.delivery_id) {
            self.records.insert(
                request.delivery_id.clone(),
                DeliveryRecord {
                    digest: request.digest(),
                    revision: 1,
                    execution: DeliveryExecution::Queued {
                        payload: request.payload.clone(),
                        prompt_id: uuid::Uuid::now_v7().to_string(),
                    },
                },
            );
        }
        self.observe(request, super::receiver_execution::ReceiverPresence::Live)
    }
    pub(crate) fn settle_turn(
        &mut self,
        turn_id: &str,
        parent_task: &xai_chat_state::ParentTask,
        origin: &ReceiverTurnOrigin,
        progress: &DeliveryProgress,
    ) -> Vec<String> {
        let mut changed = Vec::new();
        for (id, record) in &mut self.records {
            let owns_turn = match &record.execution {
                DeliveryExecution::Processing {
                    turn_id: owned,
                    parent_task: owner,
                    ..
                }
                | DeliveryExecution::Interrupted {
                    turn_id: owned,
                    parent_task: owner,
                    ..
                } => owned == turn_id && owner == parent_task,
                DeliveryExecution::Queued { prompt_id, .. } => match origin {
                    ReceiverTurnOrigin::Delivery {
                        delivery_id,
                        payload_digest,
                    } => {
                        prompt_id == turn_id
                            && delivery_id == id
                            && payload_digest == &record.digest
                    }
                    ReceiverTurnOrigin::Other => false,
                },
                DeliveryExecution::Succeeded
                | DeliveryExecution::Failed { .. }
                | DeliveryExecution::Cancelled { .. } => false,
            };
            if !owns_turn {
                continue;
            }
            let execution = match progress {
                DeliveryProgress::Succeeded => DeliveryExecution::Succeeded,
                DeliveryProgress::Failed { reason } => DeliveryExecution::Failed {
                    reason: reason.clone(),
                },
                DeliveryProgress::Cancelled { reason } => DeliveryExecution::Cancelled {
                    reason: reason.clone(),
                },
                DeliveryProgress::Interrupted { reason } => {
                    if matches!(&record.execution, DeliveryExecution::Interrupted { reason: existing, .. } if existing == reason)
                    {
                        continue;
                    }
                    let payload = match &record.execution {
                        DeliveryExecution::Processing { payload, .. }
                        | DeliveryExecution::Interrupted { payload, .. } => payload.clone(),
                        DeliveryExecution::Queued { .. }
                        | DeliveryExecution::Succeeded
                        | DeliveryExecution::Failed { .. }
                        | DeliveryExecution::Cancelled { .. } => continue,
                    };
                    DeliveryExecution::Interrupted {
                        turn_id: turn_id.to_owned(),
                        parent_task: parent_task.clone(),
                        reason: reason.clone(),
                        payload,
                    }
                }
                DeliveryProgress::Queued | DeliveryProgress::Processing { .. } => continue,
            };
            record.revision = match progress {
                DeliveryProgress::Interrupted { .. } => record.revision.saturating_add(1).max(3),
                DeliveryProgress::Succeeded
                | DeliveryProgress::Failed { .. }
                | DeliveryProgress::Cancelled { .. } => record.revision.saturating_add(1).max(4),
                DeliveryProgress::Queued | DeliveryProgress::Processing { .. } => continue,
            };
            record.execution = execution;
            changed.push(id.clone());
        }
        changed
    }
    pub(crate) fn imported_without_execution_lease(&mut self) {
        for record in self.records.values_mut() {
            let (turn_id, payload) = match &record.execution {
                DeliveryExecution::Queued { prompt_id, payload } => {
                    (prompt_id.clone(), payload.clone())
                }
                DeliveryExecution::Processing {
                    turn_id, payload, ..
                } => (turn_id.clone(), payload.clone()),
                DeliveryExecution::Interrupted { .. }
                | DeliveryExecution::Succeeded
                | DeliveryExecution::Failed { .. }
                | DeliveryExecution::Cancelled { .. } => continue,
            };
            record.execution = DeliveryExecution::Interrupted { turn_id, parent_task: xai_chat_state::ParentTask::Unknown, payload, reason: "session imported without the original receiver execution lease; execution will not be replayed".into() };
            record.revision = record.revision.saturating_add(1).max(3);
        }
    }
    pub(crate) fn retire(&mut self) {
        for record in self.records.values_mut() {
            match record.execution {
                DeliveryExecution::Queued { .. }
                | DeliveryExecution::Processing { .. }
                | DeliveryExecution::Interrupted { .. } => {
                    record.revision = record.revision.saturating_add(1).max(4);
                    record.execution = DeliveryExecution::Cancelled {
                        reason: "session history explicitly deleted".into(),
                    };
                }
                DeliveryExecution::Succeeded
                | DeliveryExecution::Failed { .. }
                | DeliveryExecution::Cancelled { .. } => {}
            }
        }
    }
}
