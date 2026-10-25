use super::{WorkspaceRpcHandler, background_capable_cfg, make_handle};
use crate::capability::CapabilityMode;
use std::sync::Arc;
use std::time::Duration;
use xai_grok_tools::computer::types::{
    BackgroundHandle, ComputerError, TaskSnapshot, TerminalBackend, TerminalRunRequest,
    TerminalRunResult,
};
use xai_grok_tools::types::resources::Terminal;
use xai_grok_workspace_types::rpc::workspace::KillTaskResponse;
use xai_tool_types::{KillFailureReason, KillOutcome, KillProcessEffect, KillStage};

struct NegativeKillTerminal(KillOutcome);

#[async_trait::async_trait]
impl TerminalBackend for NegativeKillTerminal {
    async fn run(&self, _: TerminalRunRequest) -> Result<TerminalRunResult, ComputerError> {
        panic!("kill RPC must not run a command")
    }

    async fn run_background(&self, _: TerminalRunRequest) -> Result<BackgroundHandle, ComputerError> {
        panic!("kill RPC must not start a command")
    }

    async fn get_task(&self, _: &str) -> Option<TaskSnapshot> {
        None
    }

    async fn kill_task(&self, task_id: &str) -> KillOutcome {
        assert_eq!(task_id, "task-1");
        self.0
    }

    async fn wait_for_completion(&self, _: &str, _: Option<Duration>) -> Option<TaskSnapshot> {
        panic!("kill RPC must not wait for a successful completion")
    }

    async fn list_tasks(&self) -> Vec<TaskSnapshot> {
        Vec::new()
    }
}

#[tokio::test]
async fn workspace_kill_rpc_preserves_negative_outcome_evidence() {
    for outcome in [
        KillOutcome::Indeterminate {
            stage: KillStage::ProcessTermination,
            process_effect: KillProcessEffect::Unknown,
        },
        KillOutcome::Indeterminate {
            stage: KillStage::ProcessTermination,
            process_effect: KillProcessEffect::StopRequested,
        },
        KillOutcome::Indeterminate {
            stage: KillStage::DurableEnrollment,
            process_effect: KillProcessEffect::TerminalObserved,
        },
        KillOutcome::Failed {
            stage: KillStage::ProcessTermination,
            reason: KillFailureReason::PermissionDenied,
        },
    ] {
        let handle = make_handle();
        let cfg = background_capable_cfg();
        let session = handle.create_session_with_config(
            "negative-kill-rpc", None, Some(cfg.clone()), CapabilityMode::All, None, false,
        ).expect("session");
        session.set_bind_tool_config_fingerprint(serde_json::to_value(&cfg).ok());
        session.toolset().resources.lock().await.insert(Terminal(Arc::new(NegativeKillTerminal(outcome))));
        let handler = WorkspaceRpcHandler::new(handle);
        let wire = handler.dispatch(
            "workspace.kill_task",
            serde_json::json!({"session_id": "negative-kill-rpc", "task_id": "task-1"}),
            Some("negative-kill-rpc"),
        ).await.expect("negative outcome is a typed RPC result");
        let response: KillTaskResponse = serde_json::from_value(wire).expect("typed response");
        assert_eq!(response.task_id, "task-1");
        assert_eq!(response.outcome, outcome);
    }
}
