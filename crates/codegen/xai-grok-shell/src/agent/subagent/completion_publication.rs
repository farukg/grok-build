use super::*;
use super::spawn::will_wake_for;
use crate::session::completion_delivery::{CompletionArtifacts, CompletionEventPolicy, SubagentCompletionEvent};
use xai_grok_tools::implementations::grok_build::task::coordinator::ChildCompletion;

pub(super) fn completion_event(completion: &ChildCompletion<ShellCompletionData>) -> SubagentCompletionEvent {
    let summary = xai_grok_tools::implementations::grok_build::task::completion_summary(
        &completion.attempt_id, &completion.request, &completion.result, &completion.snapshot,
    );
    let data = &completion.completion_data;
    let message = xai_grok_tools::reminders::task_completion::format_subagent_completion(
        &summary, Some(&data.task_output_tool_name), data.scheduler_delete_tool_name.as_deref(),
        data.scheduler_create_tool_name.as_deref(),
    );
    let policy = match completion.disposition.reporting {
        xai_grok_tools::implementations::grok_build::task::coordinator::CompletionReporting::Reportable => {
            if data.auto_wake_enabled { CompletionEventPolicy::Automatic } else { CompletionEventPolicy::BoundaryOnly }
        }
        xai_grok_tools::implementations::grok_build::task::coordinator::CompletionReporting::Silent => CompletionEventPolicy::Silent,
    };
    SubagentCompletionEvent {
        id: xai_chat_state::SubagentCompletionId {
            subagent_id: completion.request.id.clone(), attempt_id: completion.attempt_id.clone(),
        },
        state: completion.result.state.clone(),
        text: xai_grok_tools::reminders::wrap_reminder(&message),
        output: completion.result.output.to_string(),
        child_session_id: completion.result.child_session_id.clone(),
        tool_calls: completion.result.tool_calls,
        turns: completion.result.turns,
        duration_ms: completion.result.duration_ms,
        tokens_used: data.telemetry_tokens(),
        completed_at: chrono::Utc::now(),
        policy,
        artifacts: data.completion_artifacts(),
    }
}

pub(crate) fn completion_lifecycle_notification_from_event(
    event: &SubagentCompletionEvent,
    parent_session_id: &str,
    will_wake: bool,
) -> std::io::Result<SessionNotification> {
    let mut meta = None;
    crate::util::event_id::ensure_event_id_meta(parent_session_id, &mut meta);
    let mut meta = meta.unwrap_or_default();
    meta.insert("subagentCompletion".into(), serde_json::to_value(crate::session::completion_delivery::SubagentCompletionFact {
        id: event.id.clone(), state: event.state.clone(),
    }).map_err(std::io::Error::other)?);
    Ok(SessionNotification {
        session_id: acp::SessionId::new(parent_session_id.to_owned()),
        update: SessionUpdate::SubagentFinished {
            subagent_id: event.id.subagent_id.clone(),
            attempt_id: Some(event.id.attempt_id.to_string()),
            child_session_id: event.child_session_id.clone(),
            status: event.state.legacy_status().to_owned(),
            error: xai_grok_tools::implementations::grok_build::task::types::completion_error(&event.state),
            tool_calls: event.tool_calls, turns: event.turns, duration_ms: event.duration_ms,
            tokens_used: event.tokens_used,
            output: matches!(event.state, xai_grok_tools::implementations::grok_build::task::types::SubagentState::Completed).then(|| event.output.clone()),
            will_wake,
        },
        meta: Some(serde_json::Value::Object(meta)),
    })
}

pub(super) async fn prepare_completion_admission(completion: &ChildCompletion<ShellCompletionData>) -> std::io::Result<()> {
    let data = &completion.completion_data;
    let sender = data.parent_persistence_tx.as_ref().ok_or_else(|| std::io::Error::new(
        std::io::ErrorKind::BrokenPipe, "child result has no live parent persistence owner",
    ))?;
    let (respond_to, receive) = oneshot::channel();
    sender.send(crate::session::persistence::PersistenceMsg::PrepareSubagentCompletion {
        event: completion_event(completion), respond_to,
    }).map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "parent persistence owner is closed"))?;
    let ack = receive.await.map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "producer commit acknowledgement lost"))??;
    data.upload_committed_metadata(&completion.result, &ack);
    data.remember_prepared_producer(ack);
    Ok(())
}

pub(super) async fn publish_committed_completion(completion: ChildCompletion<ShellCompletionData>, gateway: GatewaySender) -> xai_grok_tools::implementations::grok_build::task::coordinator::TerminalVisibilityResult {
    let will_wake = will_wake_for(&completion);
    let data = &completion.completion_data;
    let Some(prepared) = data.prepared_producer() else {
        tracing::error!(subagent_id = %completion.request.id, "producer publication has no committed outcome");
        return xai_grok_tools::implementations::grok_build::task::coordinator::TerminalVisibilityResult::Deferred { error: "missing prepared producer outcome".into() };
    };
    let result = match &data.parent_cmd_tx {
        Some(sender) => {
            let (respond_to, receive) = oneshot::channel();
            match sender.send(crate::session::SessionCommand::PublishSubagentCompletion {
                event: (*prepared.event).clone(), will_wake, respond_to,
            }) {
                Ok(()) => receive.await.map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "completion publication acknowledgement lost"))
                    .and_then(|result| result),
                Err(_) => Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "parent persistence owner closed before publication")),
            }
        }
        None => Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "parent persistence owner unavailable")),
    };
    let visibility = match &result {
        Ok(_) => xai_grok_tools::implementations::grok_build::task::coordinator::TerminalVisibilityResult::Published,
        Err(error) => xai_grok_tools::implementations::grok_build::task::coordinator::TerminalVisibilityResult::Deferred { error: error.to_string() },
    };
    let (notification, text, offer) = match result {
        Ok(published) => (published.notification, published.text, published.offer),
        Err(error) => {
            tracing::error!(subagent_id = %completion.request.id, %error, "completion history projection is pending recovery");
            let Ok(mut notification) = completion_lifecycle_notification_from_event(&prepared.event, &completion.request.parent_session_id, will_wake) else { return visibility; };
            if let Some(serde_json::Value::Object(meta)) = &mut notification.meta {
                meta.remove("eventId");
                meta.insert("completionPublication".into(), serde_json::json!("awaitingHistory"));
            }
            let text = crate::session::completion_delivery::completion_text_with_reference(&prepared.event,
                &crate::session::completion_delivery::CompletionResultReference::Prepared { path: prepared.summary_path });
            (notification, text, crate::session::completion_delivery::CompletionOffer::pending(prepared.event.policy))
        }
    };
    super::spawn::publish_child_completion(completion, &gateway, offer, notification, text);
    visibility
}

pub(super) fn report_commit_failure(completion: ChildCompletion<ShellCompletionData>, gateway: &GatewaySender, error: &str) {
    let text = format!("Completion {}/{} could not be durably admitted. {} Persistence error: {error}",
        completion.request.id, completion.attempt_id, completion.result.state.model_text());
    let notification = acp::SessionNotification::new(acp::SessionId::new(completion.request.parent_session_id),
        acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(text)))));
    if let Ok(params) = serde_json::value::to_raw_value(&notification) {
        gateway.forward_fire_and_forget(acp::ExtNotification::new("session/update", params.into()));
    }
}

#[cfg(test)]
pub(crate) fn present_child_completion(completion: ChildCompletion<ShellCompletionData>, gateway: &GatewaySender, will_wake: bool) {
    let mut event = completion_event(&completion);
    if let CompletionArtifacts::Metadata { metadata } = &mut event.artifacts {
        metadata.status = SubagentMetaStatus::of_state(&event.state);
        metadata.completed_at = Some(event.completed_at);
        metadata.duration_ms = Some(event.duration_ms);
        metadata.tool_calls = Some(event.tool_calls);
        metadata.turns = Some(event.turns);
        metadata.error = xai_grok_tools::implementations::grok_build::task::types::completion_error(&event.state);
    }
    let artifacts = completion.completion_data.state.lock().terminal_artifacts.clone();
    match artifacts {
        TerminalArtifacts::Unprepared => {}
        TerminalArtifacts::StartPublished { dir, .. } | TerminalArtifacts::Prepared { dir, .. } => {
            super::metadata_store::materialize_at(&dir, &event).expect("fixture terminal metadata");
        }
    }
    let notification = completion_lifecycle_notification_from_event(&event, &completion.request.parent_session_id, will_wake).expect("fixture completion notification");
    let text = event.text;
    if let Some(sender) = &completion.completion_data.parent_cmd_tx {
        let _ = sender.send(SessionCommand::SubagentCompletionPublished { notification: notification.clone() });
    }
    let offer = if will_wake { crate::session::completion_delivery::CompletionOffer::Automatic } else { crate::session::completion_delivery::CompletionOffer::None };
    super::spawn::publish_child_completion(completion, gateway, offer, notification, text);
}
