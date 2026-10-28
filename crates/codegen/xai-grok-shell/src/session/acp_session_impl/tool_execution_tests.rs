use super::*;
use super::super::replay_buffer_send_update_tests::make_replay_send_update_fixture;
use crate::extensions::tool_timing::{ToolExecutionTermination, ToolExecutionTiming, timing_from_meta};
use std::time::Duration;
use xai_grok_tools::types::tool::{ToolKind, ToolNamespace};
use xai_grok_workspace::permission::{PermissionCommand, PermissionHandle, PermissionResolution};
use xai_tool_runtime::{ListToolsContext, Tool, ToolCallContext, ToolError};

#[derive(Debug)]
enum TimingFixtureSchedule {
    Immediate,
    Held(Arc<tokio::sync::Notify>),
}

#[derive(Debug)]
struct TimingFixtureTool {
    kind: ToolKind,
    schedule: TimingFixtureSchedule,
}

impl Tool for TimingFixtureTool {
    type Args = serde_json::Value;
    type Output = ToolsToolOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("timing_fixture").unwrap()
    }

    fn description(&self, _: &ListToolsContext) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new("timing_fixture", "Timing fixture")
    }

    async fn run(&self, _: ToolCallContext, _: Self::Args) -> Result<Self::Output, ToolError> {
        let until = tokio::time::Instant::now() + Duration::from_secs(2);
        match &self.schedule {
            TimingFixtureSchedule::Immediate => {}
            TimingFixtureSchedule::Held(release) => release.notified().await,
        }
        tokio::time::sleep_until(until).await;
        Ok(ToolsToolOutput::Text("fixture result".into()))
    }
}

impl xai_grok_tools::types::tool_metadata::ToolMetadata for TimingFixtureTool {
    fn kind(&self) -> ToolKind { self.kind }
    fn tool_namespace(&self) -> ToolNamespace { ToolNamespace::GrokBuild }
    fn description_template(&self) -> &str { "Timing fixture" }
}

fn install_fixture_tool(actor: &SessionActor, kind: ToolKind, schedule: TimingFixtureSchedule) {
    let toolset = actor.agent.borrow().tool_bridge().toolset();
    toolset.register_tool("timing_fixture".into(), TimingFixtureTool { kind, schedule }, None).unwrap();
    assert!(actor.workspace_ops.bind_local_session(
        actor.session_info.id.0.as_ref(), actor.tool_context.cwd.to_path_buf(),
        actor.tool_context.hunk_tracker_handle.clone(), toolset, None,
    ).unwrap());
}

fn permission_gate(actor: &mut SessionActor) -> tokio::sync::mpsc::UnboundedReceiver<PermissionCommand> {
    let (cmd_tx, commands) = tokio::sync::mpsc::unbounded_channel();
    actor.permissions = PermissionHandle::Actor {
        cmd_tx,
        yolo_state: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        auto_state: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        side_query_wired: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        yolo_pin: None, deny_read_globs: Arc::new(vec![]),
        in_flight: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        user_prompt_notify: Arc::new(parking_lot::Mutex::new(None)),
    };
    commands
}

fn call(id: &str) -> crate::sampling::types::ToolCallResponse {
    crate::sampling::types::ToolCallResponse {
        id: id.into(), kind: "function".into(),
        function: crate::sampling::types::ToolCallFunction {
            name: "timing_fixture".into(), arguments: r#"{"file_path":"/tmp/timing-fixture-shared"}"#.into(),
        },
    }
}

fn timing(event: SessionEvent) -> Option<(String, ToolExecutionTiming)> {
    if let SessionEvent::Notification(notification) = event
        && let acp::SessionUpdate::ToolCallUpdate(update) = notification.into_acp().update
    {
        match timing_from_meta(update.meta.as_ref()).unwrap() {
            ToolExecutionTiming::Unknown => None,
            timing @ (ToolExecutionTiming::Started { .. } | ToolExecutionTiming::Finished { .. }) => {
                assert_eq!(update.fields.status, None, "timing does not confirm a backend outcome");
                Some((update.tool_call_id.0.to_string(), timing))
            }
        }
    } else {
        None
    }
}

async fn next_timing(events: &mut tokio::sync::mpsc::UnboundedReceiver<SessionEvent>) -> (String, ToolExecutionTiming) {
    loop {
        if let Some(timing) = timing(events.recv().await.unwrap()) {
            return timing;
        }
    }
}

#[test]
fn native_execution_wire_retains_duration_without_a_received_start() {
    let entry = xai_grok_session_events::decode_event_entry(
        r#"{"ts":"2026-10-28T00:00:00.000Z","type":"tool_execution","tool_call_id":"wire-orphan","timing":{"state":"finished","executionId":"12345678-1234-4234-8234-123456789abc","durationMs":2000,"termination":{"kind":"dispatch_cancelled"}}}"#,
    ).unwrap();
    let crate::session::events::Event::ToolExecution { tool_call_id, timing, .. } = entry.event else {
        panic!("the canonical decoder must retain the execution event");
    };
    assert_eq!(tool_call_id, "wire-orphan");
    assert!(matches!(timing, ToolExecutionTiming::Finished {
        duration_ms: 2_000, termination: ToolExecutionTermination::DispatchCancelled, ..
    }));
    let meta = crate::extensions::tool_timing::timing_meta(timing).unwrap();
    assert_eq!(timing_from_meta(Some(&meta)).unwrap(), timing);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn actual_permission_thirty_seconds_then_execution_two_seconds() {
    tokio::task::LocalSet::new().run_until(async {
        let mut fixture = make_replay_send_update_fixture().await;
        let native = tempfile::tempdir().unwrap();
        fixture.actor.events = crate::session::events::EventTracker::new(native.path());
        install_fixture_tool(&fixture.actor, ToolKind::Other, TimingFixtureSchedule::Immediate);
        let mut commands = permission_gate(&mut fixture.actor);
        let execute = fixture.actor.execute_tool_calls(vec![call("permission-timed")], None, None);
        let observe = async {
            let PermissionCommand::Request { respond_to, .. } = commands.recv().await.unwrap() else {
                panic!("the actual permission boundary must be reached");
            };
            tokio::time::advance(Duration::from_secs(30)).await;
            while let Ok(event) = fixture.event_rx.try_recv() {
                assert!(timing(event).is_none(), "permission waiting is not execution");
            }
            respond_to.send(PermissionResolution { decision: Decision::Allow, event: None }).unwrap();
            let (id, start) = next_timing(&mut fixture.event_rx).await;
            assert_eq!(id, "permission-timed");
            let ToolExecutionTiming::Started { execution_id, started_at } = start else { panic!("missing actual start"); };
            tokio::time::advance(Duration::from_secs(2)).await;
            let (id, finish) = next_timing(&mut fixture.event_rx).await;
            assert_eq!(id, "permission-timed");
            assert_eq!(finish, ToolExecutionTiming::Finished {
                execution_id, started_at, duration_ms: 2_000, termination: ToolExecutionTermination::Returned,
            });
        };
        let (result, ()) = tokio::join!(execute, observe);
        assert!(matches!(result.unwrap(), ToolLoop::Continue));
        while let Ok(event) = fixture.event_rx.try_recv() { assert!(timing(event).is_none()); }
        let entries: Vec<_> = std::fs::read_to_string(native.path().join("events.jsonl")).unwrap()
            .lines().map(|line| xai_grok_session_events::decode_event_entry(line).unwrap()).collect();
        assert_eq!(entries.iter().filter(|entry| matches!(entry.event,
            crate::session::events::Event::ToolStarted { .. })).count(), 1,
            "canonical call registration remains intact");
        assert_eq!(entries.iter().filter(|entry| matches!(entry.event,
            crate::session::events::Event::ToolExecution { .. })).count(), 2);
    }).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn deny_cancel_and_cap_rejection_never_publish_execution() {
    tokio::task::LocalSet::new().run_until(async {
        for decision in [Decision::Reject("denied".into()), Decision::PolicyDeny("policy".into()), Decision::Cancelled] {
            let mut fixture = make_replay_send_update_fixture().await;
            let native = tempfile::tempdir().unwrap();
            fixture.actor.events = crate::session::events::EventTracker::new(native.path());
            install_fixture_tool(&fixture.actor, ToolKind::Other, TimingFixtureSchedule::Immediate);
            let mut commands = permission_gate(&mut fixture.actor);
            let execute = fixture.actor.execute_tool_calls(vec![call("never-executed")], None, None);
            let reject = async {
                let PermissionCommand::Request { respond_to, .. } = commands.recv().await.unwrap() else {
                    panic!("expected permission request");
                };
                respond_to.send(PermissionResolution { decision, event: None }).unwrap();
            };
            let (result, ()) = tokio::join!(execute, reject);
            assert!(result.is_ok());
            while let Ok(event) = fixture.event_rx.try_recv() { assert!(timing(event).is_none()); }
            for line in std::fs::read_to_string(native.path().join("events.jsonl")).unwrap().lines() {
                assert!(!matches!(xai_grok_session_events::decode_event_entry(line).unwrap().event,
                    crate::session::events::Event::ToolExecution { .. }));
            }
        }
        let mut fixture = make_replay_send_update_fixture().await;
        install_fixture_tool(&fixture.actor, ToolKind::ImageGen, TimingFixtureSchedule::Immediate);
        fixture.actor.rebuild_spec.media_gen_batch_limits.max_image = 0;
        assert!(matches!(fixture.actor.execute_tool_calls(vec![call("cap-rejected")], None, None).await.unwrap(), ToolLoop::Continue));
        let mut statuses = Vec::new();
        while let Ok(event) = fixture.event_rx.try_recv() {
            if let SessionEvent::Notification(notification) = event {
                match notification.into_acp().update {
                    acp::SessionUpdate::ToolCall(call) => {
                        assert_eq!(timing_from_meta(call.meta.as_ref()).unwrap(), ToolExecutionTiming::Unknown);
                        statuses.push(call.status);
                    }
                    acp::SessionUpdate::ToolCallUpdate(update) => {
                        assert_eq!(timing_from_meta(update.meta.as_ref()).unwrap(), ToolExecutionTiming::Unknown);
                        if let Some(status) = update.fields.status { statuses.push(status); }
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(statuses, vec![acp::ToolCallStatus::Pending, acp::ToolCallStatus::Failed]);
    }).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn same_resource_waits_until_previous_dispatch_releases_its_guard() {
    tokio::task::LocalSet::new().run_until(async {
        let mut fixture = make_replay_send_update_fixture().await;
        install_fixture_tool(&fixture.actor, ToolKind::ImageGen, TimingFixtureSchedule::Immediate);
        let execute = fixture.actor.execute_tool_calls(vec![call("one"), call("two")], None, None);
        let observe = async {
            let (first_id, first_start) = next_timing(&mut fixture.event_rx).await;
            let ToolExecutionTiming::Started { execution_id: first, started_at: first_point } = first_start else { panic!("missing first start"); };
            tokio::task::yield_now().await;
            while let Ok(event) = fixture.event_rx.try_recv() {
                assert!(timing(event).is_none(), "a resource-blocked dispatch must not start");
            }
            tokio::time::advance(Duration::from_secs(2)).await;
            assert_eq!(next_timing(&mut fixture.event_rx).await, (first_id.clone(), ToolExecutionTiming::Finished {
                execution_id: first, started_at: first_point, duration_ms: 2_000, termination: ToolExecutionTermination::Returned,
            }));
            let (second_id, second_start) = next_timing(&mut fixture.event_rx).await;
            assert_ne!(first_id, second_id);
            let ToolExecutionTiming::Started { execution_id: second, started_at: second_point } = second_start else { panic!("missing resource-admitted start"); };
            assert_ne!(first, second);
            tokio::time::advance(Duration::from_secs(2)).await;
            assert_eq!(next_timing(&mut fixture.event_rx).await, (second_id, ToolExecutionTiming::Finished {
                execution_id: second, started_at: second_point, duration_ms: 2_000, termination: ToolExecutionTermination::Returned,
            }));
        };
        let (result, ()) = tokio::join!(execute, observe);
        assert!(result.is_ok());
        while let Ok(event) = fixture.event_rx.try_recv() { assert!(timing(event).is_none()); }
    }).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn cancelling_an_executing_batch_settles_the_actual_dispatch_once() {
    tokio::task::LocalSet::new().run_until(async {
        let mut fixture = make_replay_send_update_fixture().await;
        install_fixture_tool(&fixture.actor, ToolKind::ImageGen, TimingFixtureSchedule::Immediate);
        let mut execute = Box::pin(fixture.actor.execute_tool_calls(vec![call("cancel-running"), call("cancel-blocked")], None, None));
        let (id, start) = loop {
            tokio::select! {
                event = fixture.event_rx.recv() => {
                    if let Some(event) = timing(event.unwrap()) { break event; }
                }
                result = execute.as_mut() => panic!("dispatch finished before cancellation: {result:?}"),
            }
        };
        assert_eq!(id, "cancel-running");
        let ToolExecutionTiming::Started { execution_id, started_at } = start else { panic!("missing actual start"); };
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(500)).await;
        drop(execute);
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        let mut finishes = Vec::new();
        let mut cancellations = 0;
        while let Ok(event) = fixture.event_rx.try_recv() {
            if let SessionEvent::Notification(notification) = event
                && let acp::SessionUpdate::ToolCallUpdate(update) = notification.into_acp().update
            {
                assert!(["cancel-running", "cancel-blocked"].contains(&update.tool_call_id.0.as_ref()));
                match timing_from_meta(update.meta.as_ref()).unwrap() {
                    ToolExecutionTiming::Unknown => {
                        if update.meta.as_ref().and_then(|meta| meta.get("toolTerminalStatus")) == Some(&serde_json::json!("cancelled")) {
                            assert_eq!(update.fields.status, Some(acp::ToolCallStatus::Failed));
                            cancellations += 1;
                        }
                    }
                    finished @ ToolExecutionTiming::Finished { .. } => {
                        assert_eq!(update.tool_call_id.0.as_ref(), "cancel-running");
                        assert_eq!(update.fields.status, None);
                        finishes.push(finished);
                    }
                    ToolExecutionTiming::Started { .. } => panic!("duplicate actual start"),
                }
            }
        }
        assert_eq!(finishes, vec![ToolExecutionTiming::Finished {
            execution_id, started_at, duration_ms: 500, termination: ToolExecutionTermination::DispatchCancelled,
        }]);
        assert_eq!(cancellations, 2);
    }).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn terminal_before_base_retains_native_timing_through_the_persisted_acp_rail() {
    use crate::extensions::tool_timing::ownership_from_meta;
    tokio::task::LocalSet::new().run_until(async {
        let mut fixture = make_replay_send_update_fixture().await;
        install_fixture_tool(&fixture.actor, ToolKind::ImageGen, TimingFixtureSchedule::Immediate);
        *fixture.actor.current_prompt_id.lock().unwrap() = Some("original-prompt".into());
        let request = xai_grok_sampler::RequestId::random();
        fixture.actor.register_sampling_request(&request);
        let source = fixture.actor.sampling_owner(&request).unwrap();
        fixture.actor.install_tool_lifecycle_bridge();
        fixture.actor.install_tool_lifecycle_bridge();
        fixture.actor.execute_tool_calls(vec![call("orphan")], None, Some(&source)).await.unwrap();
        let mut base = None;
        let mut terminal = None;
        let mut started = None;
        let mut timing_publications = 0;
        while let Ok(event) = fixture.event_rx.try_recv() {
            let SessionEvent::Notification(notification) = event else { continue; };
            let update = match &notification {
                SessionNotification::Acp(notification) => &notification.update,
                SessionNotification::Xai(_) => continue,
            };
            match update {
                acp::SessionUpdate::ToolCall(call) if call.tool_call_id.0.as_ref() == "orphan" => {
                    base = Some(notification);
                }
                acp::SessionUpdate::ToolCallUpdate(update) => {
                    match timing_from_meta(update.meta.as_ref()).unwrap() {
                        ToolExecutionTiming::Started { started_at, .. } => {
                            timing_publications += 1;
                            started = Some(started_at);
                        }
                        finished @ ToolExecutionTiming::Finished { duration_ms: 2000, .. } => {
                            timing_publications += 1;
                            terminal = Some((notification, finished));
                        }
                        ToolExecutionTiming::Unknown => {}
                        ToolExecutionTiming::Finished { .. } => panic!("actual execution must take two seconds"),
                    }
                }
                _ => {}
            }
        }
        assert_eq!(timing_publications, 2, "the single bridge publishes each actual edge once");
        let (terminal, expected) = terminal.unwrap();
        let base = base.unwrap();
        let owner = match &base.expect_acp().update {
            acp::SessionUpdate::ToolCall(call) => ownership_from_meta(call.meta.as_ref()).unwrap(),
            _ => panic!("actual registration"),
        };
        let ToolExecutionTiming::Finished { started_at, .. } = expected else { panic!("actual terminal"); };
        assert_eq!(Some(started_at), started);
        // Delivery deliberately omits Started and puts the actual terminal before its registration.
        fixture.actor.emit_buffered(terminal).await;
        fixture.actor.emit_buffered(base).await;
        let mut persisted_timing = Vec::new();
        let mut bases = 0;
        while let Ok(message) = fixture.persistence_rx.try_recv() {
            if let PersistenceMsg::Update(crate::session::storage::SessionUpdate::Acp(notification)) = message {
                let encoded = serde_json::to_string(&notification).unwrap();
                let replay: acp::SessionNotification = serde_json::from_str(&encoded).unwrap();
                match replay.update {
                    acp::SessionUpdate::ToolCallUpdate(update) => {
                        let timing = timing_from_meta(update.meta.as_ref()).unwrap();
                        if timing == ToolExecutionTiming::Unknown { continue; }
                        assert_eq!(update.tool_call_id.0.as_ref(), "orphan");
                        assert_eq!(update.fields.status, None, "timing does not confirm process completion");
                        assert_eq!(ownership_from_meta(update.meta.as_ref()).unwrap(), owner);
                        assert_eq!(replay.meta.as_ref().unwrap().get("promptId"), Some(&serde_json::json!("original-prompt")));
                        assert_eq!(replay.meta.as_ref().unwrap().get("requestId"), Some(&serde_json::json!(request.as_str())));
                        persisted_timing.push(timing);
                    }
                    acp::SessionUpdate::ToolCall(call) if call.tool_call_id.0.as_ref() == "orphan" => {
                        assert_eq!(ownership_from_meta(call.meta.as_ref()).unwrap(), owner);
                        assert_eq!(timing_from_meta(call.meta.as_ref()).unwrap(), ToolExecutionTiming::Unknown);
                        bases += 1;
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(persisted_timing, vec![expected]);
        assert_eq!(bases, 1);
        assert!(fixture.event_rx.try_recv().is_err());
        assert_eq!(timing_from_meta(None).unwrap(), ToolExecutionTiming::Unknown);
        let legacy = serde_json::json!({"duration_ms": 32_000, "agentTimestampMs": 123});
        assert_eq!(timing_from_meta(legacy.as_object()).unwrap(), ToolExecutionTiming::Unknown);
        let malformed = serde_json::json!({"toolExecutionTiming": {"state": "finished", "durationMs": 2_000}});
        assert!(timing_from_meta(malformed.as_object()).is_err());
    }).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn reused_provider_id_keeps_real_instances_and_late_terminal_source_separate() {
    use crate::extensions::sampling_lifecycle::SamplingOwner;
    use crate::extensions::tool_timing::{ToolCallOwnership, ToolCallSource, ownership_from_meta};
    tokio::task::LocalSet::new().run_until(async {
        let mut fixture = make_replay_send_update_fixture().await;
        let release = Arc::new(tokio::sync::Notify::new());
        install_fixture_tool(&fixture.actor, ToolKind::ImageGen, TimingFixtureSchedule::Held(Arc::clone(&release)));
        *fixture.actor.current_prompt_id.lock().unwrap() = Some("first-prompt".into());
        let request_one = xai_grok_sampler::RequestId::random();
        fixture.actor.register_sampling_request(&request_one);
        let first_owner = fixture.actor.sampling_owner(&request_one).unwrap();
        let request_two = xai_grok_sampler::RequestId::random();
        let (first_started_tx, first_started_rx) = tokio::sync::oneshot::channel();
        let actor = &fixture.actor;
        let first = actor.execute_tool_calls(vec![call("reused-provider-id")], None, Some(&first_owner));
        let second = async {
            first_started_rx.await.unwrap();
            tokio::time::advance(Duration::from_secs(1)).await;
            *actor.current_prompt_id.lock().unwrap() = Some("second-prompt".into());
            actor.begin_sampling_turn();
            actor.register_sampling_request(&request_two);
            let owner = actor.sampling_owner(&request_two).unwrap();
            actor.execute_tool_calls(vec![call("reused-provider-id")], None, Some(&owner)).await
        };
        let observe = async {
            let mut first_started_tx = Some(first_started_tx);
            let mut records = Vec::new();
            let mut registrations = Vec::new();
            let mut ordinary = Vec::new();
            while records.len() < 4 {
                let SessionEvent::Notification(notification) = fixture.event_rx.recv().await.unwrap() else { continue; };
                let notification = notification.into_acp();
                ordinary.push(notification.clone());
                match &notification.update {
                    acp::SessionUpdate::ToolCall(call) => {
                        let owner = ownership_from_meta(call.meta.as_ref()).unwrap();
                        if owner != ToolCallOwnership::Unknown { registrations.push(owner); }
                    }
                    acp::SessionUpdate::ToolCallUpdate(update) => {
                        let timing = timing_from_meta(update.meta.as_ref()).unwrap();
                        if timing == ToolExecutionTiming::Unknown { continue; }
                        assert_eq!(update.tool_call_id.0.as_ref(), "reused-provider-id");
                        assert_eq!(update.fields.status, None);
                        let owner = ownership_from_meta(update.meta.as_ref()).unwrap();
                        records.push((owner, timing, notification.meta.clone().unwrap()));
                        if records.len() == 1 { first_started_tx.take().unwrap().send(()).unwrap(); }
                        if records.len() == 2 { release.notify_waiters(); }
                        if records.len() == 2 || records.len() == 3 {
                            tokio::time::advance(Duration::from_secs(1)).await;
                        }
                    }
                    _ => {}
                }
            }
            let [(first_call, ToolExecutionTiming::Started { execution_id: first_id, started_at: first_point }, _),
                (second_call, ToolExecutionTiming::Started { execution_id: second_id, started_at: second_point }, _),
                (old_terminal_call, ToolExecutionTiming::Finished { execution_id: old_id, started_at: old_point, duration_ms: 2000, .. }, old_meta),
                (new_terminal_call, ToolExecutionTiming::Finished { execution_id: new_id, started_at: new_point, duration_ms: 2000, .. }, _)] = records.as_slice()
                else { panic!("two actual attempts must retain both original spans"); };
            assert_ne!(first_id, second_id);
            assert_eq!(first_id, old_id);
            assert_eq!(second_id, new_id);
            assert_eq!(first_point, old_point);
            assert_eq!(second_point, new_point);
            assert_eq!(first_call, old_terminal_call);
            assert_eq!(second_call, new_terminal_call);
            assert_ne!(first_call, second_call);
            assert_eq!(registrations, vec![first_call.clone(), second_call.clone()]);
            let ToolCallOwnership::Registered { invocation_id: first_instance, provider_call_id, source } = first_call else { panic!("registered first instance"); };
            let ToolCallOwnership::Registered { invocation_id: second_instance, .. } = second_call else { panic!("registered second instance"); };
            assert_ne!(first_instance, second_instance);
            assert_eq!(provider_call_id, "reused-provider-id");
            assert_eq!(source, &ToolCallSource::Sampling {
                prompt_id: first_owner.prompt_id.clone(), request_id: request_one.as_str().into(), attempt: first_owner.attempt.0,
            });
            assert_eq!(old_meta.get("promptId"), Some(&serde_json::json!("first-prompt")));
            assert_eq!(old_meta.get("requestId"), Some(&serde_json::json!(request_one.as_str())));
            assert_eq!(actor.current_prompt_id.lock().unwrap().as_deref(), Some("second-prompt"));
            let second_expected = SamplingOwner {
                prompt_id: "second-prompt".into(), request_id: request_two.clone(), attempt: first_owner.attempt,
            };
            assert_eq!(actor.sampling_owner(&request_two), Some(second_expected));
            ordinary
        };
        let (first_result, second_result, mut ordinary) = tokio::join!(first, second, observe);
        assert!(first_result.is_ok());
        assert!(second_result.is_ok());
        while let Ok(SessionEvent::Notification(SessionNotification::Acp(notification))) = fixture.event_rx.try_recv() {
            ordinary.push(*notification);
        }
        let bases: Vec<_> = ordinary.iter().filter_map(|notification| match &notification.update {
            acp::SessionUpdate::ToolCall(call) => Some(ownership_from_meta(call.meta.as_ref()).unwrap()),
            _ => None,
        }).collect();
        assert_eq!(bases.len(), 2);
        assert_ne!(bases[0], bases[1]);
        let mut canonical = [0, 0];
        let mut kinds = [0, 0];
        for notification in ordinary {
            let (owner, terminal, kind) = match notification.update {
                acp::SessionUpdate::ToolCall(call) => (ownership_from_meta(call.meta.as_ref()).unwrap(), false, false),
                acp::SessionUpdate::ToolCallUpdate(update) => (
                    ownership_from_meta(update.meta.as_ref()).unwrap(),
                    matches!(update.fields.status, Some(acp::ToolCallStatus::Completed | acp::ToolCallStatus::Failed)),
                    update.fields.kind.is_some(),
                ),
                _ => continue,
            };
            let index = bases.iter().position(|base| *base == owner).expect("every ordinary publication must use its real registered invocation");
            canonical[index] += usize::from(terminal);
            kinds[index] += usize::from(kind);
            let ToolCallOwnership::Registered { source: ToolCallSource::Sampling { prompt_id, request_id, attempt }, .. } = owner else { panic!("no SourceUnknown fallback for actual sampling"); };
            assert_eq!(prompt_id, ["first-prompt", "second-prompt"][index]);
            assert_eq!(request_id, [request_one.as_str(), request_two.as_str()][index]);
            assert_eq!(attempt, first_owner.attempt.0);
            let meta = notification.meta.unwrap();
            assert_eq!(meta.get("promptId"), Some(&serde_json::json!(prompt_id)));
            assert_eq!(meta.get("requestId"), Some(&serde_json::json!(request_id)));
            assert_eq!(meta.get("streamEpoch"), Some(&serde_json::json!(attempt)));
        }
        assert_eq!(canonical, [1, 1]);
        assert_eq!(kinds, [1, 1]);
    }).await;
}

#[path = "current_actions_tests.rs"]
mod current_actions_tests;

#[path = "tool_publication_tests.rs"]
mod tool_publication_tests;
