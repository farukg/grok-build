use super::*;
use crate::session::delivery_receiver::ParentMessageSource;
use xai_grok_tools::types::restart_recovery::{LeaderRestartRecovery, RestartAdmission};

#[tokio::test(flavor = "current_thread")]
async fn old_human_custody_cannot_reactivate_an_unregistered_native_child() {
    xai_test_utils::require_git!();
    tokio::task::LocalSet::new().run_until(async {
        let f = lost_owner_from(ParentInputFault::ShutdownAfterAdmission, TestArtifacts::Disabled,
            crate::agent::subagent::ActiveAgentMessageSource::Human).await;
        let old = f.storage.load_summary(&f.info).await.unwrap();
        let DeliveryProvenance::ParentMessage { admission } = &old.completion_delivery.receiver.records[&f.reference.delivery_id].provenance else {
            panic!("real persisted Human custody required");
        };
        assert_eq!(admission.source, ParentMessageSource::Human);
        let startup = LeaderRestartRecovery::stale(chrono::Utc::now());
        f.agent.cfg.borrow_mut().leader_restart_recovery = startup.clone();
        let sampled = f.server.request_count();
        let (sender, owner) = coordinator(&f.agent, None, None);
        let backend = ChannelBackend::for_coordinator_session(sender, f.parent.0.to_string());
        let outcome = bounded(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt)).await;
        assert!(!matches!(outcome, ActiveAgentMessageOutcome::Accepted { .. }), "old custody is not a new submission: {outcome:?}");
        bounded(backend.query(f.info.id.0.as_ref(), true, Some(30_000), None)).await.expect("query acknowledgement");
        let durable = f.storage.load_summary(&f.info).await.unwrap();
        assert!(matches!(durable.completion_delivery.restart_admission, RestartAdmission::Quiescent { .. }));
        assert_eq!(startup.register(f.info.id.0.as_ref(), old.created_at, old.completion_delivery.restart_admission).admission(),
            durable.completion_delivery.restart_admission);
        assert!(matches!(durable.completion_delivery.receiver.records[&f.reference.delivery_id].execution, DeliveryExecution::Cancelled { .. }));
        assert_eq!(f.server.request_count(), sampled);
        assert_eq!(std::fs::read(f.directory.join("chat_history.jsonl")).unwrap(), f.prior_history);
        drop(backend);
        drop(owner);
    }).await;
}
