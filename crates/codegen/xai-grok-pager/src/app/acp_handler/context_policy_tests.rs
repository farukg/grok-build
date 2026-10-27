use super::super::tests::make_app_two_agents;
use crate::actions::ActionRegistry;
use crate::app::actions::Action;
use crate::app::agent::AgentId;
use crate::app::app_view::InputOutcome;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use xai_grok_shell::extensions::context_policy::{CONTEXT_POLICY_CHANGED, ContextPolicyNotification, ContextPolicyReport, ContextPolicyResponse};
use xai_grok_shell::sampling::{ContextCategory, ContextPolicy, ContextPolicyMutation, ContextPolicySelection, ContextPolicyVersion, ContextScope, ContextSwitch};

#[test]
fn context_push_updates_the_origin_view_not_the_active_other_session() {
    let _theme = crate::theme::cache::pin_theme();
    let mut app = make_app_two_agents();
    for (_, agent) in app.agents.all_mut() {
        let _ = agent.handle_input(&Event::Key(KeyEvent::new(KeyCode::F(7), KeyModifiers::NONE)), &ActionRegistry::defaults());
        agent.context_policy_answered(Ok(ContextPolicyReport {
            policy: ContextPolicy::default(), usage: Vec::new(),
            selection: Some(ContextPolicySelection {
                scope: ContextScope::Session,
                version: ContextPolicyVersion { revision: 1, incarnation: "old".into() },
            }),
        }));
    }
    let mut policy = ContextPolicy::default();
    policy.set(ContextCategory::CoreInstructions, ContextSwitch::Excluded);
    let push = ContextPolicyNotification {
        session_id: "sess-owner".into(),
        response: ContextPolicyResponse::Confirmed { report: ContextPolicyReport {
            policy, usage: Vec::new(),
            selection: Some(ContextPolicySelection {
                scope: ContextScope::Global,
                version: ContextPolicyVersion { revision: 2, incarnation: "owner".into() },
            }),
        } },
    };
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    assert!(crate::app::acp_handler::handle(xai_acp_lib::AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
        request: xai_grok_test_support::acp_fixtures::ext_notification(CONTEXT_POLICY_CHANGED, &push), response_tx,
    }), &mut app));
    assert_eq!(app.agents.get(&AgentId(0)).unwrap().context_policy_generation, 1);
    assert_eq!(app.agents.get(&AgentId(1)).unwrap().context_policy_generation, 0);
    assert!(!app.agents.get(&AgentId(1)).unwrap().context_confirmation_pending());
    let owner = app.agents.get_mut(&AgentId(0)).unwrap();
    let _ = owner.handle_input(&Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)), &ActionRegistry::defaults());
    assert!(owner.context_confirmation_pending());
    let InputOutcome::Action(Action::SetContextPolicy(edit)) = owner.handle_input(
        &Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)), &ActionRegistry::defaults(),
    ) else { panic!("scope confirmation comes from the pushed owner state"); };
    assert_eq!(edit.expected.revision, 2);
    assert_eq!(edit.expected.incarnation, "owner");
    assert_eq!(edit.mutation, ContextPolicyMutation::SelectScope { scope: ContextScope::Session });
}
