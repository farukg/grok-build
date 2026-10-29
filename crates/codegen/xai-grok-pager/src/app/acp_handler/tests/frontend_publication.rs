use super::*;
use std::future::Future;
use xai_grok_shell::extensions::current_actions::{
    self as wire, DeliveryRole, GenerationEnd, LoadCut,
};
use xai_grok_shell::session::current_actions::{CurrentActionsSnapshot, SnapshotBinding};

async fn poll_once<F: Future>(mut future: std::pin::Pin<&mut F>) -> std::task::Poll<F::Output> {
    std::future::poll_fn(|cx| std::task::Poll::Ready(future.as_mut().poll(cx))).await
}

async fn pump<T>(
    future: impl Future<Output = T>,
    rx: &mut xai_acp_lib::AcpClientRx,
    app: &mut AppView,
) -> T {
    tokio::pin!(future);
    loop {
        tokio::select! {
            result = &mut future => return result,
            message = rx.recv() => { handle(message.expect("live local receiver"), app); }
        }
    }
}

fn notification(app: &mut AppView, method: &str, value: &impl serde::Serialize) {
    let (response_tx, mut completion) = tokio::sync::oneshot::channel();
    handle(
        AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
            request: acp_fixtures::ext_notification(method, value),
            response_tx,
        }),
        app,
    );
    assert!(completion.try_recv().unwrap().is_ok());
}

#[test]
#[ignore = "isolated resident actor / MockInference process; mandatory publication CI filter"]
#[serial_test::serial(GROK_HOME)]
fn actual_resident_prompt_load_and_reconnect_use_pager_marker_before_response() {
    resident_prompt_load(xai_acp_lib::ReceiverSettlement::AppliedOrDiscarded);
}

#[test]
#[ignore = "isolated resident actor / MockInference process; mandatory publication CI filter"]
#[serial_test::serial(GROK_HOME)]
fn actual_transport_only_resident_load_never_claims_pager_settlement() {
    resident_prompt_load(xai_acp_lib::ReceiverSettlement::TransportOnly);
}

fn resident_prompt_load(settlement: xai_acp_lib::ReceiverSettlement) {
    use std::rc::Rc;
    use std::sync::Arc;
    use xai_grok_shell::agent::{
        MvpAgent,
        config::{Config, ModelEntry},
    };
    use xai_grok_test_support::{
        InferenceEndpoint, InferenceRequestMatcher, MockInferenceServer, ScriptedResponse,
    };
    let mut keepalive = None;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, async {
        tokio::time::timeout(Duration::from_secs(90), async {
        xai_grok_extra_ca::ensure_default_crypto_provider();
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let server = MockInferenceServer::start().await.unwrap();
        let _env = [
            crate::test_util::EnvVarGuard::set("GROK_HOME", home.path()),
            crate::test_util::EnvVarGuard::set("GROK_XAI_API_BASE_URL", server.url()),
            crate::test_util::EnvVarGuard::set("GROK_TELEMETRY_ENABLED", "false"),
            crate::test_util::EnvVarGuard::set("GROK_FEEDBACK_ENABLED", "false"),
        ];
        server.set_response("finished");
        let (client_tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let gateway = xai_acp_lib::AcpGatewaySender::new(client_tx);
        let gateway = match settlement {
            xai_acp_lib::ReceiverSettlement::AppliedOrDiscarded => gateway.with_local_receiver_settlement().unwrap(),
            xai_acp_lib::ReceiverSettlement::TransportOnly => gateway,
        };
        let mut entry = ModelEntry::fallback("test-model", &Default::default());
        entry.info.base_url = server.url();
        entry.info.api_backend = xai_grok_shell::sampling::ApiBackend::Responses;
        entry.api_key = Some("test".into());
        let config = Config::default();
        let auth = Arc::new(xai_grok_login::AuthManager::new(home.path(), xai_grok_login::GrokComConfig::default()));
        let agent = Rc::new(MvpAgent::new(gateway, &config, auth, Some([("test-model".into(), entry)].into_iter().collect()), None).unwrap());
        keepalive = Some(agent.clone());
        let expected = uuid::Uuid::now_v7().to_string();
        let mut app = make_app_viewing_child(&expected, "unused-child");
        let id = AgentId(0);
        app.active_view = ActiveView::Agent(id);
        pump(acp::Agent::initialize(&*agent, acp::InitializeRequest::new(acp::ProtocolVersion::V1)
            .client_capabilities(acp::ClientCapabilities::new().fs(acp::FileSystemCapabilities::new()).terminal(false))), &mut rx, &mut app).await.unwrap();
        acp::Agent::authenticate(&*agent, acp::AuthenticateRequest::new(acp::AuthMethodId::new("xai.api_key"))).await.unwrap();
        let created = pump(acp::Agent::new_session(&*agent, acp::NewSessionRequest::new(cwd.path().to_path_buf())
            .meta(json!({"modelId": "test-model", "sessionId": expected}).as_object().cloned())), &mut rx, &mut app).await.unwrap();
        let sid = created.session_id;
        assert_eq!(sid.0.as_ref(), expected);
        while let Ok(message) = rx.try_recv() { handle(message, &mut app); }
        crate::app::dispatch::dispatch(Action::SendPrompt("actual-pager-publication-input".into()), &mut app);
        let original = test_agent(&app, id).session.current_prompt_id.clone().unwrap();
        let mut blocked = server.expect_response_blocked("pager resident original RPC",
            InferenceRequestMatcher::foreground_containing(InferenceEndpoint::Responses, "actual-pager-publication-input"),
            ScriptedResponse::sse(xai_grok_test_support::sse::responses_api_script_exact("finished", "test-model")));
        let prompt = acp::Agent::prompt(&*agent, acp::PromptRequest::new(sid.clone(), vec![acp::ContentBlock::from("actual-pager-publication-input")])
            .meta(json!({"promptId": original}).as_object().cloned()));
        tokio::pin!(prompt);
        let mut held = None;
        {
        let reached = blocked.wait_blocked();
        tokio::pin!(reached);
        loop {
            tokio::select! {
                result = &mut prompt => panic!("original inference unexpectedly finished: {result:?}"),
                _ = &mut reached => break,
                message = rx.recv() => {
                    let message = message.unwrap();
                    if held.is_none() { held = Some(message); }
                    else { handle(message, &mut app); }
                }
            }
        }
        }
        held = held.or_else(|| rx.try_recv().ok());
        assert!(held.is_some(), "actual producer must have a held earlier handler");
        for reconnect in [false, true] {
            let view = app.agents.get_mut(&id).unwrap();
            if reconnect { view.begin_session_reload(42); } else { view.begin_replay_window(); }
            let binding = view.begin_publication_load().unwrap();
            let observed = acp::Agent::ext_method(&*agent, acp::ExtRequest::new(
                xai_grok_shell::session::current_actions::CAPABILITY,
                serde_json::value::to_raw_value(&binding).unwrap().into(),
            )).await.unwrap();
            let CurrentActionsSnapshot::Current(before) = serde_json::from_str::<CurrentActionsSnapshot>(observed.0.get()).unwrap()
                else { panic!("actual held resident reader through canonical Get"); };
            assert_eq!(before.turn.as_ref().and_then(|turn| turn.owner.prompt_id()), Some(original.as_str()));
            let mut meta = acp::Meta::new();
            wire::stamp_load_binding(&mut meta, &binding).unwrap();
            let load = acp::Agent::load_session(&*agent, acp::LoadSessionRequest::new(sid.clone(), cwd.path().to_path_buf()).meta(Some(meta)));
            tokio::pin!(load);
            if let Some(message) = held.take() {
                if settlement == xai_acp_lib::ReceiverSettlement::AppliedOrDiscarded {
                    let later = tokio::select! {
                        result = &mut load => panic!("cut overtook the earlier handler: {result:?}"),
                        later = rx.recv() => later.unwrap(),
                    };
                    handle(later, &mut app);
                    assert!(poll_once(load.as_mut()).await.is_pending(), "later completion cannot settle the earlier handler");
                }
                handle(message, &mut app);
            }
            let mut marker = None;
            let response = loop {
                tokio::select! {
                    result = &mut load => break result.unwrap(),
                    result = &mut prompt => panic!("load settled original prompt: {result:?}"),
                    message = rx.recv() => {
                        let message = message.unwrap();
                        if let AcpClientMessage::ExtNotification(args) = &message
                            && args.request.method.as_ref() == wire::GENERATION_END_METHOD {
                            assert!(poll_once(load.as_mut()).await.is_pending(), "marker handler must finish before the load reply");
                            marker = Some(serde_json::from_str::<GenerationEnd>(args.request.params.get()).unwrap());
                        }
                        handle(message, &mut app);
                        if marker.is_some() { assert!(test_agent(&app, id).session.loading_replay); }
                    }
                }
            };
            let snapshot = wire::load_snapshot(response.meta.as_ref()).unwrap();
            let CurrentActionsSnapshot::Current(capture) = snapshot else { panic!("actual resident reader observation"); };
            assert_eq!(capture.binding, binding);
            assert!(capture.turn.is_some());
            assert!(!capture.sampling.is_empty());
            assert_eq!(capture.turn, before.turn);
            assert_eq!(capture.sampling, before.sampling);
            match settlement {
                xai_acp_lib::ReceiverSettlement::AppliedOrDiscarded => {
                    let end = marker.expect("actual producer marker through productive pager handler");
                    assert_eq!(capture, end.snapshot);
                    assert!(matches!(wire::load_cut(response.meta.as_ref()).unwrap(), LoadCut::Settled));
                }
                xai_acp_lib::ReceiverSettlement::TransportOnly => {
                    assert!(marker.is_none());
                    assert!(matches!(wire::load_cut(response.meta.as_ref()).unwrap(), LoadCut::Unavailable { reason: wire::CutUnavailable::UnsupportedTransport }));
                }
            }
            assert!(!test_agent(&app, id).has_current_actions_capture(&capture), "marker alone cannot adopt the snapshot");
            let publication = crate::app::effects::current_actions::LoadPublication::read(binding.clone(), response.meta.as_ref());
            if reconnect {
                crate::app::acp_handler::current_actions::apply_load_publication(&mut app, id, &publication);
                let view = app.agents.get_mut(&id).unwrap();
                assert!(!view.complete_publication_load(&binding, &capture, &publication.cut), "finalize is required");
                assert!(view.finalize_reload_and_maybe_adopt(42, true, Some(original.clone())));
                assert_eq!(view.complete_publication_load(&binding, &capture, &publication.cut), settlement == xai_acp_lib::ReceiverSettlement::AppliedOrDiscarded);
            } else {
                let result = crate::app::effects::current_actions::publication_result(id, Some(binding), response.meta.as_ref(), TaskResult::SessionLoaded {
                    agent_id: id, session_id: sid.clone(), models: response.models, modes: response.modes,
                    code_restored: false, restore_summary: None, restore_degree: None, running_prompt_id: Some(original.clone()),
                });
                crate::app::dispatch::dispatch(Action::TaskComplete(result), &mut app);
            }
            let view = test_agent(&app, id);
            assert!(view.has_current_actions_capture(&capture));
            assert!(!view.session.loading_replay);
            assert!(view.session_reload.is_none());
            if settlement == xai_acp_lib::ReceiverSettlement::AppliedOrDiscarded {
                assert!(matches!(view.load_reconstruction, crate::app::agent_view::load_continuation::LoadReconstruction::Closed));
                assert!(view.late_replay_until.is_none());
            } else {
                assert!(view.late_replay_until.is_some());
            }
            assert_eq!(view.session.current_prompt_id.as_deref(), Some(original.as_str()));
            assert!(poll_once(prompt.as_mut()).await.is_pending());
        }
        blocked.release();
        let response = pump(&mut prompt, &mut rx, &mut app).await.unwrap();
        crate::app::dispatch::dispatch(Action::TaskComplete(TaskResult::PromptResponse {
            agent_id: id, prompt_id: Some(original), http_status: None, result: Ok(response),
        }), &mut app);
        assert!(!test_agent(&app, id).session.state.is_busy());
        let old_actor = test_agent(&app, id).current_actions_context().unwrap().incarnation;
        pump(acp::Agent::close_session(&*agent, acp::CloseSessionRequest::new(sid.clone())), &mut rx, &mut app).await.unwrap();
        let view = app.agents.get_mut(&id).unwrap();
        view.begin_replay_window();
        let binding = view.begin_publication_load().unwrap();
        let mut meta = acp::Meta::new();
        wire::stamp_load_binding(&mut meta, &binding).unwrap();
        let response = pump(acp::Agent::load_session(&*agent, acp::LoadSessionRequest::new(sid.clone(), cwd.path().to_path_buf())
            .meta(Some(meta))), &mut rx, &mut app).await.unwrap();
        let CurrentActionsSnapshot::Current(replacement) = wire::load_snapshot(response.meta.as_ref()).unwrap()
            else { panic!("replacement resident reader"); };
        assert_ne!(replacement.version.incarnation, old_actor);
        assert_eq!(test_agent(&app, id).current_actions_context().unwrap().incarnation, old_actor, "marker cannot attach the replacement actor");
        let result = crate::app::effects::current_actions::publication_result(id, Some(binding), response.meta.as_ref(), TaskResult::SessionLoaded {
            agent_id: id, session_id: sid, models: response.models, modes: response.modes,
            code_restored: false, restore_summary: None, restore_degree: None, running_prompt_id: None,
        });
        crate::app::dispatch::dispatch(Action::TaskComplete(result), &mut app);
        assert_eq!(test_agent(&app, id).current_actions_context().unwrap().incarnation, replacement.version.incarnation);
        assert!(test_agent(&app, id).has_current_actions_capture(&replacement));
        blocked.assert_satisfied();
        }).await.expect("resident publication scenario must finish");
    });
    drop(local);
    drop(keepalive);
}

fn negative_capture(binding: SnapshotBinding, incarnation: uuid::Uuid) -> GenerationEnd {
    use xai_grok_shell::session::current_actions::{
        BackendToolsObservation, CurrentActions, SourceVersion,
    };
    GenerationEnd {
        snapshot: CurrentActions {
            binding,
            version: SourceVersion {
                incarnation,
                sequence: 1,
            },
            event_frontier: 100,
            turn: None,
            sampling: Vec::new(),
            tools: Vec::new(),
            backend_tools: BackendToolsObservation::Unavailable,
            compactions: Vec::new(),
            hooks: Vec::new(),
            interactions: Vec::new(),
        },
    }
}

#[test]
fn stale_generation_reply_and_marker_cannot_take_current_load_custody() {
    let id = AgentId(0);
    let (mut app, _, prompt) = target(id, true);
    let view = app.agents.get_mut(&id).unwrap();
    view.begin_session_reload(5);
    let original = view.begin_publication_load().unwrap();
    let mut stale = original.clone();
    stale.load_generation = stale.load_generation.wrapping_sub(1);
    let end = negative_capture(stale.clone(), uuid::Uuid::new_v4());
    let activity = test_agent(&app, id).last_active_at;
    notification(&mut app, wire::GENERATION_END_METHOD, &end);
    assert!(
        !app.agents
            .get_mut(&id)
            .unwrap()
            .receive_generation_end(end.clone())
    );
    crate::app::dispatch::dispatch(
        Action::TaskComplete(TaskResult::WithPublication {
            agent_id: id,
            binding: stale,
            snapshot: CurrentActionsSnapshot::Current(end.snapshot),
            cut: LoadCut::Settled,
            result: Box::new(TaskResult::SessionLoaded {
                agent_id: id,
                session_id: acp::SessionId::new(original.session_id.clone()),
                models: None,
                modes: None,
                code_restored: false,
                restore_summary: None,
                restore_degree: None,
                running_prompt_id: None,
            }),
        }),
        &mut app,
    );
    let view = test_agent(&app, id);
    assert_eq!(view.current_actions_binding(), Some(original));
    assert!(view.current_actions_context().is_none());
    assert!(view.session.loading_replay);
    assert!(view.session_reload.is_some());
    assert_eq!(
        view.session.current_prompt_id.as_deref(),
        Some(prompt.as_str())
    );
    assert_eq!(view.last_active_at, activity);
}

#[test]
fn missing_role_historical_role_and_foreign_actor_cannot_bootstrap_transition_authority() {
    use xai_grok_shell::session::current_actions::{
        CurrentActionTransition, CurrentActionsEvent, CurrentHook, FrozenOwner, SourceVersion,
    };
    let id = AgentId(0);
    let (mut app, _, prompt) = target(id, true);
    let view = app.agents.get_mut(&id).unwrap();
    let context = crate::app::agent_view::CurrentActionsContext {
        binding: view.current_actions_binding().unwrap(),
        incarnation: uuid::Uuid::new_v4(),
    };
    assert!(view.bind_current_actions(context.clone()));
    let event = CurrentActionsEvent {
        session_id: context.binding.session_id.clone(),
        version: SourceVersion {
            incarnation: context.incarnation,
            sequence: 10,
        },
        transition: CurrentActionTransition::HookStarted {
            hook: CurrentHook {
                batch_id: uuid::Uuid::new_v4(),
                turn: FrozenOwner::Root {
                    prompt_id: prompt,
                    task_id: "negative-transition".into(),
                },
                started_at: MonotonicPoint::at(tokio::time::Instant::now()),
            },
        },
    };
    let activity = test_agent(&app, id).last_active_at;
    for role in [
        None,
        Some(DeliveryRole::Historical {
            binding: context.binding.clone(),
        }),
        Some(DeliveryRole::LiveUnbound {
            session_id: context.binding.session_id.clone(),
            incarnation: uuid::Uuid::new_v4(),
        }),
    ] {
        let mut candidate = event.clone();
        if let Some(DeliveryRole::LiveUnbound { incarnation, .. }) = &role {
            candidate.version.incarnation = *incarnation;
        }
        let mut payload = serde_json::to_value(&candidate).unwrap();
        if let Some(role) = role {
            let mut meta = acp::Meta::new();
            wire::stamp_delivery(&mut meta, &role).unwrap();
            payload
                .as_object_mut()
                .unwrap()
                .insert("_meta".into(), serde_json::to_value(meta).unwrap());
        }
        notification(
            &mut app,
            xai_grok_shell::session::current_actions::TRANSITION_METHOD,
            &payload,
        );
        let view = test_agent(&app, id);
        assert_eq!(view.current_actions_context(), Some(&context));
        assert_eq!(view.current_action_started_at(), None);
        assert_eq!(view.last_active_at, activity);
    }
}

#[test]
fn explicit_discard_completes_without_row_clock_ack_or_reconnect_cursor() {
    use xai_grok_shell::session::current_actions::SourceVersion;
    for historical in [false, true] {
        let id = AgentId(0);
        let (mut app, sid, prompt) = target(id, true);
        let view = app.agents.get_mut(&id).unwrap();
        let context = crate::app::agent_view::CurrentActionsContext {
            binding: view.current_actions_binding().unwrap(),
            incarnation: uuid::Uuid::new_v4(),
        };
        view.bind_current_actions(context.clone());
        view.advance_last_seen_event_id(format!("{sid}-7"), Some(7));
        let rows = view.scrollback.len();
        let activity = view.last_active_at;
        let ack = view.prompt_ack.is_some();
        let role = if historical {
            DeliveryRole::Historical {
                binding: context.binding.clone(),
            }
        } else {
            DeliveryRole::Live {
                context: wire::PublicationContext {
                    binding: context.binding.clone(),
                    incarnation: context.incarnation,
                },
            }
        };
        let record = wire::DiscardedRecord { role,
            notification: wire::DiscardedNotification::Acp(Box::new(acp::SessionNotification::new(acp::SessionId::new(sid),
                acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new(acp::ContentBlock::from("discarded original must not render"))))
                .meta(json!({"eventId": format!("{sid}-8"), "promptId": prompt,
                    "currentActionsVersion": SourceVersion { incarnation: context.incarnation, sequence: 2 }}).as_object().cloned()))),
        };
        notification(&mut app, wire::DISCARD_METHOD, &record);
        let view = test_agent(&app, id);
        assert_eq!(view.scrollback.len(), rows);
        assert_eq!(view.last_active_at, activity);
        assert_eq!(view.prompt_ack.is_some(), ack);
        assert_eq!(view.last_seen_event_seq, Some(7));
        assert_eq!(
            view.last_seen_event_id.as_deref(),
            Some(format!("{sid}-7").as_str())
        );
        assert_eq!(view.current_action_started_at(), None);
        assert_eq!(view.current_actions_context(), Some(&context));
    }
}
