use super::*;

#[test]
fn stale_acp_child_submission_reaches_durable_native_inference_without_parent_reactivation() {
    xai_test_utils::require_git!();
    run_local_for_bridge_test(|| successful_wake_via(ActiveAgentMessageSource::Human, WakeIngress::RestartAcp));
}
