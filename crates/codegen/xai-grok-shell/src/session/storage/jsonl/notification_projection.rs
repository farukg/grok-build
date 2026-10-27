use super::*;
use xai_chat_state::input_projection::NotificationProjection;

impl JsonlStorageAdapter {
    pub(crate) async fn append_notification(
        &self,
        info: &Info,
        projection: &NotificationProjection,
        admission: &xai_chat_state::InputAdmission,
    ) -> Result<CompletionProjectionAck, CompletionProjectionError> {
        let storage = self.clone();
        let info = info.clone();
        let projection = projection.clone();
        let admission = admission.clone();
        tokio::task::spawn_blocking(move || storage.append_notification_sync(&info, &projection, &admission))
            .await.map_err(|error| StrictAppendError::Indeterminate(io::Error::other(error)))?
    }

    fn append_notification_sync(
        &self,
        info: &Info,
        projection: &NotificationProjection,
        admission: &xai_chat_state::InputAdmission,
    ) -> Result<CompletionProjectionAck, CompletionProjectionError> {
        let path = self.chat_file(info);
        let _lock = Self::lock_append(&path).map_err(StrictAppendError::NotCommitted)?;
        if admission.cancellation.is_cancelled() { return Ok(CompletionProjectionAck::TurnClosed); }
        if projection.binding.parent_task != admission.parent_task
            || projection.receipts.iter().any(|receipt| receipt.parent_task != admission.parent_task)
            || !matches!(projection.item, ConversationItem::User(_))
        {
            return Err(StrictAppendError::NotCommitted(io::Error::new(io::ErrorKind::InvalidInput, "invalid notification batch admission")));
        }
        let retained_projections = Self::read_notification_projections(&path).map_err(StrictAppendError::NotCommitted)?;
        for previous in &retained_projections {
            if matches!(&previous.identity, xai_chat_state::input_projection::InputReceiptIdentity::Notification { batch_id, .. } if batch_id == &projection.binding.id) {
                Self::sync_file_durable(&OpenOptions::new().read(true).open(&path).map_err(StrictAppendError::Indeterminate)?)
                    .map_err(StrictAppendError::Indeterminate)?;
                Self::sync_parent_directory(&path).map_err(StrictAppendError::Indeterminate)?;
                let history = self.read_chat_history_sync(path.clone(), CHAT_FORMAT_VERSION).map_err(StrictAppendError::Indeterminate)?;
                let sources = Self::retained_completion_sources(&retained_projections, projection.receipt_identity());
                return Ok(CompletionProjectionAck::AlreadyPresent { history, sources });
            }
        }
        let receipts = Self::completion_receipts(&path).map_err(StrictAppendError::NotCommitted)?;
        let mut summary = self.read_summary_sync(info).map_err(StrictAppendError::NotCommitted)?;
        if !receipts.is_empty() {
            Self::sync_file_durable(&OpenOptions::new().read(true).open(&path).map_err(StrictAppendError::Indeterminate)?)
                .map_err(StrictAppendError::Indeterminate)?;
            Self::sync_parent_directory(&path).map_err(StrictAppendError::Indeterminate)?;
            apply_patch_locked(&self.summary_file(info), &self.summary_lock_file(info), &SummaryPatch {
                completion_mutation: Some(CompletionMutation::RetainReceipts(receipts.clone())), ..Default::default()
            }).map_err(StrictAppendError::Indeterminate)?;
        }
        summary.completion_delivery.apply(&CompletionMutation::RetainReceipts(receipts.clone()));
        self.reconcile_completion_turns_sync(info, &mut summary).map_err(StrictAppendError::NotCommitted)?;
        if projection.receipts.iter().any(|receipt| summary.completion_delivery.obligations.iter().any(|obligation| {
            matches!(obligation, crate::session::completion_delivery::CompletionObligation::Suppressed { id, .. } if id == &receipt.completion)
        })) {
            return Err(StrictAppendError::NotCommitted(io::Error::new(io::ErrorKind::PermissionDenied, "notification batch disposition changed")));
        }
        if projection.receipts.iter().any(|expected| summary.completion_delivery.receipts.iter().any(|receipt| receipt.completion == expected.completion)) {
            return Err(StrictAppendError::NotCommitted(io::Error::new(io::ErrorKind::AlreadyExists, "notification terminal was projected by another input")));
        }
        let mut line = serde_json::to_vec(projection).map_err(|error| StrictAppendError::NotCommitted(io::Error::other(error)))?;
        line.push(b'\n');
        Self::append_jsonl_line_unlocked_with(&path, line, AppendDurability::Durable, Self::sync_file_durable, || Self::sync_parent_directory(&path))
            .map_err(|error| match error {
                AppendLineError::NotCommitted(source) => StrictAppendError::NotCommitted(source),
                AppendLineError::Committed(source) => StrictAppendError::Committed { acknowledgement: CompletionProjectionAck::Appended, source },
            })?;
        let mut committed = receipts;
        committed.extend(projection.receipts.iter().cloned());
        apply_patch_locked(&self.summary_file(info), &self.summary_lock_file(info), &SummaryPatch {
            completion_mutation: Some(CompletionMutation::RetainReceipts(committed)),
            chat_format_version: Some(CHAT_FORMAT_VERSION),
            chat_messages: Some(crate::session::storage::summary_write::CounterOp::Increment(1)),
            ..Default::default()
        }).map_err(|source| StrictAppendError::Committed { acknowledgement: CompletionProjectionAck::Appended, source })?;
        Ok(CompletionProjectionAck::Appended)
    }
}
