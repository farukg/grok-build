use super::*;
use crate::session::commands::{TaskCompletionBatch, TaskWakeAdmission, TaskWakeFallback};
use xai_grok_tools::computer::types::{TaskKind, TaskSnapshot};

pub(super) async fn handle_task_completions(
    config: &NotificationBridgeConfig,
    snapshots: Vec<TaskSnapshot>,
) {
    let goal_active = config.goal_loop_active.load(Ordering::Acquire);
    let automatic = config.auto_wake_enabled || goal_active;
    let mut publications = Vec::new();
    let mut admissions = Vec::new();
    let mut responses = Vec::new();
    for snapshot in snapshots {
        if !xai_grok_tools::reminders::task_completion::task_owned_by_session(
            &snapshot,
            Some(config.session_id.0.as_ref()),
        ) {
            tracing::warn!(task_id = %snapshot.task_id, "terminal publication belongs to another session");
            continue;
        }
        let task_id = snapshot.task_id.clone();
        let source = match snapshot.kind {
            TaskKind::Bash => NotificationSource::BashTaskCompleted {
                task_id: task_id.clone(),
            },
            TaskKind::Monitor => NotificationSource::MonitorCompleted {
                task_id: task_id.clone(),
            },
        };
        let fallback = TaskWakeFallback {
            prompt_id: match snapshot.kind {
                TaskKind::Bash => format!("bash-completed-{task_id}"),
                TaskKind::Monitor => format!("monitor-completed-{task_id}"),
            },
            prompt_blocks: vec![acp::ContentBlock::from(terminal_completion_text(
                config, &snapshot,
            ))],
            source,
        };
        if snapshot.is_auto_wake_suppressed() {
            publications.push((snapshot, None));
            continue;
        }
        config.task_completion_reservations.reserve(task_id);
        let (respond_to, response) = tokio::sync::oneshot::channel();
        publications.push((snapshot, Some(responses.len())));
        responses.push(response);
        admissions.push(TaskWakeAdmission {
            respond_to,
            fallback,
        });
    }
    let prompt_id = match admissions.as_slice() {
        [admission] => format!("task-completed-{}", admission.fallback.source.task_id()),
        [] => String::new(),
        _ => format!("notifications-{}", uuid::Uuid::now_v7()),
    };
    let (respond_to, completion_rx) = tokio::sync::oneshot::channel();
    let enqueued = !admissions.is_empty()
        && config
            .session_cmd_tx
            .send(SessionCommand::TaskCompletionBatch(TaskCompletionBatch {
                prompt_id: prompt_id.clone(),
                automatic,
                admissions,
                respond_to,
            }))
            .is_ok();
    if !enqueued {
        for (snapshot, response) in &publications {
            if response.is_some() {
                config
                    .task_completion_reservations
                    .release(&snapshot.task_id);
            }
        }
    }
    // All receipts share one budget; a stalled actor cannot serialize a burst into N timeouts.
    let deadline = tokio::time::Instant::now() + TASK_WAKE_ADMISSION_TIMEOUT;
    let accepted = futures::future::join_all(responses.into_iter().map(|response| async move {
        tokio::time::timeout_at(deadline, response)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(false)
    }))
    .await;
    if accepted.iter().any(|accepted| *accepted) {
        let trace_tx = config
            .synthetic_trace_tx
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(trace_tx) = trace_tx {
            let (respond_to, before_session_copy_rx) = tokio::sync::oneshot::channel();
            if config
                .session_cmd_tx
                .send(SessionCommand::CopyFile { respond_to })
                .is_ok()
            {
                let _ = trace_tx.send(crate::upload::turn::SyntheticTurnTraceRequest {
                    session_id: config.session_id.clone(),
                    prompt_id,
                    completion_rx,
                    before_session_copy_rx,
                });
            }
        }
    }
    for (snapshot, response) in publications {
        let will_wake = response
            .and_then(|index| accepted.get(index))
            .copied()
            .unwrap_or(false);
        if will_wake && snapshot.kind == TaskKind::Monitor {
            let _ = config
                .session_cmd_tx
                .send(SessionCommand::DropMonitorNotifications {
                    task_id: snapshot.task_id.clone(),
                });
        }
        emit_task_completion(config, snapshot, will_wake);
    }
}

fn emit_task_completion(
    config: &NotificationBridgeConfig,
    snapshot: TaskSnapshot,
    will_wake: bool,
) {
    let _publication = config.gateway.begin_publication();
    let task_id = snapshot.task_id.clone();
    let mut notification = crate::extensions::notification::SessionNotification {
        session_id: config.session_id.clone(),
        update: crate::extensions::notification::SessionUpdate::TaskCompleted {
            task_snapshot: snapshot,
            will_wake,
        },
        meta: None,
    };
    let mut meta = None;
    stamp_event_id(config, &mut meta);
    notification.meta = meta.map(serde_json::Value::Object);
    if let Some(params) = crate::tools::task_completed_frame::encode(&mut notification) {
        let _ = config.persistence.tx.send(PersistenceMsg::Update(
            crate::session::storage::SessionUpdate::Xai(Box::new(notification)),
        ));
        config
            .gateway
            .forward_fire_and_forget(acp::ExtNotification::new(
                crate::tools::task_completed_frame::METHOD,
                params.into_inner().into(),
            ));
    } else {
        config.gateway.fail_publication();
    }
    let _ = config
        .session_cmd_tx
        .send(SessionCommand::DispatchNotificationHook {
            notification_type: "task_complete".into(),
            message: Some(format!("Background task completed: {task_id}")),
            title: None,
            level: Some("info".into()),
        });
    request_background_tasks_snapshot(config, SessionActivityProvenance::Live);
}
