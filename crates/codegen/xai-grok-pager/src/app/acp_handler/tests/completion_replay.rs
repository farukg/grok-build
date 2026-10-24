use super::*;
use xai_grok_tools::implementations::grok_build::task::types::{
    InterruptionCause, SubagentActor, SubagentState,
};

#[tokio::test(flavor = "current_thread")]
async fn native_completion_replay_has_one_visible_terminal_effect() {
    tokio::task::LocalSet::new().run_until(async {
        for (state, status, error) in [
            (SubagentState::Completed, "completed", None),
            (SubagentState::Failed { message: "Subagent initial prompt was not admitted before the deadline".into() },
                "failed", Some("Subagent initial prompt was not admitted before the deadline")),
            (SubagentState::Interrupted { cause: InterruptionCause::ExplicitStop { actor: SubagentActor::Human } },
                "cancelled", Some("Subagent was stopped by human user.")),
        ] {
            let replay = tokio::time::timeout(std::time::Duration::from_secs(10),
                xai_grok_shell::agent::testkit::completion_replay_for_e2e(state.clone()))
                .await.expect("real coordinator, producer and replay FIFO");
            assert_eq!(replay.outcome.state, state);
            assert_eq!(replay.metadata.error.as_deref(), error);
            assert!(replay.metadata.completed_at.is_some());
            let mut app = make_app_with_agent(&replay.parent_session_id.to_string());
            let mut raw_finishes = 0;
            let mut event_id = None;
            let mut terminal_entry = None;
            for message in replay.messages {
                let notification: xai_grok_shell::extensions::notification::SessionNotification =
                    serde_json::from_str(message.params.get()).expect("actual emitted lifecycle");
                if matches!(&notification.update, XaiSessionUpdate::SubagentFinished { subagent_id, .. }
                    if subagent_id == &replay.child_session_id) {
                    raw_finishes += 1;
                    let id = notification.meta.as_ref().unwrap()["eventId"].as_str().unwrap();
                    match &event_id {
                        Some(first) => assert_eq!(id, first),
                        None => event_id = Some(id.to_owned()),
                    }
                }
                handle_ext_notification(&message, &mut app);
                let agent = test_agent(&app, AgentId(0));
                if let Some(info) = agent.subagent_sessions.get(&replay.child_session_id)
                    && info.is_finished()
                    && let Some(entry) = if info.attempt.is_background {
                        info.attempt.terminal_entry_id
                    } else {
                        info.attempt.scrollback_entry_id
                    } {
                    match terminal_entry {
                        Some(first) => assert_eq!(entry, first, "replay must preserve the visible row identity"),
                        None => terminal_entry = Some(entry),
                    }
                }
            }
            assert!(raw_finishes > 1, "the real replay must reach the consumer, not be omitted from the oracle");
            let agent = test_agent(&app, AgentId(0));
            let info = agent.subagent_sessions.get(&replay.child_session_id).expect("real child lifecycle");
            assert!(info.is_finished());
            assert_eq!(info.attempt.status.as_deref(), Some(status));
            assert_eq!(info.attempt.error.as_deref(), error);
            let visible = agent.scrollback.iter_entries().filter(|(_, entry)| matches!(&entry.block,
                RenderBlock::Subagent(block) if block.child_session_id == replay.child_session_id
                    && matches!(&block.kind, crate::scrollback::blocks::SubagentBlockKind::Completed { .. }
                        | crate::scrollback::blocks::SubagentBlockKind::Failed { .. }
                        | crate::scrollback::blocks::SubagentBlockKind::Cancelled { .. }))).count();
            assert_eq!(visible, 1, "all live redeliveries must have one visible terminal effect");
            let entry = agent.scrollback.get_by_id(terminal_entry.expect("visible terminal row after all deliveries")).unwrap();
            assert!(!entry.is_running);
            let RenderBlock::Subagent(block) = &entry.block else { panic!("real visible child lifecycle row") };
            assert!(matches!((&block.kind, status),
                (crate::scrollback::blocks::SubagentBlockKind::Completed { .. }, "completed")
                | (crate::scrollback::blocks::SubagentBlockKind::Failed { .. }, "failed")
                | (crate::scrollback::blocks::SubagentBlockKind::Cancelled { .. }, "cancelled")));
            if let crate::scrollback::blocks::SubagentBlockKind::Failed { error: rendered_error, .. } = &block.kind {
                assert_eq!(rendered_error.as_deref(), error);
            }
        }
    }).await;
}
