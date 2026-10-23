#[cfg(test)]
#[path = "completion_tests.rs"]
mod tests;

use super::*;
use crate::session::completion_delivery::{CompletionMutation, SubagentCompletionEvent};
use crate::session::storage::summary_write::{SummaryPatch, apply_patch_locked};
use xai_chat_state::{
    CompletionProjection, CompletionProjectionAck, CompletionProjectionError, CompletionReceipt,
    StrictAppendError,
};

impl JsonlStorageAdapter {
    pub(super) async fn prepare_completion(
        &self,
        info: &Info,
        mut event: SubagentCompletionEvent,
    ) -> io::Result<crate::session::ProducerCommitAck> {
        let storage = self.clone();
        let info = info.clone();
        tokio::task::spawn_blocking(move || {
            let path = storage.chat_file(&info);
            let _lock = Self::lock_append(&path)?;
            if matches!(
                event.state,
                xai_grok_tools::implementations::grok_build::task::types::SubagentState::Running
            ) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "producer preparation requires a classified terminal result",
                ));
            }
            let summary = storage.read_summary_sync(&info)?;
            if let Some(crate::session::completion_delivery::CompletionObligation::Preparing {
                event: existing,
                ..
            }) = summary
                .completion_delivery
                .obligations
                .iter()
                .find(|o| o.id() == &event.id)
            {
                event = existing.clone();
            }
            crate::agent::subagent::normalize_completion_artifacts(
                &storage.session_dir(&info),
                &info.id.to_string(),
                &mut event,
            )?;
            apply_patch_locked(
                &storage.summary_file(&info),
                &storage.summary_lock_file(&info),
                &SummaryPatch {
                    completion_mutation: Some(CompletionMutation::Ready(event.clone())),
                    ..Default::default()
                },
            )?;
            Ok(crate::session::ProducerCommitAck {
                event,
                summary_path: storage.summary_file(&info),
            })
        })
        .await
        .map_err(io::Error::other)?
    }

    pub(super) async fn publish_completion(
        &self,
        info: &Info,
        event: &SubagentCompletionEvent,
        will_wake: bool,
    ) -> io::Result<crate::session::PublishedCompletion> {
        let storage = self.clone();
        let info = info.clone();
        let event = event.clone();
        tokio::task::spawn_blocking(move || {
            crate::agent::subagent::materialize_completion_artifacts(
                &storage.session_dir(&info),
                &info.id.to_string(),
                &event,
            )?;
            let path = storage.updates_file(&info);
            let _lock = Self::lock_append(&path)?;
            let summary = storage.read_summary_sync(&info)?;
            let projected = summary
                .completion_delivery
                .receipts
                .iter()
                .any(|r| r.completion == event.id)
                || Self::completion_receipts(&storage.chat_file(&info))?
                    .iter()
                    .any(|r| r.completion == event.id);
            let offer = if projected {
                crate::session::completion_delivery::CompletionOffer::None
            } else {
                crate::session::completion_delivery::CompletionOffer::pending(event.policy)
            };
            let mut existing = None;
            for update in storage.read_updates_jsonl(path.clone())? {
                if let crate::session::storage::SessionUpdate::Xai(notification) = update {
                    let fact = notification
                        .meta
                        .as_ref()
                        .and_then(|m| m.get("subagentCompletion"))
                        .and_then(|value| {
                            serde_json::from_value::<
                                crate::session::completion_delivery::SubagentCompletionFact,
                            >(value.clone())
                            .ok()
                        });
                    if fact.is_some_and(|fact| fact.id == event.id) {
                        existing = Some(*notification);
                        break;
                    }
                }
            }
            let newly_published = existing.is_none();
            let notification = match existing {
                Some(notification) => {
                    Self::sync_file_durable(&OpenOptions::new().read(true).open(&path)?)?;
                    Self::sync_parent_directory(&path)?;
                    notification
                }
                None => {
                    let will_wake = will_wake && !projected;
                    let notification =
                        crate::agent::subagent::completion_lifecycle_notification_from_event(
                            &event,
                            &info.id.to_string(),
                            will_wake,
                        )?;
                    let update =
                        crate::session::storage::SessionUpdate::Xai(Box::new(notification.clone()));
                    let envelope =
                        SessionUpdateEnvelope::from_update(&update).map_err(io::Error::other)?;
                    let mut line = serde_json::to_vec(&envelope).map_err(io::Error::other)?;
                    line.push(b'\n');
                    Self::append_jsonl_line_unlocked_with(
                        &path,
                        line,
                        AppendDurability::Durable,
                        &mut Self::sync_file_durable,
                        || Self::sync_parent_directory(&path),
                    )
                    .map_err(|error| error.into_update_error().into_io_error())?;
                    notification
                }
            };
            let event_id = notification
                .meta
                .as_ref()
                .and_then(|m| m.get("eventId"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "completion history has no event identity",
                    )
                })?
                .to_owned();
            let reference =
                crate::session::completion_delivery::CompletionResultReference::Published {
                    path: path.clone(),
                    event_id,
                };
            let text = crate::session::completion_delivery::completion_text_with_reference(
                &event, &reference,
            );
            apply_patch_locked(
                &storage.summary_file(&info),
                &storage.summary_lock_file(&info),
                &SummaryPatch {
                    completion_mutation: Some(CompletionMutation::PublicationFinished {
                        id: event.id.clone(),
                        reference,
                    }),
                    messages: newly_published
                        .then_some(super::super::summary_write::CounterOp::Increment(1)),
                    record_activity: newly_published,
                    ..Default::default()
                },
            )?;
            Ok(crate::session::PublishedCompletion {
                notification,
                text,
                offer,
            })
        })
        .await
        .map_err(io::Error::other)?
    }

    pub(super) async fn recover_completions(&self, info: &Info) -> io::Result<Summary> {
        let summary = self.read_summary_sync(info)?;
        for obligation in &summary.completion_delivery.obligations {
            if let crate::session::completion_delivery::CompletionObligation::Preparing {
                event,
                ..
            } = obligation
            {
                self.publish_completion(info, event, false).await?;
            }
        }
        let receipts = Self::completion_receipts(&self.chat_file(info))?;
        if !receipts.is_empty() {
            self.apply_summary_patch(
                info,
                SummaryPatch {
                    completion_mutation: Some(CompletionMutation::RetainReceipts(receipts)),
                    ..Default::default()
                },
            )
            .await?;
        }
        let mut summary = self.read_summary_sync(info)?;
        self.reconcile_completion_turns(info, &mut summary).await?;
        Ok(summary)
    }

    fn completion_receipts(path: &Path) -> io::Result<Vec<CompletionReceipt>> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut receipts = Vec::new();
        for line in bytes.split_inclusive(|byte| *byte == b'\n') {
            if !line.ends_with(b"\n") {
                break;
            }
            if line.trim_ascii().is_empty() {
                continue;
            }
            let value: serde_json::Value =
                serde_json::from_slice(line).map_err(io::Error::other)?;
            if value.get("_subagent_completion").is_some() {
                let projection: CompletionProjection = serde_json::from_value(value)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                receipts.extend(projection.receipts().cloned());
            }
        }
        Ok(receipts)
    }

    pub(super) fn read_completion_receipts_sync(&self, info: &Info) -> io::Result<Vec<CompletionReceipt>> {
        let path = self.chat_file(info);
        let _lock = Self::lock_append(&path)?;
        let mut receipts = Self::completion_receipts(&path)?;
        for receipt in self.read_summary_sync(info)?.completion_delivery.receipts {
            if !receipts.iter().any(|actual| actual.completion == receipt.completion) { receipts.push(receipt); }
        }
        Ok(receipts)
    }

    pub(crate) async fn record_completion_ready(
        &self,
        info: &Info,
        event: SubagentCompletionEvent,
    ) -> io::Result<()> {
        self.apply_summary_patch(
            info,
            SummaryPatch {
                completion_mutation: Some(CompletionMutation::Ready(event)),
                ..Default::default()
            },
        )
        .await
    }

    pub(super) async fn append_completion(
        &self,
        info: &Info,
        projection: &CompletionProjection,
        admission: &xai_chat_state::InputAdmission,
    ) -> Result<CompletionProjectionAck, CompletionProjectionError> {
        let storage = self.clone();
        let info = info.clone();
        let projection = projection.clone();
        let admission = admission.clone();
        tokio::task::spawn_blocking(move || {
            storage.append_completion_sync(&info, &projection, &admission, Self::sync_file_durable)
        })
        .await
        .map_err(|error| StrictAppendError::Indeterminate(io::Error::other(error)))?
    }

    pub(crate) fn append_completion_sync(
        &self,
        info: &Info,
        projection: &CompletionProjection,
        admission: &xai_chat_state::InputAdmission,
        mut sync_file: impl FnMut(&std::fs::File) -> io::Result<()>,
    ) -> Result<CompletionProjectionAck, CompletionProjectionError> {
        let path = self.chat_file(info);
        let _lock = Self::lock_append(&path).map_err(StrictAppendError::NotCommitted)?;
        if admission.cancellation.is_cancelled() {
            return Ok(CompletionProjectionAck::TurnClosed);
        }
        if projection
            .receipts()
            .any(|receipt| receipt.parent_task != admission.parent_task)
        {
            return Err(StrictAppendError::NotCommitted(io::Error::new(
                io::ErrorKind::InvalidInput,
                "completion belongs to another caller task",
            )));
        }
        let mut summary = self
            .read_summary_sync(info)
            .map_err(StrictAppendError::NotCommitted)?;
        let receipts = Self::completion_receipts(&path).map_err(StrictAppendError::NotCommitted)?;
        let unindexed: Vec<_> = receipts
            .iter()
            .filter(|receipt| {
                !summary
                    .completion_delivery
                    .receipts
                    .iter()
                    .any(|indexed| indexed.completion == receipt.completion)
            })
            .cloned()
            .collect();
        if !unindexed.is_empty() {
            let file = OpenOptions::new()
                .read(true)
                .open(&path)
                .map_err(StrictAppendError::Indeterminate)?;
            sync_file(&file).map_err(StrictAppendError::Indeterminate)?;
            Self::sync_parent_directory(&path).map_err(StrictAppendError::Indeterminate)?;
            let mutation = CompletionMutation::RetainReceipts(unindexed);
            apply_patch_locked(
                &self.summary_file(info),
                &self.summary_lock_file(info),
                &SummaryPatch {
                    completion_mutation: Some(mutation.clone()),
                    ..Default::default()
                },
            )
            .map_err(StrictAppendError::Indeterminate)?;
            summary.completion_delivery.apply(&mutation);
        }
        self.reconcile_completion_turns_sync(info, &mut summary)
            .map_err(StrictAppendError::NotCommitted)?;
        let existing = receipts
            .iter()
            .chain(&summary.completion_delivery.receipts)
            .find(|receipt| receipt.completion == projection.receipt.completion);
        let tool_history = if let ConversationItem::ToolResult(candidate) = &projection.item {
            let history = self
                .read_chat_history_sync(path.clone(), CHAT_FORMAT_VERSION)
                .map_err(StrictAppendError::NotCommitted)?;
            let duplicate = history.iter().any(|item| matches!(item, ConversationItem::ToolResult(result) if result.tool_call_id == candidate.tool_call_id));
            if duplicate
                && !projection.receipts().all(|expected| {
                    receipts
                        .iter()
                        .chain(&summary.completion_delivery.receipts)
                        .any(|r| r.completion == expected.completion)
                })
            {
                return Err(StrictAppendError::NotCommitted(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "tool-call identity conflicts with completion receipt",
                )));
            }
            duplicate.then_some(history)
        } else {
            None
        };
        let is_duplicate = match &projection.item {
            ConversationItem::ToolResult(_) => tool_history.is_some(),
            ConversationItem::User(_) => existing.is_some(),
            ConversationItem::System(_)
            | ConversationItem::Assistant(_)
            | ConversationItem::BackendToolCall(_)
            | ConversationItem::Reasoning(_) => {
                return Err(StrictAppendError::NotCommitted(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "completion projection must be a caller user message or tool result",
                )));
            }
        };
        let continuation = if matches!(projection.item, ConversationItem::User(_)) {
            summary
                .completion_delivery
                .obligations
                .iter()
                .find_map(|obligation| {
                    let receipt = obligation.receipt()?;
                    if receipt.completion != projection.receipt.completion {
                        return None;
                    }
                    match (&receipt.parent_task, &projection.receipt.parent_task) {
                        (
                            xai_chat_state::ParentTask::Instance { id: previous },
                            xai_chat_state::ParentTask::Instance { id: current },
                        ) => {
                            if previous == current {
                                return None;
                            }
                            let mut rebound = receipt.clone();
                            rebound.prompt_id = projection.receipt.prompt_id.clone();
                            rebound.parent_task = projection.receipt.parent_task.clone();
                            Some(rebound)
                        }
                        (
                            xai_chat_state::ParentTask::Unknown,
                            xai_chat_state::ParentTask::Unknown
                            | xai_chat_state::ParentTask::Instance { .. },
                        )
                        | (
                            xai_chat_state::ParentTask::Instance { .. },
                            xai_chat_state::ParentTask::Unknown,
                        ) => None,
                    }
                })
        } else {
            None
        };
        let acknowledgement = if is_duplicate {
            if path.exists() {
                let file = OpenOptions::new()
                    .read(true)
                    .open(&path)
                    .map_err(StrictAppendError::NotCommitted)?;
                sync_file(&file).map_err(StrictAppendError::Indeterminate)?;
                Self::sync_parent_directory(&path).map_err(StrictAppendError::Indeterminate)?;
            }
            let history = self
                .read_chat_history_sync(path.clone(), CHAT_FORMAT_VERSION)
                .map_err(StrictAppendError::Indeterminate)?;
            match &continuation {
                Some(_) => CompletionProjectionAck::Continue { history },
                None => CompletionProjectionAck::AlreadyPresent { history },
            }
        } else {
            let mut line = serde_json::to_vec(projection).map_err(|error| {
                StrictAppendError::NotCommitted(io::Error::new(io::ErrorKind::InvalidData, error))
            })?;
            line.push(b'\n');
            Self::append_jsonl_line_unlocked_with(
                &path,
                line,
                AppendDurability::Durable,
                &mut sync_file,
                || Self::sync_parent_directory(&path),
            )
            .map_err(|error| match error {
                AppendLineError::NotCommitted(source) => StrictAppendError::NotCommitted(source),
                AppendLineError::Committed(source) => StrictAppendError::Committed {
                    acknowledgement: CompletionProjectionAck::Appended,
                    source,
                },
            })?;
            CompletionProjectionAck::Appended
        };
        let projected_receipts = projection
            .receipts()
            .map(|expected| {
                receipts
                    .iter()
                    .chain(&summary.completion_delivery.receipts)
                    .find(|actual| actual.completion == expected.completion)
                    .unwrap_or(expected)
                    .clone()
            })
            .collect();
        let mutation = match continuation {
            Some(receipt) => CompletionMutation::Projected(vec![receipt]),
            None => CompletionMutation::RetainReceipts(projected_receipts),
        };
        apply_patch_locked(
            &self.summary_file(info),
            &self.summary_lock_file(info),
            &SummaryPatch {
                completion_mutation: Some(mutation),
                chat_format_version: Some(CHAT_FORMAT_VERSION),
                chat_messages: matches!(acknowledgement, CompletionProjectionAck::Appended)
                    .then_some(crate::session::storage::summary_write::CounterOp::Increment(1)),
                ..Default::default()
            },
        )
        .map_err(|source| StrictAppendError::Committed {
            acknowledgement: acknowledgement.clone(),
            source,
        })?;
        Ok(acknowledgement)
    }

    pub(super) async fn reconcile_completion_turns(
        &self,
        info: &Info,
        summary: &mut Summary,
    ) -> io::Result<()> {
        self.reconcile_completion_turns_sync(info, summary)
    }

    fn reconcile_completion_turns_sync(
        &self,
        info: &Info,
        summary: &mut Summary,
    ) -> io::Result<()> {
        if !summary
            .completion_delivery
            .obligations
            .iter()
            .any(|o| o.receipt().is_some())
        {
            return Ok(());
        }
        let path = self.updates_file(info);
        match OpenOptions::new().read(true).open(&path) {
            Ok(file) => {
                Self::sync_file_durable(&file)?;
                Self::sync_parent_directory(&path)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        }
        for update in self.read_updates_jsonl(path)? {
            if let crate::session::storage::SessionUpdate::Xai(notification) = update
                && let crate::extensions::notification::SessionUpdate::TurnCompleted {
                    prompt_id,
                    stop_reason,
                    agent_result,
                    ..
                } = notification.update
                && matches!(
                    crate::session::delivery_receiver::terminal_progress(
                        &serde_json::Value::String(stop_reason),
                        &agent_result.map_or(serde_json::Value::Null, serde_json::Value::String),
                        None
                    ),
                    crate::session::delivery_receiver::DeliveryProgress::Succeeded
                )
                && summary
                    .completion_delivery
                    .obligations
                    .iter()
                    .filter_map(|o| o.receipt())
                    .any(|receipt| {
                        receipt.prompt_id == prompt_id
                            && xai_chat_state::input_projection::parent_task_from_meta(
                                notification.meta.as_ref(),
                            )
                            .is_ok_and(|parent| parent == receipt.parent_task)
                    })
            {
                let parent_task = xai_chat_state::input_projection::parent_task_from_meta(
                    notification.meta.as_ref(),
                )
                .map_err(io::Error::other)?;
                let mutation = CompletionMutation::TurnEnded {
                    prompt_id,
                    parent_task,
                };
                apply_patch_locked(
                    &self.summary_file(info),
                    &self.summary_lock_file(info),
                    &SummaryPatch {
                        completion_mutation: Some(mutation.clone()),
                        ..Default::default()
                    },
                )?;
                summary.completion_delivery.apply(&mutation);
            }
        }
        Ok(())
    }

    pub(super) async fn replace_chat_preserving_completion_receipts(
        &self,
        info: &Info,
        items: &[ConversationItem],
    ) -> io::Result<()> {
        let storage = self.clone();
        let info = info.clone();
        let items = items.to_vec();
        tokio::task::spawn_blocking(move || {
            let path = storage.chat_file(&info);
            let _lock = Self::lock_append(&path)?;
            let receipts = Self::completion_receipts(&path)?;
            if !receipts.is_empty() {
                apply_patch_locked(
                    &storage.summary_file(&info),
                    &storage.summary_lock_file(&info),
                    &SummaryPatch {
                        completion_mutation: Some(CompletionMutation::RetainReceipts(receipts)),
                        ..Default::default()
                    },
                )?;
            }
            crate::session::storage::write_jsonl_atomic(&path, &items)
        })
        .await
        .map_err(io::Error::other)?
    }
}
