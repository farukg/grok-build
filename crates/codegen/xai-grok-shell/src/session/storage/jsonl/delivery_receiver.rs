use super::*;
use crate::session::delivery_receiver::{
    DeliveryAction, DeliveryExecution, DeliveryProgress, DeliveryReceipt, DeliveryRequest,
    DeliveryResult,
};
use crate::session::receiver_execution::{
    ReceiverAccess, ReceiverExecutionLease, ReceiverPresence,
};
use crate::session::storage::summary_write::mutate_locked;

#[derive(Clone, Copy)]
enum AdmissionAuthority {
    Observer,
    Owner,
}
use xai_chat_state::{
    CompletionProjectionAck, CompletionProjectionError, DeliveryBinding, DeliveryProjection,
    StrictAppendError,
};

impl JsonlStorageAdapter {
    fn receiver_lease_file(&self, info: &Info) -> PathBuf {
        self.session_dir(info).join("delivery-execution.lock")
    }
    fn receiver_presence(&self, info: &Info) -> io::Result<ReceiverPresence> {
        let file = match OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.receiver_lease_file(info))
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(ReceiverPresence::Detached);
            }
            Err(error) => return Err(error),
        };
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => {
                fs2::FileExt::unlock(&file)?;
                Ok(ReceiverPresence::Detached)
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(ReceiverPresence::Live),
            Err(error) => Err(error),
        }
    }
    fn acquire_receiver_lease(&self, info: &Info) -> io::Result<ReceiverExecutionLease> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.receiver_lease_file(info))?;
        fs2::FileExt::try_lock_exclusive(&file)?;
        Ok(ReceiverExecutionLease {
            _file: file,
            session_id: info.id.to_string(),
        })
    }
    pub(super) async fn claim_receiver_owned(
        &self,
        info: &Info,
    ) -> io::Result<(ReceiverExecutionLease, Summary)> {
        let storage = self.clone();
        let info = info.clone();
        tokio::task::spawn_blocking(move || {
            let _chat_lock = Self::lock_append(&storage.chat_file(&info))?;
            mutate_locked(
                &storage.summary_file(&info),
                &storage.summary_lock_file(&info),
                |summary| {
                    let lease = storage.acquire_receiver_lease(&info)?;
                    storage.recover_delivery_state(&info, summary)?;
                    Ok((lease, summary.clone()))
                },
            )
        })
        .await
        .map_err(io::Error::other)?
    }
    fn recover_delivery_state(&self, info: &Info, summary: &mut Summary) -> io::Result<()> {
        self.reconcile_delivery_terminals(info, summary, ReceiverPresence::Detached)?;
        for record in summary.completion_delivery.receiver.records.values_mut() {
            if let DeliveryExecution::Processing {
                turn_id,
                payload,
                parent_task,
            } = &record.execution
            {
                record.execution = DeliveryExecution::Interrupted {
                    turn_id: turn_id.clone(),
                    parent_task: parent_task.clone(),
                    reason: crate::session::delivery_receiver::RECOVERY_INTERRUPTION_REASON.into(),
                    payload: payload.clone(),
                };
                record.revision = record.revision.saturating_add(1).max(3);
            }
        }
        Ok(())
    }

    pub(super) fn delete_session_sync(&self, info: &Info) -> io::Result<()> {
        let dir = self.session_dir(info);
        if !dir.exists() {
            return Ok(());
        }
        let _chat_lock = Self::lock_append(&self.chat_file(info))?;
        let mut receiver_lease = None;
        let retain = mutate_locked(
            &self.summary_file(info),
            &self.summary_lock_file(info),
            |summary| {
                if !summary.completion_delivery.receiver.records.is_empty() {
                    receiver_lease = Some(self.acquire_receiver_lease(info)?);
                    self.reconcile_delivery_terminals(info, summary, ReceiverPresence::Detached)?;
                }
                summary.completion_delivery.receiver.retire();
                let retain = !summary.completion_delivery.receiver.records.is_empty();
                summary.completion_delivery.obligations.clear();
                summary.completion_delivery.receipts.clear();
                summary.hidden = Some(true);
                summary.num_messages = 0;
                summary.num_chat_messages = 0;
                Ok(retain)
            },
        )?;
        if retain {
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                if entry.path() == self.summary_file(info)
                    || entry.path() == self.summary_lock_file(info)
                    || entry.path() == self.receiver_lease_file(info)
                {
                    continue;
                }
                if entry.file_type()?.is_dir() {
                    std::fs::remove_dir_all(entry.path())?;
                } else {
                    std::fs::remove_file(entry.path())?;
                }
            }
            super::super::sync_parent_dir_durable(&dir.join("summary.json"))?;
            return Ok(());
        }
        match std::fs::remove_dir_all(dir) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn delivery_bindings(&self, info: &Info) -> io::Result<Vec<DeliveryBinding>> {
        let bytes = match std::fs::read(self.chat_file(info)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut bindings = Vec::new();
        for line in bytes.split_inclusive(|byte| *byte == b'\n') {
            if !line.ends_with(b"\n") {
                break;
            }
            if line.trim_ascii().is_empty() {
                continue;
            }
            let value: serde_json::Value =
                serde_json::from_slice(line).map_err(io::Error::other)?;
            if let Some(binding) = value.get("_session_delivery") {
                bindings.push(serde_json::from_value(binding.clone()).map_err(io::Error::other)?);
            }
        }
        Ok(bindings)
    }

    fn reconcile_delivery_terminals(
        &self,
        info: &Info,
        summary: &mut Summary,
        presence: ReceiverPresence,
    ) -> io::Result<()> {
        if !summary
            .completion_delivery
            .receiver
            .records
            .values()
            .any(|record| {
                matches!(
                    record.execution,
                    DeliveryExecution::Queued { .. }
                        | DeliveryExecution::Processing { .. }
                        | DeliveryExecution::Interrupted { .. }
                )
            })
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
            {
                if stop_reason == crate::session::interrupted_turn::INTERRUPTED_STOP_REASON
                    && matches!(presence, ReceiverPresence::Live)
                {
                    continue;
                }
                let progress = crate::session::delivery_receiver::terminal_progress(
                    &serde_json::Value::String(stop_reason),
                    &agent_result.map_or(serde_json::Value::Null, serde_json::Value::String),
                    notification
                        .meta
                        .as_ref()
                        .and_then(|m| m.get("cancelTrigger"))
                        .and_then(|value| value.as_str()),
                );
                let parent_task = xai_chat_state::input_projection::parent_task_from_meta(
                    notification.meta.as_ref(),
                )
                .map_err(io::Error::other)?;
                let origin = crate::session::delivery_receiver::ReceiverTurnOrigin::from_meta(
                    notification.meta.as_ref(),
                )
                .map_err(io::Error::other)?;
                summary.completion_delivery.receiver.settle_turn(
                    &prompt_id,
                    &parent_task,
                    &origin,
                    &progress,
                );
            }
        }
        Ok(())
    }

    pub(super) async fn receive_delivery_owned(
        &self,
        info: &Info,
        request: &DeliveryRequest,
        access: ReceiverAccess<'_>,
    ) -> io::Result<DeliveryResult> {
        let authority = match access {
            ReceiverAccess::Observe => AdmissionAuthority::Observer,
            ReceiverAccess::Owner(lease) if lease.session_id == info.id.to_string() => {
                AdmissionAuthority::Owner
            }
            ReceiverAccess::Owner(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "receiver lease belongs to another session",
                ));
            }
        };
        let storage = self.clone();
        let info = info.clone();
        let request = request.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(reason) = request.validate() {
                return Ok(DeliveryResult::Rejected {
                    reason: reason.into(),
                });
            }
            if request.session_id != info.id.to_string() {
                return Ok(DeliveryResult::Rejected {
                    reason: "target session mismatch".into(),
                });
            }
            match request.action {
                DeliveryAction::Observe => {
                    let summary_lock = match OpenOptions::new()
                        .read(true)
                        .open(storage.summary_lock_file(&info))
                    {
                        Ok(lock) => {
                            fs2::FileExt::lock_shared(&lock)?;
                            Some(lock)
                        }
                        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                        Err(error) => return Err(error),
                    };
                    let mut summary = storage.read_summary_sync(&info)?;
                    if summary_lock.is_none()
                        && !summary.completion_delivery.receiver.records.is_empty()
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "receiver synchronization file is missing",
                        ));
                    }
                    let presence = storage.receiver_presence(&info)?;
                    storage.reconcile_delivery_terminals(&info, &mut summary, presence)?;
                    Ok(summary
                        .completion_delivery
                        .receiver
                        .observe(&request, presence))
                }
                DeliveryAction::Admit => {
                    match authority {
                        AdmissionAuthority::Owner => {}
                        AdmissionAuthority::Observer => {
                            return Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "receiver admission requires its execution owner",
                            ));
                        }
                    }
                    let _chat_lock = Self::lock_append(&storage.chat_file(&info))?;
                    mutate_locked(
                        &storage.summary_file(&info),
                        &storage.summary_lock_file(&info),
                        |summary| {
                            storage.reconcile_delivery_terminals(
                                &info,
                                summary,
                                ReceiverPresence::Live,
                            )?;
                            let observed = summary
                                .completion_delivery
                                .receiver
                                .observe(&request, ReceiverPresence::Live);
                            match observed {
                                DeliveryResult::Absent
                                    if summary.is_hidden()
                                        || summary.parent_session_id.is_some() =>
                                {
                                    Ok(DeliveryResult::Rejected {
                                        reason: "receiver requires a live root session".into(),
                                    })
                                }
                                DeliveryResult::Absent => {
                                    Ok(summary.completion_delivery.receiver.admit(&request))
                                }
                                other @ (DeliveryResult::Admitted { .. }
                                | DeliveryResult::Conflict { .. }
                                | DeliveryResult::Rejected { .. }
                                | DeliveryResult::Indeterminate { .. }) => Ok(other),
                            }
                        },
                    )
                }
            }
        })
        .await
        .map_err(io::Error::other)?
    }

    pub(crate) fn append_delivery_sync(
        &self,
        info: &Info,
        projection: &DeliveryProjection,
        admission: &xai_chat_state::InputAdmission,
    ) -> Result<CompletionProjectionAck, CompletionProjectionError> {
        let path = self.chat_file(info);
        let _chat_lock = Self::lock_append(&path).map_err(StrictAppendError::NotCommitted)?;
        if admission.cancellation.is_cancelled() {
            return Ok(CompletionProjectionAck::TurnClosed);
        }
        if projection.binding.parent_task != admission.parent_task {
            return Err(StrictAppendError::NotCommitted(io::Error::new(
                io::ErrorKind::InvalidInput,
                "delivery belongs to another caller task",
            )));
        }
        let summary = self
            .read_summary_sync(info)
            .map_err(StrictAppendError::NotCommitted)?;
        let binding = &projection.binding;
        let record = summary
            .completion_delivery
            .receiver
            .records
            .get(&binding.delivery_id)
            .ok_or_else(|| {
                StrictAppendError::NotCommitted(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "delivery was not admitted",
                ))
            })?;
        if record.digest != binding.payload_digest {
            return Err(StrictAppendError::NotCommitted(io::Error::new(
                io::ErrorKind::InvalidInput,
                "delivery digest mismatch",
            )));
        }
        let existing = self
            .delivery_bindings(info)
            .map_err(StrictAppendError::NotCommitted)?
            .into_iter()
            .find(|existing| existing.delivery_id == binding.delivery_id);
        if let Some(existing) = existing {
            if existing.payload_digest != binding.payload_digest {
                return Err(StrictAppendError::NotCommitted(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "delivery history digest mismatch",
                )));
            }
            Self::sync_file_durable(
                &OpenOptions::new()
                    .read(true)
                    .open(&path)
                    .map_err(StrictAppendError::Indeterminate)?,
            )
            .map_err(StrictAppendError::Indeterminate)?;
            Self::sync_parent_directory(&path).map_err(StrictAppendError::Indeterminate)?;
            return Ok(CompletionProjectionAck::AlreadyPresent {
                history: self
                    .read_chat_history_sync(path, CHAT_FORMAT_VERSION)
                    .map_err(StrictAppendError::Indeterminate)?,
            });
        }
        let payload = match &record.execution {
            DeliveryExecution::Queued { payload, .. } => payload,
            DeliveryExecution::Processing { .. }
            | DeliveryExecution::Interrupted { .. }
            | DeliveryExecution::Succeeded
            | DeliveryExecution::Failed { .. }
            | DeliveryExecution::Cancelled { .. } => {
                return Err(StrictAppendError::NotCommitted(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "delivery already projected; execution replay is prohibited",
                )));
            }
        };
        if !matches!(&projection.item, ConversationItem::User(_))
            || projection.item.text_content() != *payload
        {
            return Err(StrictAppendError::NotCommitted(io::Error::new(
                io::ErrorKind::InvalidInput,
                "delivery projection differs from admitted payload",
            )));
        }
        mutate_locked(
            &self.summary_file(info),
            &self.summary_lock_file(info),
            |summary| {
                let record = summary
                    .completion_delivery
                    .receiver
                    .records
                    .get_mut(&binding.delivery_id)
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "admitted delivery disappeared")
                    })?;
                match &record.execution {
                    DeliveryExecution::Queued { payload, .. } => {
                        record.execution = DeliveryExecution::Processing {
                            turn_id: binding.turn_id.clone(),
                            parent_task: binding.parent_task.clone(),
                            payload: payload.clone(),
                        };
                        record.revision = 2;
                        Ok(())
                    }
                    DeliveryExecution::Processing { .. }
                    | DeliveryExecution::Interrupted { .. }
                    | DeliveryExecution::Succeeded
                    | DeliveryExecution::Failed { .. }
                    | DeliveryExecution::Cancelled { .. } => Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "delivery execution is already owned",
                    )),
                }
            },
        )
        .map_err(StrictAppendError::NotCommitted)?;
        let mut line = serde_json::to_vec(projection)
            .map_err(|error| StrictAppendError::NotCommitted(io::Error::other(error)))?;
        line.push(b'\n');
        Self::append_jsonl_line_unlocked_with(
            &path,
            line,
            AppendDurability::Durable,
            &mut Self::sync_file_durable,
            || Self::sync_parent_directory(&path),
        )
        .map_err(|error| match error {
            AppendLineError::NotCommitted(source) => StrictAppendError::NotCommitted(source),
            AppendLineError::Committed(source) => StrictAppendError::Committed {
                acknowledgement: CompletionProjectionAck::Appended,
                source,
            },
        })?;
        mutate_locked(
            &self.summary_file(info),
            &self.summary_lock_file(info),
            |summary| {
                summary.num_chat_messages = summary.num_chat_messages.saturating_add(1);
                Ok(())
            },
        )
        .map_err(|source| StrictAppendError::Committed {
            acknowledgement: CompletionProjectionAck::Appended,
            source,
        })?;
        Ok(CompletionProjectionAck::Appended)
    }

    pub(super) fn finish_delivery_turn_sync(
        &self,
        info: &Info,
        turn_id: &str,
        parent_task: &xai_chat_state::ParentTask,
        origin: &crate::session::delivery_receiver::ReceiverTurnOrigin,
        progress: &DeliveryProgress,
    ) -> io::Result<Vec<DeliveryReceipt>> {
        mutate_locked(
            &self.summary_file(info),
            &self.summary_lock_file(info),
            |summary| {
                let ids = summary.completion_delivery.receiver.settle_turn(
                    turn_id,
                    parent_task,
                    origin,
                    progress,
                );
                Ok(ids
                    .into_iter()
                    .filter_map(|id| {
                        summary
                            .completion_delivery
                            .receiver
                            .records
                            .get(&id)
                            .map(|record| {
                                record.receipt(&info.id.to_string(), &id, ReceiverPresence::Live)
                            })
                    })
                    .collect())
            },
        )
    }

    pub(super) async fn recover_deliveries_owned(&self, info: &Info) -> io::Result<Summary> {
        let storage = self.clone();
        let info = info.clone();
        tokio::task::spawn_blocking(move || {
            let _chat_lock = Self::lock_append(&storage.chat_file(&info))?;
            mutate_locked(
                &storage.summary_file(&info),
                &storage.summary_lock_file(&info),
                |summary| {
                    storage.reconcile_delivery_terminals(&info, summary, ReceiverPresence::Live)?;
                    for record in summary.completion_delivery.receiver.records.values_mut() {
                        if let DeliveryExecution::Processing {
                            turn_id,
                            payload,
                            parent_task,
                        } = &record.execution
                        {
                            record.execution = DeliveryExecution::Interrupted {
                                turn_id: turn_id.clone(),
                                parent_task: parent_task.clone(),
                                reason:
                                    crate::session::delivery_receiver::RECOVERY_INTERRUPTION_REASON
                                        .into(),
                                payload: payload.clone(),
                            };
                            record.revision = record.revision.saturating_add(1).max(3);
                        }
                    }
                    Ok(summary.clone())
                },
            )
        })
        .await
        .map_err(io::Error::other)?
    }
}
