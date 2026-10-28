//! Internal authorization for generic prompt-queue controls.

use super::*;
use xai_agent_lifecycle::{InputPolicy, QueuePolicy, ShutdownPolicy};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InputOrigin(
    PromptOrigin,
    Vec<xai_chat_state::completion::CompletionIdentity>,
    crate::session::completion_delivery::CompletionOffer,
    super::notification_drain::NotificationBatchPayload,
);

impl InputOrigin {
    pub(crate) fn from_prompt_id(prompt_id: &str) -> Self {
        Self::new(PromptOrigin::from_prompt_id(prompt_id))
    }

    pub(crate) const fn new(origin: PromptOrigin) -> Self {
        Self(
            origin,
            Vec::new(),
            crate::session::completion_delivery::CompletionOffer::None,
            super::notification_drain::NotificationBatchPayload::TerminalOnly,
        )
    }

    pub(crate) fn with_completions(
        mut self,
        ids: Vec<xai_chat_state::completion::CompletionIdentity>,
    ) -> Self {
        self.1 = ids;
        self.2 = crate::session::completion_delivery::CompletionOffer::Automatic;
        self
    }

    pub(crate) fn with_notification_batch(
        mut self,
        payload: super::notification_drain::NotificationBatchPayload,
    ) -> Self {
        self.3 = payload;
        self
    }

    pub(crate) fn notification_batch(
        &self,
    ) -> &super::notification_drain::NotificationBatchPayload {
        &self.3
    }

    pub(crate) fn with_completion_offer(
        mut self,
        offer: crate::session::completion_delivery::CompletionOffer,
    ) -> Self {
        self.2 = offer;
        self
    }

    pub(crate) fn is_boundary_completion(&self) -> bool {
        self.2 == crate::session::completion_delivery::CompletionOffer::Boundary
            || matches!(&self.0, PromptOrigin::SubagentCompleted { completion } if matches!(completion.kind, crate::session::CompletionWakeKind::Boundary | crate::session::CompletionWakeKind::BoundaryRecovery))
    }

    pub(crate) fn completions(&self) -> &[xai_chat_state::completion::CompletionIdentity] {
        &self.1
    }

    pub(crate) fn policy(&self) -> InputPolicy {
        let mut policy = self.0.policy();
        if !self.1.is_empty() {
            policy.shutdown = ShutdownPolicy::Drain;
        }
        policy
    }

    pub(super) fn remove_consumed_completions(
        &mut self,
        consumed: &[xai_chat_state::completion::CompletionIdentity],
    ) -> bool {
        let had_completions = !self.1.is_empty();
        self.1.retain(|id| !consumed.contains(id));
        let rows_empty = match &mut self.3 {
            super::notification_drain::NotificationBatchPayload::TerminalOnly => true,
            super::notification_drain::NotificationBatchPayload::Rows { rows, .. } => {
                rows.retain(|row| match row {
                    super::notification_drain::NotificationBatchRow::Terminal { id, .. } => {
                        !consumed.contains(id)
                    }
                    super::notification_drain::NotificationBatchRow::MonitorEvent { .. } => true,
                });
                rows.is_empty()
            }
        };
        had_completions && self.1.is_empty() && rows_empty
    }

    pub(crate) const fn as_prompt_origin(&self) -> &PromptOrigin {
        &self.0
    }

    pub(crate) fn is_synthetic(&self) -> bool {
        self.0.is_synthetic()
    }

    pub(crate) fn is_preemptible_runtime_wake(&self) -> bool {
        !matches!(self.policy().shutdown, ShutdownPolicy::Drain)
    }

    pub(crate) fn completion_id(&self) -> Option<&str> {
        self.0.completion_id()
    }
}

/// Authorization for generic queue controls.
///
/// Two capabilities (visible, editable) rather than an enum, so a protected row (visible, not editable) needs no dead variant in lower stack layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct QueueMutationPolicy {
    visible: bool,
    editable: bool,
}

impl QueueMutationPolicy {
    pub(crate) const fn new(visible: bool, editable: bool) -> Self {
        Self { visible, editable }
    }

    pub(crate) const fn editable() -> Self {
        Self::new(true, true)
    }

    pub(crate) const fn hidden() -> Self {
        Self::new(false, false)
    }

    pub(crate) fn is_visible(self) -> bool {
        self.visible
    }

    pub(crate) fn is_editable(self) -> bool {
        self.editable
    }

    pub(crate) fn is_protected(self) -> bool {
        self.visible && !self.editable
    }

    pub(crate) fn from_input_origin(origin: &InputOrigin) -> Self {
        match origin.policy().queue {
            QueuePolicy::VisibleProtected => Self::new(true, false),
            QueuePolicy::VisibleEditable => Self::editable(),
            QueuePolicy::Hidden => Self::hidden(),
        }
    }
}

impl InputItem {
    pub(crate) fn is_queue_visible(&self) -> bool {
        self.queue_mutation_policy.is_visible()
    }

    pub(crate) fn is_queue_editable(&self) -> bool {
        self.queue_mutation_policy.is_editable()
    }

    pub(crate) fn is_queue_protected(&self) -> bool {
        self.queue_mutation_policy.is_protected()
    }

    pub(crate) fn editable_queue_meta_matches(
        &self,
        id: &str,
        expected_version: u64,
        owner: Option<&str>,
    ) -> bool {
        self.is_queue_editable()
            && self.queue_meta.as_ref().is_some_and(|meta| {
                meta.id == id
                    && meta.version == expected_version
                    && owner.is_none_or(|expected| meta.owner.as_deref() == Some(expected))
            })
    }

    pub(crate) fn has_queue_id(&self, id: &str) -> bool {
        self.queue_meta.as_ref().is_some_and(|meta| meta.id == id)
    }
}

pub(super) enum QueueOwner<'a> {
    Unscoped,
    Exact(Option<&'a str>),
}

impl QueueOwner<'_> {
    pub(super) fn matches(&self, owner: Option<&str>) -> bool {
        match self {
            Self::Unscoped => true,
            Self::Exact(expected) => owner == *expected,
        }
    }

    pub(super) fn editor(&self) -> Option<&str> {
        match self {
            Self::Unscoped => None,
            Self::Exact(owner) => *owner,
        }
    }
}

use crate::extensions::interject::{
    MAX_QUEUE_REQUEST_BYTES, QueueAction, QueueCaller, QueueRejection, QueueRequest, QueueResponse,
    QueueResult,
};

pub(crate) const QUEUE_OPERATION_LIMIT: usize = 1024;
pub(crate) const QUEUE_OPERATION_BYTES: usize = 16 * 1024 * 1024;

struct QueueOperationRecord {
    caller: QueueCaller,
    owner: Option<String>,
    edit_digest: Option<blake3::Hash>,
    response: QueueResponse,
}

enum QueueReplay {
    Response(QueueResponse),
    ObserveQueued { version: u64, owner: Option<String> },
}

#[derive(Default)]
pub(crate) struct QueueOperations {
    settled: std::collections::HashMap<uuid::Uuid, QueueOperationRecord>,
    reserved_bytes: usize,
}

impl QueueOperations {
    fn replay(
        &self,
        caller: &QueueCaller,
        request: &QueueRequest,
    ) -> Result<Option<QueueReplay>, QueueRejection> {
        let Some(record) = self.settled.get(&request.operation.operation_id) else {
            return Ok(None);
        };
        if record.caller != *caller {
            return Err(QueueRejection::Unauthorized);
        }
        if record.response.operation != request.operation || record.owner != request.owner {
            return Err(QueueRejection::OperationConflict);
        }
        match &request.action {
            QueueAction::Prioritize { new_text } => {
                if record.edit_digest
                    != new_text
                        .as_deref()
                        .map(|text| blake3::hash(text.as_bytes()))
                {
                    return Err(QueueRejection::OperationConflict);
                }
                Ok(Some(QueueReplay::Response(record.response.clone())))
            }
            QueueAction::Observe => match &record.response.result {
                QueueResult::Queued { row } => Ok(Some(QueueReplay::ObserveQueued {
                    version: row.version,
                    owner: row.owner.clone(),
                })),
                QueueResult::GoalInterjection { .. }
                | QueueResult::Started { .. }
                | QueueResult::Rejected { .. }
                | QueueResult::Unconfirmed => {
                    Ok(Some(QueueReplay::Response(record.response.clone())))
                }
            },
        }
    }

    fn admit(&mut self, bytes: usize) -> Result<(), QueueRejection> {
        if self.settled.len() >= QUEUE_OPERATION_LIMIT
            || bytes > QUEUE_OPERATION_BYTES.saturating_sub(self.reserved_bytes)
        {
            return Err(QueueRejection::Capacity);
        }
        self.reserved_bytes += bytes;
        Ok(())
    }
}

fn optional_bytes(value: Option<&str>) -> usize {
    value.map_or(0, str::len)
}

fn request_bytes(request: &QueueRequest) -> usize {
    let text_bytes = match &request.action {
        QueueAction::Prioritize { new_text } => optional_bytes(new_text.as_deref()),
        QueueAction::Observe => 0,
    };
    request
        .operation
        .session_id
        .len()
        .saturating_add(request.operation.id.len())
        .saturating_add(optional_bytes(request.owner.as_deref()))
        .saturating_add(text_bytes)
}

fn receipt_reservation(
    caller: &QueueCaller,
    request: &QueueRequest,
    state: &State,
    replacement_text: Option<&str>,
) -> usize {
    let mut bytes = std::mem::size_of::<(uuid::Uuid, QueueOperationRecord)>()
        .saturating_mul(4)
        .saturating_add(64)
        .saturating_add(state.running_prompt_id().map_or(0, str::len))
        .saturating_add(caller.retained_bytes())
        .saturating_add(request.operation.session_id.len())
        .saturating_add(request.operation.id.len())
        .saturating_add(optional_bytes(request.owner.as_deref()));
    if let Some(meta) = state
        .pending_inputs
        .iter()
        .filter_map(|item| item.queue_meta.as_ref())
        .find(|meta| meta.id == request.operation.id)
    {
        let replacement_bytes = optional_bytes(replacement_text);
        let combined_bytes = meta.combined_texts.as_ref().map_or(0, |texts| {
            texts.iter().fold(
                texts.len().saturating_mul(std::mem::size_of::<String>()),
                |bytes, text| bytes.saturating_add(text.len()),
            )
        });
        bytes = bytes
            .saturating_add(meta.id.len())
            .saturating_add(optional_bytes(meta.owner.as_deref()))
            .saturating_add(
                optional_bytes(meta.last_editor.as_deref())
                    .max(optional_bytes(request.owner.as_deref())),
            )
            .saturating_add(meta.kind.len())
            .saturating_add(meta.text.len().max(replacement_bytes))
            .saturating_add(combined_bytes);
    }
    bytes
}

impl SessionActor {
    pub(super) async fn handle_queue_send_now(
        &self,
        caller: QueueCaller,
        request: QueueRequest,
    ) -> (QueueResponse, Option<super::prompt_queue::SendNowOutcome>) {
        let rejected = |reason| QueueResponse {
            operation: request.operation.clone(),
            result: QueueResult::Rejected { reason, row: None },
        };
        if request.operation.session_id != self.session_info.id.0.as_ref() {
            return (rejected(QueueRejection::WrongSession), None);
        }
        if !caller.controls(&self.session_info.id) {
            return (rejected(QueueRejection::Unauthorized), None);
        }
        if request_bytes(&request) > MAX_QUEUE_REQUEST_BYTES {
            return (rejected(QueueRejection::PayloadTooLarge), None);
        }
        let replay = self.queue_operations.borrow().replay(&caller, &request);
        match replay {
            Err(reason) => return (rejected(reason), None),
            Ok(Some(QueueReplay::Response(response))) => return (response, None),
            Ok(Some(QueueReplay::ObserveQueued { version, owner })) => {
                let state = self.state.lock().await;
                let result = if state.running_prompt_id() == Some(request.operation.id.as_str()) {
                    if state.front_message_committed
                        && state.pending_inputs.iter().any(|item| {
                            item.queue_meta.as_ref().is_some_and(|meta| {
                                meta.id == request.operation.id
                                    && meta.version == version
                                    && meta.owner == owner
                            })
                        })
                    {
                        QueueResult::Started {
                            prompt_id: request.operation.id.clone(),
                        }
                    } else {
                        QueueResult::Unconfirmed
                    }
                } else {
                    match self
                        .build_queue_wire(&state)
                        .into_iter()
                        .find(|row| row.id == request.operation.id)
                    {
                        Some(row) if row.version == version && row.owner == owner => {
                            QueueResult::Queued { row }
                        }
                        Some(row) => QueueResult::Rejected {
                            reason: QueueRejection::Stale,
                            row: Some(row),
                        },
                        None => QueueResult::Unconfirmed,
                    }
                };
                return (
                    QueueResponse {
                        operation: request.operation,
                        result,
                    },
                    None,
                );
            }
            Ok(None) => {}
        }
        match &request.action {
            QueueAction::Observe => (
                QueueResponse {
                    operation: request.operation,
                    result: QueueResult::Unconfirmed,
                },
                None,
            ),
            QueueAction::Prioritize { new_text } => {
                let mut state = self.state.lock().await;
                let prepared = Self::prepare_queue_priority(
                    &state,
                    &request.operation.id,
                    request.operation.expected_version,
                    QueueOwner::Exact(request.owner.as_deref()),
                    new_text.as_deref(),
                );
                let reservation =
                    receipt_reservation(&caller, &request, &state, prepared.replacement_text());
                if let Err(reason) = self.queue_operations.borrow_mut().admit(reservation) {
                    return (rejected(reason), None);
                }
                let edit_digest = new_text
                    .as_deref()
                    .map(|text| blake3::hash(text.as_bytes()));
                let outcome = self.interject_queued_prompt_locked(&mut state, prepared);
                let response = QueueResponse {
                    operation: request.operation,
                    result: outcome.result(),
                };
                self.queue_operations.borrow_mut().settled.insert(
                    response.operation.operation_id,
                    QueueOperationRecord {
                        caller,
                        owner: request.owner,
                        edit_digest,
                        response: response.clone(),
                    },
                );
                let mutation = match &outcome {
                    super::prompt_queue::SendNowOutcome::Rejected { .. } => None,
                    super::prompt_queue::SendNowOutcome::Queued { .. }
                    | super::prompt_queue::SendNowOutcome::GoalInterjection { .. }
                    | super::prompt_queue::SendNowOutcome::Unconfirmed { .. } => Some(outcome),
                };
                (response, mutation)
            }
        }
    }
}
