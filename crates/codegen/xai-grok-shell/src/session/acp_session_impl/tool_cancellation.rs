//! Native dispatch cancellation delivery.
use super::*;

enum CancellationStage {
    Preparation,
    Dispatch(std::time::Instant),
}

enum DispatchCompletion<'a> {
    Executing {
        writer: xai_grok_session_events::EventWriter,
        tool_name: &'a str,
        tool_call_id: &'a str,
        stage: CancellationStage,
        owner: crate::extensions::tool_timing::ToolCallOwnership,
    },
    Settled,
}

pub(super) struct DispatchCompletionGuard<'a>(DispatchCompletion<'a>);

impl<'a> DispatchCompletionGuard<'a> {
    pub(super) fn new(
        writer: xai_grok_session_events::EventWriter,
        tool_name: &'a str,
        tool_call_id: &'a str,
        started: std::time::Instant,
        owner: crate::extensions::tool_timing::ToolCallOwnership,
    ) -> Self {
        Self(DispatchCompletion::Executing {
            writer,
            tool_name,
            tool_call_id,
            stage: CancellationStage::Dispatch(started),
            owner,
        })
    }

    pub(super) fn preparing(
        writer: xai_grok_session_events::EventWriter,
        tool_name: &'a str,
        tool_call_id: &'a str,
        owner: crate::extensions::tool_timing::ToolCallOwnership,
    ) -> Self {
        Self(DispatchCompletion::Executing {
            writer,
            tool_name,
            tool_call_id,
            stage: CancellationStage::Preparation,
            owner,
        })
    }

    pub(super) fn settle(&mut self) {
        self.0 = DispatchCompletion::Settled;
    }
}

impl Drop for DispatchCompletionGuard<'_> {
    fn drop(&mut self) {
        match &self.0 {
            DispatchCompletion::Executing {
                writer,
                tool_name,
                tool_call_id,
                stage,
                owner,
            } => {
                let duration_ms = match stage {
                    CancellationStage::Preparation => 0,
                    CancellationStage::Dispatch(started) => started.elapsed().as_millis() as u64,
                };
                writer.cancel_tool(tool_call_id, owner);
                writer.emit(crate::session::events::Event::ToolCompleted {
                    tool_name: tool_name.to_string(),
                    tool_call_id: tool_call_id.to_string(),
                    duration_ms,
                    outcome: crate::session::events::ToolOutcome::Cancelled,
                    source: crate::session::events::ToolCompletedSource::Shell,
                    rewriting_hook: None,
                });
            }
            DispatchCompletion::Settled => {}
        }
    }
}

pub(super) fn native_error_outcome(
    kind: xai_tool_runtime::ToolErrorKind,
) -> crate::session::events::ToolOutcome {
    use crate::session::events::ToolOutcome;
    use xai_tool_runtime::ToolErrorKind;
    match kind {
        ToolErrorKind::Cancelled => ToolOutcome::Cancelled,
        ToolErrorKind::NotImplemented
        | ToolErrorKind::InvalidArguments
        | ToolErrorKind::NotFound
        | ToolErrorKind::PermissionDenied
        | ToolErrorKind::Unauthorized
        | ToolErrorKind::Timeout
        | ToolErrorKind::RateLimited
        | ToolErrorKind::UsagePoolExhausted
        | ToolErrorKind::UsageLimitReached
        | ToolErrorKind::GlobalRateLimit
        | ToolErrorKind::ConcurrencyLimit
        | ToolErrorKind::ServiceUnavailable
        | ToolErrorKind::NetworkError
        | ToolErrorKind::Execution
        | ToolErrorKind::BehaviorVersionUnsupported
        | ToolErrorKind::RenderLimited
        | ToolErrorKind::TerminalError
        | ToolErrorKind::Custom => ToolOutcome::Error,
    }
}

pub(super) fn execution_observation(
    writer: xai_grok_session_events::EventWriter,
    prepared: Arc<PreparedToolCall>,
) -> xai_tool_runtime::execution::ToolExecutionObservation {
    xai_tool_runtime::execution::ToolExecutionObservation(Arc::new(move |timing| {
        writer.emit(crate::session::events::Event::ToolExecution {
            tool_call_id: prepared.tool_call_id.0.to_string(),
            owner: prepared.ownership(),
            timing,
        });
    }))
}

impl SessionActor {
    pub(super) fn install_tool_lifecycle_bridge(&self) {
        self.events.writer().on_tool_lifecycle(|| {
            let event_tx = self.event_tx.clone();
            let session_id = self.session_info.id.clone();
            let current_actions = self.current_actions.clone();
            Arc::new(move |event| {
                let _publication = current_actions.begin_publication();
                use crate::session::acp_types::ToolTerminalStatus;
                use xai_grok_session_events::log::ToolLifecycleEvent;
                let (tool_call_id, fields, meta, owner) = match event {
                    ToolLifecycleEvent::Cancelled {
                        tool_call_id,
                        owner,
                    } => {
                        if let crate::extensions::tool_timing::ToolCallOwnership::Registered {
                            invocation_id,
                            ..
                        } = owner
                        {
                            current_actions.settle_tool(invocation_id);
                        }
                        let mut meta = ToolTerminalStatus::Cancelled.meta();
                        if crate::extensions::tool_timing::stamp_ownership(&mut meta, owner)
                            .is_err()
                        {
                            return;
                        }
                        (
                            tool_call_id,
                            acp::ToolCallUpdateFields::new()
                                .status(Some(acp::ToolCallStatus::Failed)),
                            meta,
                            owner,
                        )
                    }
                    ToolLifecycleEvent::Execution {
                        tool_call_id,
                        owner,
                        timing,
                    } => {
                        current_actions.tool_execution(owner, *timing);
                        let Ok(mut meta) = crate::extensions::tool_timing::timing_meta(*timing)
                        else {
                            return;
                        };
                        if crate::extensions::tool_timing::stamp_ownership(&mut meta, owner)
                            .is_err()
                        {
                            return;
                        }
                        (tool_call_id, acp::ToolCallUpdateFields::new(), meta, owner)
                    }
                };
                let update =
                    acp::ToolCallUpdate::new(acp::ToolCallId::new(tool_call_id.to_owned()), fields)
                        .meta(Some(meta));
                let mut notification_meta = acp::Meta::new();
                if let Some(version) = current_actions.version()
                    && let Ok(version) = serde_json::to_value(version)
                {
                    notification_meta.insert(
                        crate::session::current_actions::VERSION_META_KEY.into(),
                        version,
                    );
                }
                notification_meta.insert(
                    "eventId".into(),
                    crate::util::event_id::generate_event_id(&session_id.0).into(),
                );
                notification_meta.insert(
                    "agentTimestampMs".into(),
                    chrono::Utc::now().timestamp_millis().into(),
                );
                crate::extensions::tool_timing::stamp_original_source(
                    &mut notification_meta,
                    owner,
                );
                let notification = acp::SessionNotification::new(
                    session_id.clone(),
                    acp::SessionUpdate::ToolCallUpdate(update),
                )
                .meta(Some(notification_meta));
                let _ = event_tx.send(SessionEvent::Notification(notification.into()));
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::replay_buffer_send_update_tests::make_replay_send_update_fixture;
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn native_cancellation_enters_the_acp_replay_rail_but_error_text_does_not() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let mut fixture = make_replay_send_update_fixture().await;
                fixture.actor.install_tool_lifecycle_bridge();
                for outcome in [
                    crate::session::events::ToolOutcome::Error,
                    crate::session::events::ToolOutcome::Cancelled,
                    crate::session::events::ToolOutcome::PermissionCancelled,
                ] {
                    fixture.actor.events.writer().emit(
                        crate::session::events::Event::ToolCompleted {
                            tool_name: "opaque_tool".into(),
                            tool_call_id: "native-call".into(),
                            duration_ms: 1,
                            outcome,
                            source: crate::session::events::ToolCompletedSource::Shell,
                            rewriting_hook: None,
                        },
                    );
                    assert!(
                        fixture.event_rx.try_recv().is_err(),
                        "canonical tool events cannot publish a second terminal"
                    );
                }
                let writer = fixture.actor.events.writer();
                let mut settled = DispatchCompletionGuard::new(
                    writer.clone(),
                    "opaque_tool",
                    "settled",
                    std::time::Instant::now(),
                    crate::extensions::tool_timing::ToolCallOwnership::Unknown,
                );
                settled.settle();
                drop(settled);
                assert!(fixture.event_rx.try_recv().is_err());
                drop(DispatchCompletionGuard::new(
                    writer,
                    "opaque_tool",
                    "dropped",
                    std::time::Instant::now(),
                    crate::extensions::tool_timing::ToolCallOwnership::Unknown,
                ));
                let SessionEvent::Notification(notification) = fixture.event_rx.try_recv().unwrap()
                else {
                    panic!("aborted dispatch must publish native cancellation");
                };
                fixture.actor.emit_buffered(notification.clone()).await;
                let mut persisted = Vec::new();
                while let Ok(message) = fixture.persistence_rx.try_recv() {
                    if let PersistenceMsg::Update(crate::session::storage::SessionUpdate::Acp(
                        notification,
                    )) = message
                    {
                        persisted.push(notification);
                    }
                }
                assert_eq!(persisted.len(), 1);
                assert!(
                    fixture.event_rx.try_recv().is_err(),
                    "drop must publish exactly one canonical terminal"
                );
                let acp::SessionUpdate::ToolCallUpdate(update) = notification.into_acp().update
                else {
                    panic!("aborted dispatch must publish its tool identity");
                };
                assert_eq!(update.tool_call_id.0.as_ref(), "dropped");
                assert_eq!(
                    update.meta.as_ref().unwrap().get("toolTerminalStatus"),
                    Some(&serde_json::json!("cancelled"))
                );
                for kind in [
                    xai_tool_runtime::ToolErrorKind::Execution,
                    xai_tool_runtime::ToolErrorKind::Cancelled,
                ] {
                    let error: anyhow::Error =
                        xai_tool_runtime::ToolError::new(kind, "cancelled, identical diagnostic")
                            .into();
                    fixture
                        .actor
                        .handle_tool_error(
                            &acp::ToolCallId::new("typed-error"),
                            "typed-error",
                            "opaque_tool",
                            None,
                            &error,
                            "fixture-model",
                            None,
                        )
                        .await;
                    let SessionEvent::Notification(notification) =
                        fixture.event_rx.try_recv().unwrap()
                    else {
                        panic!("error must publish a notification");
                    };
                    let acp::SessionUpdate::ToolCallUpdate(update) = notification.into_acp().update
                    else {
                        panic!("error must publish tool completion");
                    };
                    let terminal = update
                        .meta
                        .as_ref()
                        .and_then(|meta| meta.get("toolTerminalStatus"));
                    match kind {
                        xai_tool_runtime::ToolErrorKind::Execution => assert_eq!(terminal, None),
                        xai_tool_runtime::ToolErrorKind::Cancelled => {
                            assert_eq!(terminal, Some(&serde_json::json!("cancelled")))
                        }
                        other => panic!("unexpected fixture error {other:?}"),
                    }
                }
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn permission_cancelled_prepare_emits_explicit_tool_terminal_metadata() {
        use super::super::super::support::test_agent_with_tools;
        use xai_grok_tools::registry::types::ToolConfig;
        use xai_grok_workspace::permission::{
            PermissionCommand, PermissionHandle, PermissionResolution,
        };
        tokio::task::LocalSet::new()
            .run_until(async {
                let mut fixture = make_replay_send_update_fixture().await;
                *fixture.actor.agent.borrow_mut() =
                    test_agent_with_tools(vec![ToolConfig::from_id(
                        "GrokBuild:read_file".to_owned(),
                    )])
                    .await;
                let (cmd_tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
                fixture.actor.permissions = PermissionHandle::Actor {
                    cmd_tx,
                    yolo_state: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    auto_state: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    side_query_wired: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    yolo_pin: None,
                    deny_read_globs: Arc::new(vec![]),
                    in_flight: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                    user_prompt_notify: Arc::new(parking_lot::Mutex::new(None)),
                };
                tokio::task::spawn_local(async move {
                    while let Some(command) = commands.recv().await {
                        if let PermissionCommand::Request { respond_to, .. } = command {
                            let _ = respond_to.send(PermissionResolution {
                                decision: Decision::Cancelled,
                                event: None,
                            });
                        }
                    }
                });
                let call = crate::sampling::types::ToolCallResponse {
                    id: "permission-cancelled".into(),
                    kind: "function".into(),
                    function: crate::sampling::types::ToolCallFunction {
                        name: "read_file".into(),
                        arguments: r#"{"target_file":"/tmp/fixture.rs"}"#.into(),
                    },
                };
                assert!(matches!(
                    fixture
                        .actor
                        .prepare_tool_call(call, &mut Vec::new(), None, None)
                        .await,
                    Ok(Err(ToolLoop::Cancelled))
                ));
                let mut terminals = Vec::new();
                while let Ok(event) = fixture.event_rx.try_recv() {
                    if let SessionEvent::Notification(notification) = event
                        && let acp::SessionUpdate::ToolCallUpdate(update) =
                            notification.into_acp().update
                        && update.fields.status == Some(acp::ToolCallStatus::Failed)
                    {
                        terminals.push(update);
                    }
                }
                assert_eq!(terminals.len(), 1);
                assert_eq!(terminals[0].tool_call_id.0.as_ref(), "permission-cancelled");
                assert_eq!(
                    terminals[0]
                        .meta
                        .as_ref()
                        .unwrap()
                        .get("toolTerminalStatus"),
                    Some(&serde_json::json!("cancelled"))
                );
            })
            .await;
    }
}
