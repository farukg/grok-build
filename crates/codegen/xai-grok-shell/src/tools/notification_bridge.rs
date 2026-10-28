//! Translates `xai-grok-tools` `ToolNotification` events into `xai-grok-shell`'s native systems (ACP gateway, hunk tracker, file state tracker).
use crate::session::acp_types::SessionActivityProvenance;
use crate::session::commands::SessionCommand;
use crate::session::commands::{NotificationPriority, NotificationSource};
use crate::session::persistence::{DurableAppendError, PersistenceHandle, PersistenceMsg};
#[path = "task_completion_publication.rs"]
mod task_publication;
use agent_client_protocol::{self as acp, Client as _};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use task_publication::handle_task_completions;
use tokio::sync::{Mutex as TokioMutex, mpsc};
use xai_acp_lib::AcpAgentGatewaySender as GatewaySender;
use xai_grok_tools::notification::types::{ToolNotification, ToolNotificationHandle};
use xai_grok_tools::types::output::{BashOutput, ToolOutput};
use xai_grok_workspace::session::file_state::FileStateTracker;
use xai_hunk_tracker::HunkTrackerHandle;
const TASK_WAKE_ADMISSION_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

#[derive(Hash, PartialEq, Eq)]
enum BashStreamKey {
    Legacy(String),
    Registered(String),
}

impl BashStreamKey {
    fn new(
        provider_call_id: &str,
        owner: &crate::extensions::tool_timing::ToolCallOwnership,
    ) -> Self {
        match owner {
            crate::extensions::tool_timing::ToolCallOwnership::Unknown => {
                Self::Legacy(provider_call_id.to_owned())
            }
            crate::extensions::tool_timing::ToolCallOwnership::Registered {
                invocation_id, ..
            } => Self::Registered(invocation_id.clone()),
        }
    }
}
pub(crate) struct NotificationBridgeConfig {
    pub current_actions: crate::session::current_actions::Source,
    /// ACP gateway for sending streaming updates to TUI
    pub gateway: GatewaySender,
    pub session_id: acp::SessionId,
    /// Hunk tracker for recording agent writes
    pub hunk_tracker_handle: HunkTrackerHandle,
    /// File state tracker for rewind functionality
    pub file_state_tracker: Arc<FileStateTracker>,
    /// Current prompt index (shared with session state)
    pub prompt_index: Arc<TokioMutex<usize>>,
    /// Working directory for path relativization
    pub cwd: PathBuf,
    /// Shared gate: when false, suppress gateway forwarding.
    /// Events are still processed for hunk tracking and file state.
    pub gateway_enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Persistence handle for FIFO ordinary writes and durable tombstone barriers.
    pub persistence: PersistenceHandle,
    /// When true, send incremental `output_delta` instead of full `output` in bash streaming updates.
    /// The client must opt in via the `x.ai/incrementalBashOutput` capability.
    pub incremental_bash_output: bool,
    /// Plan mode tracker shared with the session actor.
    /// Used to transition state on `PlanModeEntered` / `PlanModeExited` tool notifications.
    pub plan_mode: Arc<parking_lot::Mutex<crate::session::plan_mode::PlanModeTracker>>,
    /// Session-level prompt mode shared with the session actor.
    /// Updated on `PlanModeEntered` / `PlanModeExited` and `session/set_mode` so the next turn starts in the correct mode.
    pub current_prompt_mode: Arc<parking_lot::Mutex<crate::session::plan_mode::PromptMode>>,
    /// Set at turn start, then updated only by agent tool calls (`EnterPlanMode` / `ExitPlanMode`).
    /// NOT affected by `session/set_mode`.
    /// Read at turn end for `end_prompt_mode`.
    pub turn_prompt_mode: Arc<parking_lot::Mutex<crate::session::plan_mode::PromptMode>>,
    /// Session command channel for monitor events and task-completed injections.
    pub session_cmd_tx: mpsc::UnboundedSender<SessionCommand>,
    pub task_completion_reservations:
        xai_grok_tools::reminders::task_completion::TaskCompletionReservations,
    pub task_wake_suppressed: xai_grok_tools::reminders::task_completion::TaskWakeSuppressed,
    /// Channel for requesting trace uploads for synthetic auto-wake turns.
    /// Wrapped in `Arc<Mutex<..>>` because the coordinator creates the channel after the notification bridge is spawned.
    /// The bridge reads the latest value on each notification.
    pub(crate) synthetic_trace_tx: Arc<
        std::sync::Mutex<
            Option<
                tokio::sync::mpsc::UnboundedSender<crate::upload::turn::SyntheticTurnTraceRequest>,
            >,
        >,
    >,
    /// Resolved name of the `BackgroundTaskAction` tool. Written exactly once after the agent's toolset is finalized. Read many times thereafter from the notification bridge and the session actor's between-turn drain.
    /// `None` means no such tool is registered in this toolset, which is a valid resolved state.
    pub task_output_tool_name: Arc<std::sync::OnceLock<Option<String>>>,
    /// Resolved name of the `Read` tool, used by `format_bash_completion`'s footer.
    /// The footer points the model at `task.output_file` so it can recover full bash output even when no polling tool is available.
    /// Written once and then only read, like `task_output_tool_name`.
    pub read_tool_name: Arc<std::sync::OnceLock<Option<String>>>,
    /// When `false`, bash task completions fall back to the idle-gated `InjectNotification` path instead of immediate synthetic prompts.
    pub auto_wake_enabled: bool,
    /// When `true`, an approved `PlanModeExited` also queues the tracker's next-turn exit reminder.
    /// Grok-build leaves this `false`: its exit-plan tool result already informs the model, and a deferred reminder would arrive stale. Shared with the session actor (the `gateway_enabled` pattern).
    /// Refreshed on zero-turn rebuilds so the bridge always agrees with the live session gate.
    pub queue_exit_reminder_on_approved_exit: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// When `true`, suppress the bash auto-wake synthetic prompt.
    /// Shared `Arc` written in one place; see `SessionActor::set_goal_loop_active_resource` for the rationale.
    pub goal_loop_active: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Coalesce live `EmitBackgroundTasksSnapshot` requests. Last-wins only
    /// needs the latest list; a burst of completions shares one emit.
    pub background_tasks_snapshot_pending: std::sync::Arc<AtomicBool>,
    /// When `false`, suppress local `background_tasks` snapshot emits (live
    /// and actor). Gateway-backed sessions keep remote Running on their own
    /// rail; an empty local registry must not last-wins-clear it.
    /// Default `true` for local sessions; flipped off when
    /// `session_computer_sessions` is non-empty (load/new/mid-session add).
    pub emit_local_background_tasks: std::sync::Arc<AtomicBool>,
}
/// Returns `None` if the slot is unset (toolset not yet finalized) or if the resolved value is `None` (no such tool registered in this toolset).
pub(crate) fn resolved_tool_name(slot: &std::sync::OnceLock<Option<String>>) -> Option<&str> {
    slot.get().and_then(|v| v.as_deref())
}
/// Stamp a bridge-emitted notification's meta before it forks into persistence and broadcast; see `util::event_id::ensure_event_id_meta`.
fn stamp_event_id(config: &NotificationBridgeConfig, meta: &mut Option<acp::Meta>) {
    crate::util::event_id::ensure_event_id_meta(&config.session_id.0, meta);
    config
        .current_actions
        .stamp_delivery(meta.get_or_insert_with(acp::Meta::new));
}
fn request_background_tasks_snapshot(
    config: &NotificationBridgeConfig,
    provenance: SessionActivityProvenance,
) {
    if !config.emit_local_background_tasks.load(Ordering::Acquire) {
        return;
    }
    if config
        .background_tasks_snapshot_pending
        .swap(true, Ordering::AcqRel)
    {
        return;
    }
    let _ = config
        .session_cmd_tx
        .send(SessionCommand::EmitBackgroundTasksSnapshot {
            provenance,
            respond_to: None,
            pending: Some(Arc::clone(&config.background_tasks_snapshot_pending)),
        });
}
fn stamp_scheduler_meta(
    config: &NotificationBridgeConfig,
    meta: &mut Option<acp::Meta>,
    generation: &str,
    revision: u64,
) {
    stamp_event_id(config, meta);
    let meta = meta.get_or_insert_with(acp::Meta::new);
    meta.insert("x.ai/schedulerGeneration".to_owned(), generation.into());
    meta.insert("x.ai/schedulerRevision".to_owned(), revision.into());
}
fn durable_append_landed(result: Result<(), DurableAppendError>) -> Result<(), String> {
    match result {
        Ok(()) => Ok(()),
        Err(DurableAppendError::Committed(error)) => {
            tracing::warn!(%error, "Scheduler tombstone committed with bookkeeping failure");
            Ok(())
        }
        Err(DurableAppendError::NotCommitted(error)) => {
            Err(format!("scheduler tombstone was not committed: {error}"))
        }
        Err(DurableAppendError::AcknowledgementLost(error)) => Err(format!(
            "scheduler tombstone commit status is unknown: {error}"
        )),
    }
}
async fn handle_scheduled_task_removed(
    config: &NotificationBridgeConfig,
    removed: xai_grok_tools::notification::ScheduledTaskRemoved,
    acknowledgement: Option<tokio::sync::oneshot::Sender<Result<(), String>>>,
) -> Result<(), String> {
    let mut publication = config.gateway.begin_publication();
    publication.require_delivery();
    tracing::info!(task_id = %removed.task_id, "Scheduled task removed");
    let result: Result<Box<serde_json::value::RawValue>, String> = async {
        let mut meta = None;
        stamp_scheduler_meta(config, &mut meta, &removed.generation, removed.revision);
        let notification = crate::extensions::notification::SessionNotification {
            session_id: config.session_id.clone(),
            update: crate::extensions::notification::SessionUpdate::ScheduledTaskDeleted {
                task_id: removed.task_id,
                reason: removed.reason,
            },
            meta: meta.map(serde_json::Value::Object),
        };
        let params = serde_json::to_value(&notification)
            .and_then(|value| serde_json::value::to_raw_value(&value))
            .map_err(|error| format!("failed to serialize scheduled task deletion: {error}"))?;
        let update = crate::session::storage::SessionUpdate::Xai(Box::new(notification));
        if acknowledgement.is_some() {
            durable_append_landed(config.persistence.append_update_durably(update).await)?;
        } else {
            config
                .persistence
                .tx
                .send(PersistenceMsg::Update(update))
                .map_err(|_| "session persistence stopped".to_owned())?;
        }
        Ok(params)
    }
    .await;
    match result {
        Ok(params) => {
            if let Some(acknowledgement) = acknowledgement {
                let _ = acknowledgement.send(Ok(()));
            }
            config
                .gateway
                .forward_fire_and_forget(acp::ExtNotification::new(
                    "x.ai/scheduled_task_deleted",
                    params.into(),
                ));
            publication.enqueued();
            Ok(())
        }
        Err(error) => {
            if let Some(acknowledgement) = acknowledgement {
                let _ = acknowledgement.send(Err(error.clone()));
            }
            Err(error)
        }
    }
}
fn terminal_completion_text(
    config: &NotificationBridgeConfig,
    snapshot: &xai_grok_tools::computer::types::TaskSnapshot,
) -> String {
    match snapshot.kind {
        xai_grok_tools::computer::types::TaskKind::Bash => {
            xai_grok_tools::reminders::task_completion::format_bash_completion(
                snapshot,
                resolved_tool_name(&config.task_output_tool_name),
                resolved_tool_name(&config.read_tool_name),
            )
        }
        xai_grok_tools::computer::types::TaskKind::Monitor => {
            xai_grok_tools::reminders::task_completion::format_monitor_completion(
                snapshot,
                resolved_tool_name(&config.task_output_tool_name),
            )
        }
    }
}

async fn classify_terminal_goal_policy(
    config: &NotificationBridgeConfig,
    notification: crate::session::commands::TaskWakeFallback,
) -> Result<crate::session::commands::TerminalNotificationDecision, String> {
    let (respond_to, response) = tokio::sync::oneshot::channel();
    config
        .session_cmd_tx
        .send(SessionCommand::ClassifyTerminalNotification {
            notification,
            respond_to,
        })
        .map_err(|_| "terminal goal policy owner closed".to_string())?;
    response
        .await
        .map_err(|_| "terminal goal policy acknowledgement lost".to_string())?
        .map_err(|error| format!("terminal goal policy failed: {error}"))
}

async fn enroll_task_completion(
    config: &NotificationBridgeConfig,
    snapshot: xai_grok_tools::computer::types::TaskSnapshot,
) -> Result<(), String> {
    use crate::session::completion_delivery::{CompletionEventPolicy, CompletionPayloadSource};
    let text = terminal_completion_text(config, &snapshot);
    let policy = if snapshot.is_auto_wake_suppressed() {
        CompletionEventPolicy::Silent
    } else if config.auto_wake_enabled {
        CompletionEventPolicy::Automatic
    } else {
        CompletionEventPolicy::BoundaryOnly
    };
    let policy_notification = if !snapshot.is_auto_wake_suppressed() {
        Some(crate::session::commands::TaskWakeFallback {
            prompt_id: format!("task-completed-{}", snapshot.task_id),
            prompt_blocks: vec![acp::ContentBlock::from(text.clone())],
            source: match snapshot.kind {
                xai_grok_tools::computer::types::TaskKind::Bash => {
                    NotificationSource::BashTaskCompleted {
                        task_id: snapshot.task_id.clone(),
                    }
                }
                xai_grok_tools::computer::types::TaskKind::Monitor => {
                    NotificationSource::MonitorCompleted {
                        task_id: snapshot.task_id.clone(),
                    }
                }
            },
        })
    } else {
        None
    };
    let (respond_to, response) = tokio::sync::oneshot::channel();
    config
        .persistence
        .tx
        .send(PersistenceMsg::EnrollTerminalCompletion {
            source: CompletionPayloadSource::Task(Box::new(snapshot)),
            text,
            policy,
            respond_to,
        })
        .map_err(|_| "terminal enrollment owner closed".to_string())?;
    response
        .await
        .map_err(|_| "terminal enrollment acknowledgement lost".to_string())?
        .map_err(|error| format!("terminal enrollment failed: {error}"))?;
    if let Some(notification) = policy_notification {
        classify_terminal_goal_policy(config, notification).await?;
    }
    Ok(())
}

pub(crate) fn spawn_notification_bridge(
    config: NotificationBridgeConfig,
) -> ToolNotificationHandle {
    let (handle, mut rx) = ToolNotificationHandle::acknowledged_channel();
    tokio::task::spawn_local(async move {
        let mut offsets: HashMap<BashStreamKey, usize> = HashMap::new();
        while let Some(delivery) = rx.recv().await {
            let mut deliveries = Vec::with_capacity(rx.len() + 1);
            deliveries.push(delivery);
            for _ in 0..rx.len() {
                if let Ok(delivery) = rx.try_recv() {
                    deliveries.push(delivery);
                }
            }
            let cohort_ids: Vec<_> = deliveries
                .iter()
                .filter_map(|delivery| {
                    if delivery.acknowledgement.is_some() {
                        return None;
                    }
                    if let ToolNotification::TaskCompleted(snapshot) = &delivery.notification
                        && xai_grok_tools::reminders::task_completion::task_owned_by_session(
                            snapshot,
                            Some(config.session_id.0.as_ref()),
                        )
                    {
                        config
                            .task_completion_reservations
                            .reserve(snapshot.task_id.clone());
                        Some(snapshot.task_id.clone())
                    } else {
                        None
                    }
                })
                .collect();
            let mut completions = Vec::new();
            for delivery in deliveries {
                let acknowledgement = delivery.acknowledgement;
                match delivery.notification {
                    ToolNotification::TaskCompleted(snapshot) if acknowledgement.is_some() => {
                        let result = enroll_task_completion(&config, snapshot).await;
                        if let Some(acknowledgement) = acknowledgement {
                            let _ = acknowledgement.send(result);
                        }
                    }
                    ToolNotification::TaskCompleted(snapshot) => completions.push(snapshot),
                    ToolNotification::ScheduledTaskRemoved(removed) => {
                        if let Err(error) =
                            handle_scheduled_task_removed(&config, removed, acknowledgement).await
                        {
                            tracing::warn!(%error, "Failed to handle scheduled task removal");
                        }
                    }
                    notification => {
                        handle_notification(
                            &config,
                            &delivery.tool_ownership,
                            notification,
                            &mut offsets,
                        )
                        .await;
                        if let Some(acknowledgement) = acknowledgement {
                            let _ = acknowledgement.send(Ok(()));
                        }
                    }
                }
            }
            if !completions.is_empty() {
                handle_task_completions(&config, completions).await;
            }
            for task_id in cohort_ids {
                config.task_completion_reservations.release(&task_id);
            }
        }
        tracing::debug!("Notification bridge task exiting (sender dropped)");
    });
    handle
}
/// The update is persisted to `updates.jsonl` so session replay re-applies the mode, and forwarded to the gateway so the pager updates live.
async fn emit_current_mode_update(
    config: &NotificationBridgeConfig,
    mode: xai_grok_tools::types::SessionMode,
) {
    let _publication = config.gateway.begin_publication();
    let mut notification = acp::SessionNotification::new(
        config.session_id.clone(),
        acp::SessionUpdate::CurrentModeUpdate(acp::CurrentModeUpdate::new(
            acp::SessionModeId::new(mode.as_id()),
        )),
    );
    stamp_event_id(config, &mut notification.meta);
    let _ = config.persistence.tx.send(PersistenceMsg::Update(
        crate::session::storage::SessionUpdate::Acp(Box::new(notification.clone())),
    ));
    config.gateway.forward_fire_and_forget(notification);
}
async fn handle_notification(
    config: &NotificationBridgeConfig,
    owner: &crate::extensions::tool_timing::ToolCallOwnership,
    notification: ToolNotification,
    offsets: &mut HashMap<BashStreamKey, usize>,
) {
    let _publication = config.gateway.begin_publication();
    match notification {
        ToolNotification::BashOutputChunk(chunk) => {
            let key = BashStreamKey::new(&chunk.base.tool_call_id, owner);
            let (output, output_delta) = if config.incremental_bash_output {
                let prev_offset = offsets.get(&key).copied().unwrap_or(0);
                let full = &chunk.base.output;
                let delta = full
                    .get(prev_offset..)
                    .map(|s| s.to_vec())
                    .unwrap_or_else(|| full.clone());
                offsets.insert(key, full.len());
                (Vec::new(), Some(delta))
            } else {
                (chunk.base.output.clone(), None)
            };
            let bash_output = ToolOutput::Bash(BashOutput {
                output_for_prompt: BashOutput::make_output_for_prompt(&String::from_utf8_lossy(
                    &chunk.base.output,
                )),
                output,
                exit_code: 0,
                command: chunk.base.command.clone(),
                truncated: chunk.base.truncated,
                signal: None,
                timed_out: false,
                description: None,
                current_dir: chunk.base.cwd.to_string_lossy().to_string(),
                output_file: String::new(),
                total_bytes: chunk.base.total_bytes,
                output_delta,
                was_bare_echo: false,
            });
            let mut tool_meta = acp::Meta::new();
            if let Err(error) =
                crate::extensions::tool_timing::stamp_ownership(&mut tool_meta, owner)
            {
                tracing::error!(%error, "invalid streaming tool ownership");
                return;
            }
            let update = acp::SessionUpdate::ToolCallUpdate(
                acp::ToolCallUpdate::new(
                    acp::ToolCallId::new(chunk.base.tool_call_id.clone()),
                    acp::ToolCallUpdateFields::new()
                        .status(Some(acp::ToolCallStatus::InProgress))
                        .content(Some(vec![acp::ToolCallContent::from(
                            acp::ContentBlock::Text(acp::TextContent::new(
                                String::from_utf8_lossy(&chunk.base.output).into_owned(),
                            )),
                        )]))
                        .raw_output(serde_json::to_value(&bash_output).ok()),
                )
                .meta(Some(tool_meta)),
            );
            let mut notification = acp::SessionNotification::new(config.session_id.clone(), update);
            crate::extensions::tool_timing::stamp_original_source(
                notification.meta.get_or_insert_with(acp::Meta::new),
                owner,
            );
            config
                .current_actions
                .stamp_delivery(notification.meta.get_or_insert_with(acp::Meta::new));
            if config
                .gateway_enabled
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                let _ = config.gateway.session_notification(notification).await;
            }
        }
        ToolNotification::BashExecutionComplete(complete) => {
            offsets.remove(&BashStreamKey::new(&complete.base.tool_call_id, owner));
            tracing::debug!(
                tool_call_id = %complete.base.tool_call_id,
                exit_code = ?complete.exit_code,
                "Bash execution complete notification received"
            );
        }
        ToolNotification::BashExecutionTimeout(timeout) => {
            tracing::debug!(
                tool_call_id = %timeout.base.tool_call_id,
                elapsed = ?timeout.elapsed,
                "Bash execution timeout notification received"
            );
        }
        ToolNotification::BashExecutionFailed(failed) => {
            tracing::warn!(
                tool_call_id = %failed.tool_call_id,
                error = %failed.error,
                "Bash execution failed notification received"
            );
        }
        ToolNotification::BashExecutionBackgrounded(bg) => {
            tracing::debug!(
                tool_call_id = %bg.base.tool_call_id,
                task_id = %bg.task_id,
                command = %bg.base.command,
                output_file = %bg.output_file.display(),
                "Bash execution backgrounded notification received — forwarding to TUI"
            );
            let mut notification = crate::extensions::notification::SessionNotification {
                session_id: config.session_id.clone(),
                update: crate::extensions::notification::SessionUpdate::TaskBackgrounded {
                    start_time: bg.base.start_time,
                    tool_call_id: bg.base.tool_call_id.clone(),
                    task_id: bg.task_id.clone(),
                    command: bg.base.command.clone(),
                    cwd: bg.base.cwd.to_string_lossy().to_string(),
                    output_file: bg.output_file.to_string_lossy().to_string(),
                    monitor_description: bg.monitor_description.clone(),
                    description: bg.description.clone(),
                },
                meta: None,
            };
            {
                let mut meta = acp::Meta::new();
                if let Err(error) =
                    crate::extensions::tool_timing::stamp_ownership(&mut meta, owner)
                {
                    tracing::error!(%error, "invalid backgrounded tool ownership");
                    return;
                }
                crate::extensions::tool_timing::stamp_original_source(&mut meta, owner);
                let mut meta_map = Some(meta);
                stamp_event_id(config, &mut meta_map);
                notification.meta = meta_map.map(serde_json::Value::Object);
            }
            let _ = config.persistence.tx.send(PersistenceMsg::Update(
                crate::session::storage::SessionUpdate::Xai(Box::new(notification.clone())),
            ));
            let params = serde_json::to_value(&notification)
                .and_then(|v| serde_json::value::to_raw_value(&v))
                .ok();
            if let Some(params) = params {
                let ext_notification =
                    acp::ExtNotification::new("x.ai/task_backgrounded", params.into());
                config.gateway.forward_fire_and_forget(ext_notification);
            } else {
                config.gateway.fail_publication();
            }
            request_background_tasks_snapshot(config, SessionActivityProvenance::Live);
        }
        ToolNotification::FileWritten(written) => {
            let prompt_index = *config.prompt_index.lock().await;
            config.hunk_tracker_handle.record_agent_write(
                written.absolute_path.clone(),
                written.content.clone(),
                prompt_index,
                written.previous_content.clone(),
            );
            if written.previous_content.is_some() || written.is_new_file {
                config
                    .file_state_tracker
                    .add_before_snapshot_for_prompt(
                        prompt_index,
                        &written.absolute_path,
                        &config.cwd,
                        written.previous_content,
                    )
                    .await;
            }
            tracing::debug!(
                path = %written.absolute_path.display(),
                is_new_file = written.is_new_file,
                "FileWritten notification forwarded to hunk tracker"
            );
        }
        ToolNotification::SubagentCompleted(_) => {}
        ToolNotification::TaskCompleted(task_snapshot) => {
            handle_task_completions(config, vec![task_snapshot]).await;
        }
        ToolNotification::PlanModeEntered(entered) => {
            let activated = config.plan_mode.lock().activate_from_tool();
            if activated {
                *config.current_prompt_mode.lock() = crate::session::plan_mode::PromptMode::Plan;
                *config.turn_prompt_mode.lock() = crate::session::plan_mode::PromptMode::Plan;
                let snapshot = config.plan_mode.lock().snapshot();
                let _ = config
                    .persistence
                    .tx
                    .send(PersistenceMsg::PlanModeState(snapshot));
                emit_current_mode_update(config, xai_grok_tools::types::SessionMode::Plan).await;
            }
            tracing::info!(
                tool_call_id = %entered.tool_call_id,
                activated,
                "Plan mode entered via EnterPlanMode tool"
            );
        }
        ToolNotification::PlanModeExited(exited) => {
            let deactivated = {
                let mut tracker = config.plan_mode.lock();
                let deactivated = tracker.deactivate_approved();
                if deactivated
                    && config
                        .queue_exit_reminder_on_approved_exit
                        .load(std::sync::atomic::Ordering::Relaxed)
                {
                    tracker.queue_exit_reminder();
                }
                deactivated
            };
            if deactivated {
                *config.current_prompt_mode.lock() = crate::session::plan_mode::PromptMode::Agent;
                *config.turn_prompt_mode.lock() = crate::session::plan_mode::PromptMode::Agent;
                let snapshot = config.plan_mode.lock().snapshot();
                let _ = config
                    .persistence
                    .tx
                    .send(PersistenceMsg::PlanModeState(snapshot));
                emit_current_mode_update(config, xai_grok_tools::types::SessionMode::Default).await;
            }
            tracing::info!(
                tool_call_id = %exited.tool_call_id,
                deactivated,
                has_plan = exited.plan_content.is_some(),
                "Plan mode exited via ExitPlanMode tool"
            );
        }
        ToolNotification::UserQuestionAsked(asked) => {
            tracing::info!(
                tool_call_id = %asked.tool_call_id,
                "User question asked"
            );
        }
        ToolNotification::LspServerStarting(s) => {
            tracing::debug!(server = %s.server_name, command = %s.command, "LSP server starting");
        }
        ToolNotification::LspServerReady(s) => {
            tracing::info!(server = %s.server_name, "LSP server ready");
        }
        ToolNotification::LspServerCrashed(s) => {
            tracing::warn!(server = %s.server_name, "LSP server crashed");
        }
        ToolNotification::LspServerRetrying(s) => {
            tracing::warn!(
                server = %s.server_name,
                attempt = s.attempt,
                max_restarts = s.max_restarts,
                backoff_ms = s.backoff_ms,
                "LSP server retrying"
            );
        }
        ToolNotification::LspServerFailed(s) => {
            tracing::error!(server = %s.server_name, error = %s.error, "LSP server failed");
        }
        ToolNotification::ScheduledTaskFired(fired) => {
            tracing::info!(
                task_id = %fired.task_id,
                schedule = %fired.human_schedule,
                subagent_id = fired.subagent_id.as_deref().unwrap_or(""),
                "Scheduled task fired"
            );
            let mut meta = None;
            stamp_scheduler_meta(config, &mut meta, &fired.generation, fired.revision);
            let fired_notif = crate::extensions::notification::SessionNotification {
                session_id: config.session_id.clone(),
                update: crate::extensions::notification::SessionUpdate::ScheduledTaskFired {
                    task_id: fired.task_id,
                    prompt: fired.prompt,
                    human_schedule: fired.human_schedule,
                    next_fire_at: fired.next_fire_at,
                    subagent_id: fired.subagent_id,
                },
                meta: meta.map(serde_json::Value::Object),
            };
            if let Ok(params) =
                serde_json::to_value(&fired_notif).and_then(|v| serde_json::value::to_raw_value(&v))
            {
                config
                    .gateway
                    .forward_fire_and_forget(acp::ExtNotification::new(
                        "x.ai/scheduled_task_fired",
                        params.into(),
                    ));
            } else {
                config.gateway.fail_publication();
            }
        }
        ToolNotification::MonitorEvent(event) => {
            let my_session = config.session_id.0.as_ref();
            if let Some(owner) = event.owner_session_id.as_deref()
                && owner != my_session
            {
                tracing::warn!(
                    task_id = %event.task_id,
                    description = %event.description,
                    monitor_owner = %owner,
                    bridge_session = %my_session,
                    "Dropped cross-session monitor event: owner does not match this bridge's session"
                );
                return;
            }
            tracing::debug!(
                task_id = %event.task_id,
                description = %event.description,
                "Monitor event received, injecting into session"
            );
            let notification = crate::extensions::notification::SessionNotification {
                session_id: config.session_id.clone(),
                update: crate::extensions::notification::SessionUpdate::MonitorEvent {
                    task_id: event.task_id.clone(),
                    description: event.description.clone(),
                    event_text: event.raw_text.clone(),
                },
                meta: None,
            };
            let params = serde_json::to_value(&notification)
                .and_then(|v| serde_json::value::to_raw_value(&v))
                .ok();
            if let Some(params) = params {
                config
                    .gateway
                    .forward_fire_and_forget(acp::ExtNotification::new(
                        "x.ai/monitor_event",
                        params.into(),
                    ));
            } else {
                config.gateway.fail_publication();
            }
            if config.task_completion_reservations.contains(&event.task_id) {
                tracing::debug!(
                    task_id = %event.task_id,
                    "skipping model inject for monitor event: task already auto-woke via TaskCompleted"
                );
                return;
            }
            let prompt_id = format!("monitor-{}-{}", event.task_id, uuid::Uuid::now_v7());
            let prompt_blocks = vec![acp::ContentBlock::Text(acp::TextContent::new(
                event.event_text,
            ))];
            let _ = config
                .session_cmd_tx
                .send(SessionCommand::InjectNotification {
                    prompt_id,
                    prompt_blocks,
                    priority: NotificationPriority::Next,
                    source: NotificationSource::MonitorEvent {
                        task_id: event.task_id.clone(),
                    },
                });
        }
        ToolNotification::ScheduledTaskRemoved(removed) => {
            if let Err(error) = handle_scheduled_task_removed(config, removed, None).await {
                tracing::warn!(%error, "Failed to handle scheduled task removal");
            }
        }
        ToolNotification::ScheduledTaskCreated(created) => {
            tracing::info!(task_id = %created.task_id, "Scheduled task created");
            let mut meta = None;
            stamp_scheduler_meta(config, &mut meta, &created.generation, created.revision);
            let notification = crate::extensions::notification::SessionNotification {
                session_id: config.session_id.clone(),
                update: crate::extensions::notification::SessionUpdate::ScheduledTaskCreated {
                    task_id: created.task_id,
                    prompt: created.prompt,
                    human_schedule: created.human_schedule,
                    next_fire_at: created.next_fire_at,
                },
                meta: meta.map(serde_json::Value::Object),
            };
            let _ = config.persistence.tx.send(PersistenceMsg::Update(
                crate::session::storage::SessionUpdate::Xai(Box::new(notification.clone())),
            ));
            if let Ok(params) = serde_json::to_value(&notification)
                .and_then(|v| serde_json::value::to_raw_value(&v))
            {
                config
                    .gateway
                    .forward_fire_and_forget(acp::ExtNotification::new(
                        "x.ai/scheduled_task_created",
                        params.into(),
                    ));
            } else {
                config.gateway.fail_publication();
            }
        }
    }
}
#[cfg(test)]
#[path = "notification_bridge_tests.rs"]
pub(crate) mod tests;
