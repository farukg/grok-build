use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use xai_grok_sampling_types::ConversationItem;

pub const PARENT_TASK_META: &str = "_grokParentTask";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParentTask {
    #[default]
    Unknown,
    Instance { id: String },
}

pub fn parent_task_from_meta(
    meta: Option<&serde_json::Value>,
) -> Result<ParentTask, serde_json::Error> {
    match meta.and_then(|meta| meta.get(PARENT_TASK_META)) {
        Some(value) => serde_json::from_value(value.clone()),
        None => Ok(ParentTask::Unknown),
    }
}

#[derive(Clone, Debug)]
pub struct InputAdmission {
    pub parent_task: ParentTask,
    pub cancellation: CancellationToken,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeliveryProjection {
    #[serde(flatten)]
    pub item: ConversationItem,
    #[serde(rename = "_session_delivery")]
    pub binding: DeliveryBinding,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryBinding {
    pub delivery_id: String,
    pub payload_digest: String,
    pub turn_id: String,
    #[serde(default)]
    pub parent_task: ParentTask,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotificationProjection {
    #[serde(flatten)]
    pub item: ConversationItem,
    #[serde(rename = "_notification_batch")]
    pub binding: NotificationBatchBinding,
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "_subagent_completion_additional")]
    pub receipts: Vec<crate::CompletionReceipt>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationBatchBinding {
    pub id: String,
    pub prompt_id: String,
    pub parent_task: ParentTask,
}

#[derive(Clone, Debug)]
pub enum InputProjection {
    Subagent(crate::CompletionProjection),
    Delivery(DeliveryProjection),
    Notification(NotificationProjection),
}

impl InputProjection {
    pub fn bind_parent(&mut self, parent: &ParentTask) {
        match self {
            Self::Subagent(projection) => {
                projection.receipt.parent_task = parent.clone();
                for receipt in &mut projection.additional_receipts {
                    receipt.parent_task = parent.clone();
                }
            }
            Self::Delivery(projection) => projection.binding.parent_task = parent.clone(),
            Self::Notification(projection) => {
                projection.binding.parent_task = parent.clone();
                for receipt in &mut projection.receipts {
                    receipt.parent_task = parent.clone();
                }
            }
        }
    }

    pub fn item(&self) -> &ConversationItem {
        match self {
            Self::Subagent(projection) => &projection.item,
            Self::Delivery(projection) => &projection.item,
            Self::Notification(projection) => &projection.item,
        }
    }

    pub fn into_item(self) -> ConversationItem {
        match self {
            Self::Subagent(projection) => projection.item,
            Self::Delivery(projection) => projection.item,
            Self::Notification(projection) => projection.item,
        }
    }
}
