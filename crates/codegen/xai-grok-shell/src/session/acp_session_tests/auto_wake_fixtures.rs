use super::*;

pub(super) fn task_output_result(task_id: &str, status: &str) -> TaskOutputResult {
    TaskOutputResult {
        task_id: task_id.to_string(),
        command: "echo test".into(),
        status: status.to_string(),
        exit_code: Some(0),
        started: "2026-01-01T00:00:00Z".into(),
        ended: Some("2026-01-01T00:00:01Z".into()),
        duration_secs: 1.0,
        output: "test".into(),
        output_file: "/tmp/out.log".into(),
        truncated: false,
        truncation_hint: String::new(),
        raw_output_bytes: 4,
    }
}

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
