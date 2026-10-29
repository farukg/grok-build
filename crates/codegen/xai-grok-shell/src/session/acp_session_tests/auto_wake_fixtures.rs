use super::*;

pub(super) fn bash_completed_notification(task_id: &str) -> PendingNotification {
    PendingNotification {
        prompt_id: format!("bash-completed-{task_id}"),
        prompt_blocks: vec![],
        priority: NotificationPriority::Later,
        source: NotificationSource::BashTaskCompleted {
            task_id: task_id.to_string(),
        },
    }
}

pub(super) fn monitor_completed_notification(task_id: &str) -> PendingNotification {
    PendingNotification {
        prompt_id: format!("monitor-completed-{task_id}"),
        prompt_blocks: vec![],
        priority: NotificationPriority::Later,
        source: NotificationSource::MonitorCompleted {
            task_id: task_id.to_string(),
        },
    }
}

pub(super) fn task_wake_admission(
    task_id: &str,
    source: NotificationSource,
) -> (
    crate::session::commands::TaskWakeAdmission,
    oneshot::Receiver<bool>,
) {
    let (respond_to, response_rx) = oneshot::channel();
    (
        crate::session::commands::TaskWakeAdmission {
            respond_to,
            fallback: crate::session::commands::TaskWakeFallback {
                prompt_id: format!("deferred-{task_id}"),
                prompt_blocks: vec![acp::ContentBlock::Text(acp::TextContent::new(format!(
                    "completion {task_id}"
                )))],
                source,
            },
        },
        response_rx,
    )
}

pub(super) fn monitor_event_notification(task_id: &str) -> PendingNotification {
    PendingNotification {
        prompt_id: format!("monitor-{task_id}"),
        prompt_blocks: vec![],
        priority: NotificationPriority::Next,
        source: NotificationSource::MonitorEvent {
            task_id: task_id.to_string(),
        },
    }
}
