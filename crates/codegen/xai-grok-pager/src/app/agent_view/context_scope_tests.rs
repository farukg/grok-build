use super::{draw, open_sidebar, press, ready_report, screen, selection, sidebar_row};
use crate::app::actions::{Action, ContextPolicyTransportError};
use crate::app::app_view::InputOutcome;
use crossterm::event::KeyCode;
use xai_grok_shell::extensions::context_policy::{ContextPolicyReport, ContextPolicyResponse};
use xai_grok_shell::sampling::{CategoryTokens, ContextCategory, ContextPolicyMutation, ContextScope, ContextSwitch};

fn global_report(revision: u64) -> ContextPolicyReport {
    ContextPolicyReport { selection: selection(ContextScope::Global, revision), ..ready_report() }
}

#[test]
fn global_category_warning_starts_on_cancel_and_the_session_alternative_targets_only_that_category() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    agent.context_policy_answered(Ok(global_report(10)));
    assert!(matches!(press(&mut agent, KeyCode::Char(' ')), InputOutcome::Changed));
    let warning = screen(&draw(&mut agent));
    assert!(warning.contains("Confirm Context"));
    assert!(warning.contains("› c  Cancel"));
    assert!(matches!(press(&mut agent, KeyCode::Enter), InputOutcome::Changed));
    assert!(!agent.context_confirmation_pending());
    assert!(sidebar_row(&draw(&mut agent), "Core instructions").contains("ON"));
    let _ = press(&mut agent, KeyCode::Char(' '));
    let InputOutcome::Action(Action::SetContextPolicy(edit)) = press(&mut agent, KeyCode::Char('s')) else { panic!("explicit Session admission"); };
    assert_eq!(edit.expected.revision, 10);
    assert_eq!(edit.mutation, ContextPolicyMutation::SetCategories {
        categories: vec![ContextCategory::CoreInstructions].try_into().unwrap(), value: ContextSwitch::Excluded, target: ContextScope::Session,
    });
    assert!(screen(&draw(&mut agent)).contains("Applying"));
    assert!(sidebar_row(&draw(&mut agent), "Core instructions").contains("ON"), "no optimistic policy change");
}

#[test]
fn context_scope_warns_in_both_directions_and_display_g_remains_independent() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    let settings = tempfile::tempdir().unwrap();
    agent.open_panel_settings_in(settings.path());
    for (from, target, key, revision) in [
        (ContextScope::Session, ContextScope::Global, 'g', 2),
        (ContextScope::Global, ContextScope::Session, 's', 3),
    ] {
        agent.context_policy_answered(Ok(ContextPolicyReport { selection: selection(from, revision), ..ready_report() }));
        let display_scope = agent.panel_persistence.display_layer();
        assert!(matches!(press(&mut agent, KeyCode::Char('g')), InputOutcome::Changed));
        assert_ne!(agent.panel_persistence.display_layer(), display_scope);
        assert!(!agent.context_confirmation_pending(), "g still controls Message Display only");
        assert!(matches!(press(&mut agent, KeyCode::Char('c')), InputOutcome::Changed));
        assert!(agent.context_confirmation_pending());
        let InputOutcome::Action(Action::SetContextPolicy(edit)) = press(&mut agent, KeyCode::Char(key)) else { panic!("explicit scope confirmation"); };
        assert_eq!(edit.mutation, ContextPolicyMutation::SelectScope { scope: target });
        assert_eq!(edit.expected.revision, revision);
    }
}

#[test]
fn notification_or_origin_session_change_invalidates_a_pending_confirmation() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    let settings = tempfile::tempdir().unwrap();
    agent.open_panel_settings_in(settings.path());
    let display_scope = agent.panel_persistence.display_layer();
    agent.context_policy_answered(Ok(global_report(4)));
    let _ = press(&mut agent, KeyCode::Char(' '));
    let mut newer = global_report(5);
    newer.policy.set(ContextCategory::CoreInstructions, ContextSwitch::Excluded);
    agent.context_policy_notified(ContextPolicyResponse::Confirmed { report: newer });
    assert!(agent.context_confirmation_pending(), "stale warning consumes its next key instead of leaking to Display");
    assert!(screen(&draw(&mut agent)).contains("confirmation expired"));
    assert!(matches!(press(&mut agent, KeyCode::Char('g')), InputOutcome::Changed));
    assert!(!agent.context_confirmation_pending());
    assert_eq!(agent.panel_persistence.display_layer(), display_scope);
    assert!(sidebar_row(&draw(&mut agent), "Core instructions").contains("OFF"));
    let _ = press(&mut agent, KeyCode::Char(' '));
    agent.session.session_id = Some(agent_client_protocol::SessionId::new("another-session"));
    assert!(matches!(press(&mut agent, KeyCode::Enter), InputOutcome::Changed));
    assert!(!agent.context_confirmation_pending());
    assert!(screen(&draw(&mut agent)).contains("Unavailable"));
}

#[test]
fn old_request_answers_cannot_overwrite_a_push_or_a_reopened_sidebar() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    let session = agent.session.session_id.clone().unwrap();
    let old = agent.context_policy_request_started().unwrap();
    let mut newest = global_report(7);
    newest.policy.set(ContextCategory::CoreInstructions, ContextSwitch::Excluded);
    agent.context_policy_notified(ContextPolicyResponse::Confirmed { report: newest });
    agent.context_policy_response_for(&session, old, Ok(ContextPolicyResponse::Confirmed { report: global_report(6) }));
    assert!(sidebar_row(&draw(&mut agent), "Core instructions").contains("OFF"));
    let _ = press(&mut agent, KeyCode::F(7));
    let _ = press(&mut agent, KeyCode::F(7));
    let current = agent.context_policy_request_started().unwrap();
    assert!(current > old);
    agent.context_policy_response_for(&session, old, Ok(ContextPolicyResponse::Confirmed { report: global_report(6) }));
    assert!(screen(&draw(&mut agent)).contains("Loading"));
}

#[test]
fn unknown_counter_is_not_zero_and_transport_loss_never_restores_an_assumed_policy() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    assert!(sidebar_row(&draw(&mut agent), "Core instructions").contains("~?"));
    let mut report = global_report(1);
    report.usage = vec![CategoryTokens { category: ContextCategory::CoreInstructions, tokens: 0 }];
    agent.context_policy_answered(Ok(report));
    assert!(sidebar_row(&draw(&mut agent), "Core instructions").contains("~0"));
    let _ = press(&mut agent, KeyCode::Char(' '));
    let _ = press(&mut agent, KeyCode::Char('g'));
    agent.context_policy_answered(Err(ContextPolicyTransportError::Rpc(agent_client_protocol::Error::internal_error())));
    assert!(screen(&draw(&mut agent)).contains("Unavailable"));
    assert!(!sidebar_row(&draw(&mut agent), "Core instructions").contains("ON"));
    let _ = press(&mut agent, KeyCode::Enter);
    assert!(agent.block_viewer.is_some(), "Enter inspects rather than changes policy");
    assert!(screen(&draw(&mut agent)).contains("unavailable"));
}
