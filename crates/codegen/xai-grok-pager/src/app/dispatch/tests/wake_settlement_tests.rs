use super::*;
use crate::app::agent_view::{CurrentActionsContext, RunningWakeTurn};
use xai_grok_shell::session::{WakeCancelFailure, WakeCancelRequest, WakeCancelResponse, WakeCancelSettlement};

fn stop(app: &mut crate::app::app_view::AppView, trigger: crate::app::actions::CancelTrigger) -> WakeCancelRequest {
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    let context = CurrentActionsContext {
        binding: agent.current_actions_binding().unwrap(),
        incarnation: uuid::Uuid::new_v4(),
    };
    assert!(agent.bind_current_actions(context));
    agent.running_wake_turn = Some(RunningWakeTurn {
        prompt_id: "task-completed-stuck".into(),
        cancel_sent: false,
    });
    agent.cancel_trigger_hint = Some(trigger);
    let effects = dispatch(Action::CancelTurn, app);
    let [Effect::CancelWake { request }] = effects.as_slice() else {
        panic!("Stop must request an actor-owned settlement, not an anonymous cancel or turn");
    };
    request.clone()
}

fn deliver(app: &mut crate::app::app_view::AppView, request: &WakeCancelRequest, response: WakeCancelResponse) {
    let effects = dispatch(Action::TaskComplete(TaskResult::WakeCancelComplete {
        request: request.clone(),
        result: Ok(response),
    }), app);
    assert!(effects.is_empty(), "settlement must never submit or drain a prompt");
}

fn response(request: &WakeCancelRequest, settlement: WakeCancelSettlement) -> WakeCancelResponse {
    WakeCancelResponse {
        context: request.context.clone(),
        prompt_id: request.prompt_id.clone(),
        settlement,
    }
}

#[test]
fn stop_and_ctrl_c_clear_waiting_only_after_queue_or_original_terminal_receipt() {
    for trigger in [crate::app::actions::CancelTrigger::Mouse, crate::app::actions::CancelTrigger::CtrlC] {
        for terminal in [false, true] {
            let mut app = test_app_with_agent();
            let request = stop(&mut app, trigger);
            assert!(get_agent(&app, AgentId(0)).wake_turn_cancelling());
            let settlement = if terminal {
                WakeCancelSettlement::Terminal {
                    notification: xai_grok_shell::extensions::notification::SessionNotification {
                        session_id: acp::SessionId::new(request.context.binding.session_id.clone()),
                        update: xai_grok_shell::extensions::notification::SessionUpdate::TurnCompleted {
                            prompt_id: request.prompt_id.clone(),
                            stop_reason: "error".into(),
                            agent_result: Some("original durable error".into()),
                            error_kind: None,
                            usage: None,
                            elapsed_ms: Some(1291),
                        },
                        meta: Some(serde_json::json!({"eventId": "original-terminal"})),
                    },
                }
            } else {
                WakeCancelSettlement::RemovedFromQueue
            };
            let receipt = response(&request, settlement);
            deliver(&mut app, &request, receipt.clone());
            let agent = get_agent(&app, AgentId(0));
            assert!(agent.session.state.is_idle());
            assert!(!agent.any_cancel_pending());
            assert!(agent.running_wake_turn.is_none());
            assert!(agent.pending_cancel_resend.is_none());
            assert!(agent.finished_wake_prompts.contains(&request.prompt_id));
            assert_eq!(agent.scrollback.len(), 0, "neither queue removal nor original failure invents Cancelled");
            deliver(&mut app, &request, receipt);
            assert_eq!(get_agent(&app, AgentId(0)).scrollback.len(), 0);
        }
    }
}

#[test]
fn unknown_failed_cross_session_incarnation_and_target_receipts_do_not_close_waiting() {
    for invalid in 0..6 {
        let mut app = test_app_with_agent();
        let request = stop(&mut app, crate::app::actions::CancelTrigger::CtrlC);
        let mut receipt = response(&request, WakeCancelSettlement::RemovedFromQueue);
        match invalid {
            0 => receipt.context.binding.session_id = "other-session".into(),
            1 => receipt.context.incarnation = uuid::Uuid::new_v4(),
            2 => receipt.prompt_id = "task-completed-other".into(),
            3 => receipt.context.binding.load_generation += 1,
            4 => receipt.settlement = WakeCancelSettlement::Unresolved { reason: WakeCancelFailure::UnknownTarget },
            5 => receipt.settlement = WakeCancelSettlement::Unresolved { reason: WakeCancelFailure::PersistenceUnavailable },
            _ => unreachable!(),
        }
        deliver(&mut app, &request, receipt);
        assert!(get_agent(&app, AgentId(0)).wake_turn_cancelling());
        assert!(!get_agent(&app, AgentId(0)).finished_wake_prompts.contains(&request.prompt_id));
    }
}

#[test]
fn delayed_receipt_preserves_new_user_prompt_and_wake_successor() {
    for successor_wake in [false, true] {
        let mut app = test_app_with_agent();
        let request = stop(&mut app, crate::app::actions::CancelTrigger::Mouse);
        {
            let agent = app.agents.get_mut(&AgentId(0)).unwrap();
            if successor_wake {
                agent.running_wake_turn = Some(RunningWakeTurn {
                    prompt_id: "task-completed-successor".into(),
                    cancel_sent: false,
                });
            } else {
                agent.session.state = AgentState::TurnRunning;
                agent.session.current_prompt_id = Some("new-user-prompt".into());
            }
        }
        deliver(&mut app, &request, response(&request, WakeCancelSettlement::RemovedFromQueue));
        let agent = get_agent(&app, AgentId(0));
        if successor_wake {
            assert_eq!(agent.running_wake_turn.as_ref().unwrap().prompt_id, "task-completed-successor");
            assert!(!agent.running_wake_turn.as_ref().unwrap().cancel_sent);
        } else {
            assert!(agent.session.state.is_turn_running());
            assert_eq!(agent.session.current_prompt_id.as_deref(), Some("new-user-prompt"));
            assert!(agent.running_wake_turn.is_none());
        }
    }
}

#[test]
fn repeated_ctrl_c_retries_same_wake_until_receipt_without_quitting() {
    use crate::app::app_view::InputOutcome;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    let mut app = test_app_with_agent();
    let request = stop(&mut app, crate::app::actions::CancelTrigger::CtrlC);
    let registry = crate::actions::ActionRegistry::defaults();
    for _ in 0..4 {
        let outcome = app.agents.get_mut(&AgentId(0)).unwrap().handle_input(
            &Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            &registry,
        );
        let InputOutcome::Action(Action::CancelTurn) = outcome else {
            panic!("unresolved wake cancellation must retry Stop, never Quit");
        };
        let effects = dispatch(Action::CancelTurn, &mut app);
        let [Effect::CancelWake { request: retry }] = effects.as_slice() else {
            panic!("repeated Ctrl+C must keep its bound settlement target");
        };
        assert_eq!(retry.context, request.context);
        assert_eq!(retry.prompt_id, request.prompt_id);
        assert!(get_agent(&app, AgentId(0)).wake_turn_cancelling());
    }
    deliver(&mut app, &request, response(&request, WakeCancelSettlement::RemovedFromQueue));
    assert!(!get_agent(&app, AgentId(0)).any_cancel_pending());
}

#[test]
fn lost_receipt_auto_retry_keeps_target_and_does_not_fabricate_terminal() {
    let mut app = test_app_with_agent();
    let request = stop(&mut app, crate::app::actions::CancelTrigger::Mouse);
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        let pending = agent.pending_cancel_resend.as_mut().unwrap();
        pending.sent_at = Instant::now() - super::super::super::turn::CANCEL_RESEND_GRACE;
    }
    let effects = super::super::super::turn::reconcile_overdue_cancels(&mut app).unwrap();
    let [Effect::CancelWake { request: retry }] = effects.as_slice() else {
        panic!("lost settlement receipt must retry the actor-owned request");
    };
    assert_eq!(retry.context, request.context);
    assert_eq!(retry.prompt_id, request.prompt_id);
    assert!(get_agent(&app, AgentId(0)).wake_turn_cancelling());
    deliver(&mut app, retry, response(retry, WakeCancelSettlement::RemovedFromQueue));
    assert!(!get_agent(&app, AgentId(0)).any_cancel_pending());
    assert!(get_agent(&app, AgentId(0)).scrollback.is_empty());
}

#[test]
fn delayed_receipt_after_rebind_is_discarded() {
    let mut app = test_app_with_agent();
    let request = stop(&mut app, crate::app::actions::CancelTrigger::CtrlC);
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.session_binding_epoch += 1;
    deliver(&mut app, &request, response(&request, WakeCancelSettlement::RemovedFromQueue));
    assert!(get_agent(&app, AgentId(0)).running_wake_turn.is_some());
}
