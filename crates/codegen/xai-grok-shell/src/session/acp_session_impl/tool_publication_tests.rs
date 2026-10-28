use super::*;
use crate::extensions::tool_timing::{ToolCallOwnership, ToolCallSource, ownership_from_meta};
use crate::session::current_actions::{CurrentActions, CurrentActionsSnapshot, SnapshotBinding};

fn snapshot(actor: &SessionActor) -> CurrentActions {
    let binding = SnapshotBinding {
        session_id: actor.session_info.id.0.to_string(),
        binding_epoch: 1,
        load_generation: 1,
    };
    let CurrentActionsSnapshot::Current(current) = actor.current_actions_reader().snapshot(binding)
    else {
        panic!("resident source must be available");
    };
    current
}

fn assert_publications(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<SessionEvent>,
    expected: &ToolCallSource,
    executions: usize,
) -> ToolCallOwnership {
    let mut registered = None;
    let mut bases = 0;
    let mut terminals = 0;
    let mut timings = 0;
    let mut refinements = 0;
    while let Ok(event) = events.try_recv() {
        let SessionEvent::Notification(SessionNotification::Acp(notification)) = event else {
            continue;
        };
        let meta = match &notification.update {
            acp::SessionUpdate::ToolCall(call) => {
                bases += 1;
                call.meta.as_ref()
            }
            acp::SessionUpdate::ToolCallUpdate(update) => {
                if matches!(
                    update.fields.status,
                    Some(acp::ToolCallStatus::Completed | acp::ToolCallStatus::Failed)
                ) {
                    terminals += 1;
                }
                if update.fields.kind.is_some() {
                    refinements += 1;
                }
                if timing_from_meta(update.meta.as_ref()).unwrap() != ToolExecutionTiming::Unknown {
                    timings += 1;
                }
                update.meta.as_ref()
            }
            _ => continue,
        };
        let owner = ownership_from_meta(meta).unwrap();
        let ToolCallOwnership::Registered {
            invocation_id,
            provider_call_id,
            source,
        } = &owner
        else {
            panic!("every ordinary producer publication needs its actual registration");
        };
        assert!(!uuid::Uuid::parse_str(invocation_id).unwrap().is_nil());
        assert_eq!(provider_call_id, "reused-provider");
        assert_eq!(source, expected);
        match &registered {
            Some(registered) => assert_eq!(&owner, registered),
            None => registered = Some(owner.clone()),
        }
        let outer = notification.meta.as_ref().unwrap();
        match expected {
            ToolCallSource::Sampling {
                prompt_id,
                request_id,
                attempt,
            } => {
                assert_eq!(outer.get("promptId"), Some(&serde_json::json!(prompt_id)));
                assert_eq!(outer.get("requestId"), Some(&serde_json::json!(request_id)));
                assert_eq!(outer.get("streamEpoch"), Some(&serde_json::json!(attempt)));
            }
            ToolCallSource::Unknown => {
                assert!(!outer.contains_key("promptId"));
                assert!(!outer.contains_key("requestId"));
                assert!(!outer.contains_key("streamEpoch"));
            }
        }
    }
    assert_eq!(bases, 1);
    assert_eq!(
        terminals, 1,
        "execution settlement and native ToolCompleted cannot duplicate the canonical outcome"
    );
    assert_eq!(timings, executions);
    assert!(
        refinements > 0,
        "the actual preparation must refine the registered kind"
    );
    registered.unwrap()
}

fn original_source(owner: &crate::extensions::sampling_lifecycle::SamplingOwner) -> ToolCallSource {
    ToolCallSource::Sampling {
        prompt_id: owner.prompt_id.clone(),
        request_id: owner.request_id.as_str().to_owned(),
        attempt: owner.attempt.0,
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn held_postflight_keeps_canonical_pending_and_original_owner_after_prompt_changes() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut fixture = make_replay_send_update_fixture().await;
            install_fixture_tool(
                &fixture.actor,
                ToolKind::ImageGen,
                TimingFixtureSchedule::Immediate,
            );
            let (gateway_tx, mut gateway_rx) = tokio::sync::mpsc::unbounded_channel();
            fixture.actor.notifications.gateway = GatewaySender::new(gateway_tx);
            fixture.actor.client_hooks.borrow_mut().insert(
                xai_grok_hooks::event::HookEventName::PostToolUse,
                vec![crate::extensions::hooks::ClientHookGroup {
                    matcher: None,
                    callback_ids: vec!["held-postflight".into()],
                    timeout: Some(Duration::from_secs(300)),
                }],
            );
            *fixture.actor.current_prompt_id.lock().unwrap() = Some("first-prompt".into());
            let first_request = xai_grok_sampler::RequestId::random();
            fixture.actor.register_sampling_request(&first_request);
            let first_owner = fixture.actor.sampling_owner(&first_request).unwrap();
            let actor = &fixture.actor;
            let execute =
                actor.execute_tool_calls(vec![call("reused-provider")], None, Some(&first_owner));
            let observe = async {
                let request = loop {
                    if let xai_acp_lib::AcpClientMessage::ExtMethod(request) =
                        gateway_rx.recv().await.unwrap()
                    {
                        break request;
                    }
                };
                let pending = snapshot(actor);
                let [tool] = pending.tools.as_slice() else {
                    panic!("postflight still owns the actual pending call");
                };
                assert!(matches!(
                    tool.executions.as_slice(),
                    [ToolExecutionTiming::Finished {
                        duration_ms: 2000,
                        ..
                    }]
                ));
                tokio::time::advance(Duration::from_secs(30)).await;
                let later = snapshot(actor);
                assert_eq!(
                    later.tools, pending.tools,
                    "postflight wait cannot extend the execution span"
                );
                assert_eq!(later.version, pending.version);
                *actor.current_prompt_id.lock().unwrap() = Some("second-prompt".into());
                actor.begin_sampling_turn();
                let second_request = xai_grok_sampler::RequestId::random();
                actor.register_sampling_request(&second_request);
                let second_owner = actor.sampling_owner(&second_request).unwrap();
                request
                    .response_tx
                    .send(Ok(acp::ExtResponse::new(
                        serde_json::value::to_raw_value(
                            &serde_json::json!({"decision":"continue"}),
                        )
                        .unwrap()
                        .into(),
                    )))
                    .unwrap();
                second_owner
            };
            let (result, second_owner) = tokio::join!(execute, observe);
            assert!(result.is_ok());
            let first =
                assert_publications(&mut fixture.event_rx, &original_source(&first_owner), 2);
            assert!(snapshot(actor).tools.is_empty());
            let execute =
                actor.execute_tool_calls(vec![call("reused-provider")], None, Some(&second_owner));
            let respond = async {
                loop {
                    if let xai_acp_lib::AcpClientMessage::ExtMethod(request) =
                        gateway_rx.recv().await.unwrap()
                    {
                        request
                            .response_tx
                            .send(Ok(acp::ExtResponse::new(
                                serde_json::value::to_raw_value(
                                    &serde_json::json!({"decision":"continue"}),
                                )
                                .unwrap()
                                .into(),
                            )))
                            .unwrap();
                        break;
                    }
                }
            };
            let (result, ()) = tokio::join!(execute, respond);
            assert!(result.is_ok());
            let second =
                assert_publications(&mut fixture.event_rx, &original_source(&second_owner), 2);
            assert_ne!(first, second);
            assert!(snapshot(actor).tools.is_empty());
        })
        .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn real_permission_rejection_and_cancellation_keep_original_source_without_execution() {
    tokio::task::LocalSet::new()
        .run_until(async {
            for decision in [Decision::Reject("denied".into()), Decision::Cancelled] {
                let mut fixture = make_replay_send_update_fixture().await;
                install_fixture_tool(
                    &fixture.actor,
                    ToolKind::Other,
                    TimingFixtureSchedule::Immediate,
                );
                let mut permissions = permission_gate(&mut fixture.actor);
                *fixture.actor.current_prompt_id.lock().unwrap() = Some("first-prompt".into());
                let request = xai_grok_sampler::RequestId::random();
                fixture.actor.register_sampling_request(&request);
                let owner = fixture.actor.sampling_owner(&request).unwrap();
                let actor = &fixture.actor;
                let execute =
                    actor.execute_tool_calls(vec![call("reused-provider")], None, Some(&owner));
                let reject = async {
                    let PermissionCommand::Request {
                        request,
                        respond_to,
                    } = permissions.recv().await.unwrap()
                    else {
                        panic!("actual permission request");
                    };
                    let ToolCallOwnership::Registered { source, .. } =
                        ownership_from_meta(request.tool_call_update.meta.as_ref()).unwrap()
                    else {
                        panic!("permission request must retain its real invocation");
                    };
                    assert_eq!(source, original_source(&owner));
                    *actor.current_prompt_id.lock().unwrap() = Some("second-prompt".into());
                    actor.begin_sampling_turn();
                    respond_to
                        .send(PermissionResolution {
                            decision,
                            event: None,
                        })
                        .unwrap();
                };
                let (result, ()) = tokio::join!(execute, reject);
                assert!(result.is_ok());
                assert_publications(&mut fixture.event_rx, &original_source(&owner), 0);
                assert!(snapshot(actor).tools.is_empty());
            }
        })
        .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn dropping_a_real_permission_wait_publishes_one_original_cancellation_without_execution() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut fixture = make_replay_send_update_fixture().await;
            install_fixture_tool(
                &fixture.actor,
                ToolKind::Other,
                TimingFixtureSchedule::Immediate,
            );
            let mut permissions = permission_gate(&mut fixture.actor);
            *fixture.actor.current_prompt_id.lock().unwrap() = Some("first-prompt".into());
            let request = xai_grok_sampler::RequestId::random();
            fixture.actor.register_sampling_request(&request);
            let owner = fixture.actor.sampling_owner(&request).unwrap();
            let mut execute = Box::pin(fixture.actor.execute_tool_calls(
                vec![call("reused-provider")],
                None,
                Some(&owner),
            ));
            let permission = tokio::select! {
                permission = permissions.recv() => permission.unwrap(),
                result = execute.as_mut() => panic!("permission must still be pending: {result:?}"),
            };
            let PermissionCommand::Request { respond_to, .. } = permission else {
                panic!("actual permission request");
            };
            *fixture.actor.current_prompt_id.lock().unwrap() = Some("second-prompt".into());
            fixture.actor.begin_sampling_turn();
            drop(execute);
            assert!(respond_to.is_closed());
            assert_publications(&mut fixture.event_rx, &original_source(&owner), 0);
            assert!(snapshot(&fixture.actor).tools.is_empty());
        })
        .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn dropped_spawned_dispatch_keeps_original_owner_and_unknown_source_does_not_borrow_prompt() {
    tokio::task::LocalSet::new().run_until(async {
        for source_known in [true, false] {
            let mut fixture = make_replay_send_update_fixture().await;
            install_fixture_tool(&fixture.actor, ToolKind::ImageGen, TimingFixtureSchedule::Held(Arc::new(tokio::sync::Notify::new())));
            *fixture.actor.current_prompt_id.lock().unwrap() = Some("first-prompt".into());
            let request = xai_grok_sampler::RequestId::random();
            fixture.actor.register_sampling_request(&request);
            let sampling = source_known.then(|| fixture.actor.sampling_owner(&request).unwrap());
            let expected = sampling.as_ref().map(original_source).unwrap_or(ToolCallSource::Unknown);
            let mut execute = Box::pin(fixture.actor.execute_tool_calls(vec![call("reused-provider")], None, sampling.as_ref()));
            let mut observed = Vec::new();
            loop {
                tokio::select! {
                    event = fixture.event_rx.recv() => {
                        let event = event.unwrap();
                        let started = matches!(&event, SessionEvent::Notification(SessionNotification::Acp(notification))
                            if matches!(&notification.update, acp::SessionUpdate::ToolCallUpdate(update)
                                if matches!(timing_from_meta(update.meta.as_ref()).unwrap(), ToolExecutionTiming::Started { .. })));
                        observed.push(event);
                        if started { break; }
                    }
                    result = execute.as_mut() => panic!("held dispatch returned before drop: {result:?}"),
                }
            }
            *fixture.actor.current_prompt_id.lock().unwrap() = Some("second-prompt".into());
            fixture.actor.begin_sampling_turn();
            tokio::time::advance(Duration::from_millis(500)).await;
            drop(execute);
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            let (tx, mut all) = tokio::sync::mpsc::unbounded_channel();
            for event in observed { tx.send(event).unwrap(); }
            while let Ok(event) = fixture.event_rx.try_recv() { tx.send(event).unwrap(); }
            drop(tx);
            assert_publications(&mut all, &expected, 2);
            assert!(snapshot(&fixture.actor).tools.is_empty());
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn retained_registration_scope_keeps_positive_identity_after_projection_retirement() {
    use crate::session::current_actions::{FrozenOwner, TurnRegistration};
    tokio::task::LocalSet::new()
        .run_until(async {
            for original in [
                FrozenOwner::Root {
                    prompt_id: "first-prompt".into(),
                    task_id: "first-task".into(),
                },
                FrozenOwner::Child {
                    prompt_id: "first-prompt".into(),
                    task_id: "first-task".into(),
                },
            ] {
                let mut fixture = make_replay_send_update_fixture().await;
                install_fixture_tool(
                    &fixture.actor,
                    ToolKind::ImageGen,
                    TimingFixtureSchedule::Immediate,
                );
                let actor = &fixture.actor;
                actor.current_actions.start_turn(
                    original.clone(),
                    xai_monotonic_clock::MonotonicPoint::at(tokio::time::Instant::now()),
                );
                let turn =
                    TurnRegistration::new(actor.current_actions.clone(), "first-task".into());
                *actor.current_prompt_id.lock().unwrap() = Some("first-prompt".into());
                let request = xai_grok_sampler::RequestId::random();
                actor.register_sampling_request(&request);
                let owner = actor.sampling_owner(&request).unwrap();
                let prepared = turn
                    .scope(actor.prepare_tool_call(
                        call("reused-provider"),
                        &mut Vec::new(),
                        None,
                        Some(&owner),
                    ))
                    .await
                    .unwrap()
                    .unwrap();
                let error: anyhow::Error =
                    ToolError::invalid_arguments("rejected fixture input").into();
                prepared
                    .current_action
                    .scope(actor.handle_tool_error(
                        &prepared.tool_call_id,
                        &prepared.call_id,
                        &prepared.tool_name,
                        None,
                        &error,
                        "",
                        None,
                    ))
                    .await;
                assert_publications(&mut fixture.event_rx, &original_source(&owner), 0);
                assert!(
                    snapshot(actor).tools.is_empty(),
                    "canonical terminal removes the active projection"
                );
                assert_eq!(prepared.current_action.owner(), original);
                actor.current_actions.start_turn(
                    FrozenOwner::Root {
                        prompt_id: "second-prompt".into(),
                        task_id: "second-task".into(),
                    },
                    xai_monotonic_clock::MonotonicPoint::at(tokio::time::Instant::now()),
                );
                *actor.current_prompt_id.lock().unwrap() = Some("second-prompt".into());
                assert!(snapshot(actor).tools.is_empty());
                assert_eq!(
                    prepared.current_action.owner(),
                    original,
                    "the live registration, not its mutable projection, owns provenance"
                );
                prepared
                    .current_action
                    .scope(actor.send_update(
                        acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                            prepared.tool_call_id.clone(),
                            acp::ToolCallUpdateFields::new().content(Some(vec![
                                acp::ToolCallContent::from(acp::ContentBlock::Text(
                                    acp::TextContent::new("late content"),
                                )),
                            ])),
                        )),
                        None,
                    ))
                    .await;
                let SessionEvent::Notification(SessionNotification::Acp(notification)) =
                    fixture.event_rx.try_recv().unwrap()
                else {
                    panic!("actual publication");
                };
                let acp::SessionUpdate::ToolCallUpdate(update) = notification.update else {
                    panic!("late content");
                };
                assert_eq!(
                    ownership_from_meta(update.meta.as_ref()).unwrap(),
                    prepared.ownership()
                );
                assert_eq!(
                    notification.meta.as_ref().unwrap().get("promptId"),
                    Some(&serde_json::json!(owner.prompt_id))
                );
                assert!(
                    snapshot(actor).tools.is_empty(),
                    "late content cannot recreate the retired registry entry"
                );
            }
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn backgrounded_publication_keeps_original_registration_after_provider_reuse() {
    tokio::task::LocalSet::new()
        .run_until(async {
            use xai_grok_tools::notification::{BashExecutionBackgrounded, BashNotificationBase};
            let fixture = make_replay_send_update_fixture().await;
            install_fixture_tool(
                &fixture.actor,
                ToolKind::ImageGen,
                TimingFixtureSchedule::Immediate,
            );
            let (mut config, mut gateway_rx, mut persistence_rx, _commands) =
                crate::tools::notification_bridge::tests::make_test_config_full_raw();
            config.session_id = fixture.actor.session_info.id.clone();
            let notifications =
                crate::tools::notification_bridge::spawn_notification_bridge(config);
            let mut registrations = Vec::new();
            for prompt in ["original-prompt", "later-prompt"] {
                *fixture.actor.current_prompt_id.lock().unwrap() = Some(prompt.into());
                fixture.actor.begin_sampling_turn();
                let request = xai_grok_sampler::RequestId::random();
                fixture.actor.register_sampling_request(&request);
                let owner = fixture.actor.sampling_owner(&request).unwrap();
                let prepared = fixture
                    .actor
                    .prepare_tool_call(call("reused-provider"), &mut Vec::new(), None, Some(&owner))
                    .await
                    .unwrap()
                    .unwrap();
                let ownership = prepared.ownership();
                registrations.push((
                    prepared,
                    ownership.clone(),
                    notifications
                        .clone()
                        .with_tool_ownership(Arc::new(ownership)),
                ));
            }
            assert_ne!(registrations[0].1, registrations[1].1);
            for (index, (_, ownership, notification_handle)) in registrations.iter().enumerate() {
                notification_handle.send_backgrounded(BashExecutionBackgrounded {
                    base: BashNotificationBase {
                        start_time: None,
                        tool_call_id: "reused-provider".into(),
                        command: "fixture".into(),
                        output: Vec::new(),
                        total_bytes: 0,
                        truncated: false,
                        cwd: "/tmp".into(),
                    },
                    task_id: format!("original-task-{index}"),
                    output_file: format!("/tmp/original-task-{index}.log").into(),
                    monitor_description: None,
                    description: None,
                });
                let crate::session::persistence::PersistenceMsg::Update(
                    crate::session::storage::SessionUpdate::Xai(saved),
                ) = persistence_rx.recv().await.unwrap()
                else {
                    panic!("actual persisted background event");
                };
                let meta = saved.meta.as_ref().unwrap().as_object().unwrap();
                assert_eq!(ownership_from_meta(Some(meta)).unwrap(), *ownership);
                let crate::extensions::notification::SessionUpdate::TaskBackgrounded {
                    task_id,
                    tool_call_id,
                    output_file,
                    ..
                } = &saved.update
                else {
                    panic!("actual backgrounded task");
                };
                assert_eq!(tool_call_id, "reused-provider");
                assert_eq!(task_id, &format!("original-task-{index}"));
                assert_eq!(output_file, &format!("/tmp/original-task-{index}.log"));
                let xai_acp_lib::AcpClientMessage::ExtNotification(delivered) =
                    gateway_rx.recv().await.unwrap()
                else {
                    panic!("actual native background publication");
                };
                assert_eq!(delivered.request.method.as_ref(), "x.ai/task_backgrounded");
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(delivered.request.params.get())
                        .unwrap(),
                    serde_json::to_value(&*saved).unwrap()
                );
                let _ = delivered.response_tx.send(Ok(()));
            }
        })
        .await;
}

struct PublicationStreamTool {
    notifications: xai_grok_tools::notification::ToolNotificationHandle,
    late: Arc<tokio::sync::Semaphore>,
    calls: std::sync::atomic::AtomicUsize,
}

impl std::fmt::Debug for PublicationStreamTool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PublicationStreamTool")
    }
}

impl Tool for PublicationStreamTool {
    type Args = serde_json::Value;
    type Output = ToolsToolOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("timing_fixture").unwrap()
    }
    fn description(&self, _: &ListToolsContext) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new("timing_fixture", "Publication fixture")
    }
    async fn run(&self, ctx: ToolCallContext, _: Self::Args) -> Result<Self::Output, ToolError> {
        use xai_grok_tools::notification::{BashNotificationBase, BashOutputChunk};
        let owner = ctx.get::<ToolCallOwnership>().unwrap();
        let notifications = self.notifications.clone().with_tool_ownership(owner);
        let output = match self
            .calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        {
            0 => b"first".to_vec(),
            1 => b"second".to_vec(),
            other => panic!("unexpected dispatch {other}"),
        };
        let mut base = BashNotificationBase {
            start_time: None,
            tool_call_id: ctx.call_id.as_str().to_owned(),
            command: "fixture".into(),
            total_bytes: output.len(),
            output,
            truncated: false,
            cwd: "/tmp".into(),
        };
        notifications.send_output_chunk(BashOutputChunk { base: base.clone() });
        let late = self.late.clone();
        tokio::spawn(async move {
            let _permit = late.acquire().await.unwrap();
            base.output.extend_from_slice(b"-tail");
            base.total_bytes = base.output.len();
            notifications.send_output_chunk(BashOutputChunk { base });
        });
        Ok(ToolsToolOutput::Text("fixture result".into()))
    }
}

impl xai_grok_tools::types::tool_metadata::ToolMetadata for PublicationStreamTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }
    fn description_template(&self) -> &str {
        "Publication fixture"
    }
}

#[tokio::test(flavor = "current_thread")]
async fn raw_stream_delivery_keeps_actual_dispatch_owner_across_spawn_and_late_reuse() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut fixture = make_replay_send_update_fixture().await;
            let (mut config, mut gateway_rx, _persistence_rx, _commands) =
                crate::tools::notification_bridge::tests::make_test_config_full_raw();
            config.incremental_bash_output = true;
            let notifications =
                crate::tools::notification_bridge::spawn_notification_bridge(config);
            let late = Arc::new(tokio::sync::Semaphore::new(0));
            let toolset = fixture.actor.agent.borrow().tool_bridge().toolset();
            toolset
                .register_tool(
                    "timing_fixture".into(),
                    PublicationStreamTool {
                        notifications,
                        late: late.clone(),
                        calls: std::sync::atomic::AtomicUsize::new(0),
                    },
                    None,
                )
                .unwrap();
            fixture
                .actor
                .workspace_ops
                .bind_local_session(
                    fixture.actor.session_info.id.0.as_ref(),
                    fixture.actor.tool_context.cwd.to_path_buf(),
                    fixture.actor.tool_context.hunk_tracker_handle.clone(),
                    toolset,
                    None,
                )
                .unwrap();
            let (raw_tx, mut raw_rx) = tokio::sync::mpsc::unbounded_channel();
            tokio::task::spawn_local(async move {
                while let Some(message) = gateway_rx.recv().await {
                    if let xai_acp_lib::AcpClientMessage::SessionNotification(request) = message {
                        raw_tx.send(request.request).unwrap();
                        let _ = request.response_tx.send(Ok(()));
                    }
                }
            });
            let mut owners = Vec::new();
            for prompt in ["first-prompt", "second-prompt"] {
                *fixture.actor.current_prompt_id.lock().unwrap() = Some(prompt.into());
                fixture.actor.begin_sampling_turn();
                let request = xai_grok_sampler::RequestId::random();
                fixture.actor.register_sampling_request(&request);
                let owner = fixture.actor.sampling_owner(&request).unwrap();
                fixture
                    .actor
                    .execute_tool_calls(vec![call("reused-provider")], None, Some(&owner))
                    .await
                    .unwrap();
                owners.push(assert_publications(
                    &mut fixture.event_rx,
                    &original_source(&owner),
                    2,
                ));
            }
            assert_ne!(owners[0], owners[1]);
            for (owner, output) in owners
                .iter()
                .zip([b"first".as_slice(), b"second".as_slice()])
            {
                let notification = raw_rx.recv().await.unwrap();
                assert_raw_publication(notification, owner, output);
            }
            late.add_permits(2);
            let mut received = Vec::new();
            for _ in 0..2 {
                let notification = raw_rx.recv().await.unwrap();
                let acp::SessionUpdate::ToolCallUpdate(update) = &notification.update else {
                    panic!("actual raw update");
                };
                let owner = ownership_from_meta(update.meta.as_ref()).unwrap();
                assert!(owners.contains(&owner));
                assert!(!received.contains(&owner));
                assert_raw_publication(notification, &owner, b"-tail");
                received.push(owner);
            }
            assert!(
                snapshot(&fixture.actor).tools.is_empty(),
                "raw delivery cannot re-register a completed call"
            );
        })
        .await;
}

fn assert_raw_publication(
    notification: acp::SessionNotification,
    owner: &ToolCallOwnership,
    delta: &[u8],
) {
    let acp::SessionUpdate::ToolCallUpdate(update) = notification.update else {
        panic!("actual raw update");
    };
    assert_eq!(ownership_from_meta(update.meta.as_ref()).unwrap(), *owner);
    let ToolCallOwnership::Registered {
        source:
            ToolCallSource::Sampling {
                prompt_id,
                request_id,
                attempt,
            },
        ..
    } = owner
    else {
        panic!("original sampling owner");
    };
    let outer = notification.meta.unwrap();
    assert_eq!(outer.get("promptId"), Some(&serde_json::json!(prompt_id)));
    assert_eq!(outer.get("requestId"), Some(&serde_json::json!(request_id)));
    assert_eq!(outer.get("streamEpoch"), Some(&serde_json::json!(attempt)));
    let output: ToolsToolOutput =
        serde_json::from_value(update.fields.raw_output.unwrap()).unwrap();
    let ToolsToolOutput::Bash(output) = output else {
        panic!("actual typed Bash raw output");
    };
    assert_eq!(output.output_delta.as_deref(), Some(delta));
    assert!(output.output.is_empty());
    assert_eq!(
        timing_from_meta(update.meta.as_ref()).unwrap(),
        ToolExecutionTiming::Unknown
    );
}

#[derive(Debug)]
struct AuthPublicationTool(std::sync::atomic::AtomicUsize);

impl Tool for AuthPublicationTool {
    type Args = serde_json::Value;
    type Output = ToolsToolOutput;
    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("timing_fixture").unwrap()
    }
    fn description(&self, _: &ListToolsContext) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new("timing_fixture", "Auth publication fixture")
    }
    async fn run(&self, _: ToolCallContext, _: Self::Args) -> Result<Self::Output, ToolError> {
        match self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed) {
            0 => {
                tokio::time::sleep(Duration::from_secs(1)).await;
                Err(ToolError::new(
                    xai_tool_runtime::ToolErrorKind::Unauthorized,
                    "fixture authentication required",
                )
                .with_details(serde_json::json!({HTTP_STATUS_DETAILS_KEY: 401})))
            }
            1 => {
                tokio::time::sleep(Duration::from_secs(2)).await;
                Ok(ToolsToolOutput::Text("fixture result".into()))
            }
            other => panic!("unexpected auth attempt {other}"),
        }
    }
}

impl xai_grok_tools::types::tool_metadata::ToolMetadata for AuthPublicationTool {
    fn kind(&self) -> ToolKind {
        ToolKind::ImageGen
    }
    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }
    fn description_template(&self) -> &str {
        "Auth publication fixture"
    }
}

struct HeldAuthRecovery {
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Semaphore>,
}

#[async_trait::async_trait]
impl xai_grok_login::refresh::TokenRefresher for HeldAuthRecovery {
    async fn refresh(
        &self,
        _: xai_grok_login::refresh::RefreshReason,
    ) -> xai_grok_login::refresh::RefreshOutcome {
        self.started.notify_one();
        let _permit = self.release.acquire().await.unwrap();
        let mut auth = xai_grok_login::GrokAuth::test_default();
        auth.expires_at = Some(chrono::Utc::now() + chrono::Duration::hours(1));
        xai_grok_login::refresh::RefreshOutcome::Success(Box::new(auth))
    }
}

async fn next_owned_timing(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<SessionEvent>,
) -> (ToolCallOwnership, ToolExecutionTiming) {
    loop {
        let SessionEvent::Notification(SessionNotification::Acp(notification)) =
            events.recv().await.unwrap()
        else {
            continue;
        };
        let acp::SessionUpdate::ToolCallUpdate(update) = notification.update else {
            continue;
        };
        let timing = timing_from_meta(update.meta.as_ref()).unwrap();
        if timing != ToolExecutionTiming::Unknown {
            return (ownership_from_meta(update.meta.as_ref()).unwrap(), timing);
        }
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn actual_auth_retry_keeps_both_execution_spans_and_excludes_held_recovery() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut fixture = make_replay_send_update_fixture().await;
            let toolset = fixture.actor.agent.borrow().tool_bridge().toolset();
            toolset
                .register_tool(
                    "timing_fixture".into(),
                    AuthPublicationTool(std::sync::atomic::AtomicUsize::new(0)),
                    None,
                )
                .unwrap();
            fixture
                .actor
                .workspace_ops
                .bind_local_session(
                    fixture.actor.session_info.id.0.as_ref(),
                    fixture.actor.tool_context.cwd.to_path_buf(),
                    fixture.actor.tool_context.hunk_tracker_handle.clone(),
                    toolset,
                    None,
                )
                .unwrap();
            let directory = tempfile::tempdir().unwrap();
            let manager = Arc::new(xai_grok_login::AuthManager::new(
                directory.path(),
                xai_grok_login::GrokComConfig::default(),
            ));
            let mut auth = xai_grok_login::GrokAuth::test_default();
            auth.auth_mode = xai_grok_login::AuthMode::Oidc;
            auth.refresh_token = Some(uuid::Uuid::new_v4().to_string());
            auth.expires_at = Some(chrono::Utc::now() - chrono::Duration::hours(1));
            manager.hot_swap(auth);
            let recovering = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Semaphore::new(0));
            manager.set_refresher(Arc::new(HeldAuthRecovery {
                started: recovering.clone(),
                release: release.clone(),
            }));
            fixture.actor.auth_manager = Some(manager);
            *fixture.actor.current_prompt_id.lock().unwrap() = Some("first-prompt".into());
            let request = xai_grok_sampler::RequestId::random();
            fixture.actor.register_sampling_request(&request);
            let owner = fixture.actor.sampling_owner(&request).unwrap();
            let execute =
                fixture
                    .actor
                    .execute_tool_calls(vec![call("reused-provider")], None, Some(&owner));
            let observe = async {
                let (registration, first) = next_owned_timing(&mut fixture.event_rx).await;
                let ToolExecutionTiming::Started {
                    execution_id: first_id,
                    started_at: first_point,
                } = first
                else {
                    panic!("first actual start");
                };
                tokio::time::advance(Duration::from_secs(1)).await;
                let (same, finished) = next_owned_timing(&mut fixture.event_rx).await;
                assert_eq!(same, registration);
                assert_eq!(
                    finished,
                    ToolExecutionTiming::Finished {
                        execution_id: first_id,
                        started_at: first_point,
                        duration_ms: 1000,
                        termination: ToolExecutionTermination::Returned,
                    }
                );
                recovering.notified().await;
                let pending = snapshot(&fixture.actor);
                assert_eq!(pending.tools[0].executions, [finished]);
                tokio::time::advance(Duration::from_secs(30)).await;
                assert_eq!(snapshot(&fixture.actor).tools, pending.tools);
                release.add_permits(1);
                let (same, second) = next_owned_timing(&mut fixture.event_rx).await;
                assert_eq!(same, registration);
                let ToolExecutionTiming::Started {
                    execution_id: second_id,
                    started_at: second_point,
                } = second
                else {
                    panic!("second actual start");
                };
                assert_ne!(first_id, second_id);
                tokio::time::advance(Duration::from_secs(2)).await;
                let (same, second_finished) = next_owned_timing(&mut fixture.event_rx).await;
                assert_eq!(same, registration);
                assert_eq!(
                    second_finished,
                    ToolExecutionTiming::Finished {
                        execution_id: second_id,
                        started_at: second_point,
                        duration_ms: 2000,
                        termination: ToolExecutionTermination::Returned,
                    }
                );
            };
            let (result, ()) = tokio::join!(execute, observe);
            assert!(result.is_ok());
            assert!(snapshot(&fixture.actor).tools.is_empty());
        })
        .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn native_postflight_cancel_uses_frozen_owner_and_cannot_repeat_after_terminal() {
    tokio::task::LocalSet::new().run_until(async {
        let mut fixture = make_replay_send_update_fixture().await;
        install_fixture_tool(&fixture.actor, ToolKind::ImageGen, TimingFixtureSchedule::Immediate);
        let (gateway_tx, mut gateway_rx) = tokio::sync::mpsc::unbounded_channel();
        fixture.actor.notifications.gateway = GatewaySender::new(gateway_tx);
        fixture.actor.client_hooks.borrow_mut().insert(
            xai_grok_hooks::event::HookEventName::PostToolUse,
            vec![crate::extensions::hooks::ClientHookGroup {
                matcher: None, callback_ids: vec!["held-postflight".into()], timeout: Some(Duration::from_secs(300)),
            }],
        );
        *fixture.actor.current_prompt_id.lock().unwrap() = Some("first-prompt".into());
        let request = xai_grok_sampler::RequestId::random();
        fixture.actor.register_sampling_request(&request);
        let owner = fixture.actor.sampling_owner(&request).unwrap();
        let mut execute = Box::pin(fixture.actor.execute_tool_calls(vec![call("reused-provider")], None, Some(&owner)));
        let request = loop {
            tokio::select! {
                message = gateway_rx.recv() => {
                    if let xai_acp_lib::AcpClientMessage::ExtMethod(request) = message.unwrap() { break request; }
                }
                result = execute.as_mut() => panic!("postflight must still be held: {result:?}"),
            }
        };
        assert!(fixture.actor.events.has_active_tool());
        assert!(matches!(snapshot(&fixture.actor).tools[0].executions.as_slice(), [ToolExecutionTiming::Finished { duration_ms: 2000, .. }]));
        *fixture.actor.current_prompt_id.lock().unwrap() = Some("second-prompt".into());
        fixture.actor.begin_sampling_turn();
        drop(execute);
        assert!(request.response_tx.is_closed());
        fixture.actor.events.cancel_active_tool();
        fixture.actor.events.cancel_active_tool();
        assert_publications(&mut fixture.event_rx, &original_source(&owner), 2);
        assert!(!fixture.actor.events.has_active_tool());
        assert!(snapshot(&fixture.actor).tools.is_empty());
    }).await;
}
