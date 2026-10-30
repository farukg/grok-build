use super::*;

impl SessionActor {
    pub(super) fn restart_recovery_permitted(&self) -> bool {
        self.tool_context.restart_recovery.admission().permits_recovery()
    }

    pub(super) async fn cancel_stale_session_work(&self) -> Result<(), acp::Error> {
        if matches!(self.tool_context.restart_recovery.admission(),
            xai_grok_tools::types::restart_recovery::RestartAdmission::NewSession { .. }) {
            self.persist_restart_admission(self.tool_context.restart_recovery.admission()).await?;
        }
        if self.restart_recovery_permitted() {
            return Ok(());
        }
        if let Some(event_tx) = self.tool_context.subagent_event_tx.clone() {
            use xai_grok_tools::implementations::grok_build::task::types::{SubagentEvent, SubagentCancelOutcome};
            let (respond_to, response) = oneshot::channel();
            event_tx.send(SubagentEvent::QuiesceRestartChildren {
                parent_session_id: self.session_id_string(), respond_to,
            }).map_err(|_| restart_owner_closed())?;
            match response.await.map_err(|_| restart_owner_closed())? {
                SubagentCancelOutcome::Cancelled { .. }
                | SubagentCancelOutcome::AlreadyFinished { .. } => {},
                SubagentCancelOutcome::NotFound => return Err(restart_owner_closed()),
                SubagentCancelOutcome::CustodyUnavailable { target_id } => {
                    return Err(acp::Error::internal_error().data(format!(
                        "Subagent '{target_id}' cancellation not confirmed: execution custody is unavailable"
                    )));
                }
            }
        }
        let bridge = self.tool_bridge_handle();
        let owner = self.session_id_string();
        let tasks = bridge.list_background_tasks().await;
        for task in tasks.iter().filter(|task| !task.completed
            && task.owner_session_id.as_deref() == Some(owner.as_str())) {
            use xai_grok_tools::types::{KillOutcome, KillSource};
            let outcome = bridge.kill_background_task(&task.task_id, KillSource::Teardown).await
                .map_err(|error| acp::Error::internal_error().data(error.to_string()))?;
            match outcome {
                KillOutcome::Killed | KillOutcome::AlreadyExited | KillOutcome::NotFound => {},
                KillOutcome::Indeterminate { .. } | KillOutcome::Failed { .. } => {
                    return Err(acp::Error::internal_error().data("stale restart task cancellation not acknowledged"));
                }
            }
        }
        let mut manager = self.workflow_manager.lock().await;
        let runs = manager.tracker().lock().snapshot();
        for run in runs {
            use crate::session::workflow::tracker::WorkflowRunStatus;
            match run.status {
                WorkflowRunStatus::Active | WorkflowRunStatus::Interrupted
                | WorkflowRunStatus::UserPaused | WorkflowRunStatus::BackOffPaused
                | WorkflowRunStatus::NoProgressPaused | WorkflowRunStatus::InfraPaused
                | WorkflowRunStatus::Blocked | WorkflowRunStatus::BudgetLimited => {
                    manager.cancel_stale_restart(&run.run_id);
                    manager.persist_control_result(&run.run_id).await
                        .map_err(|error| acp::Error::internal_error().data(error.to_string()))?;
                }
                WorkflowRunStatus::Complete | WorkflowRunStatus::Failed
                | WorkflowRunStatus::Cancelled => {},
            }
        }
        Ok(())
    }

    pub(super) async fn reactivate_for_new_user_message(&self) -> Result<(), acp::Error> {
        use xai_grok_tools::types::restart_recovery::RestartAdmission;
        match self.tool_context.restart_recovery.admission() {
            RestartAdmission::Unaffected | RestartAdmission::Reactivated { .. }
            | RestartAdmission::NewSession { .. } => Ok(()),
            RestartAdmission::Quiescent { .. } => {
                reactivate(&self.notifications.persistence_tx, &self.tool_context.restart_recovery).await?;
                self.open_subagent_spawn_admission();
                Ok(())
            }
        }
    }

    async fn persist_restart_admission(&self, admission: xai_grok_tools::types::restart_recovery::RestartAdmission) -> Result<(), acp::Error> {
        persist_admission(&self.notifications.persistence_tx, admission).await
    }
}

impl crate::session::SessionHandle {
    /// Only the fresh user-submission owner may call this, never a load or recovery wake.
    pub(crate) async fn admit_new_user_message_after_restart(&self) -> Result<(), acp::Error> {
        reactivate(&self.persistence_tx, &self.tool_context.restart_recovery).await
    }
}

async fn reactivate(
    persistence: &mpsc::UnboundedSender<PersistenceMsg>,
    recovery: &xai_grok_tools::types::restart_recovery::SessionRestartRecovery,
) -> Result<(), acp::Error> {
    let _activation = recovery.activation.lock().await;
    let admission = recovery.admission();
    if admission.permits_recovery() {
        return Ok(());
    }
    let state = completion_state(persistence).await?;
    let ids = state.obligations.iter().filter_map(|obligation| {
        match obligation {
            crate::session::completion_delivery::CompletionObligation::Suppressed { .. } => None,
            crate::session::completion_delivery::CompletionObligation::Preparing { .. }
            | crate::session::completion_delivery::CompletionObligation::Pending { .. }
            | crate::session::completion_delivery::CompletionObligation::Projected { .. } => Some(obligation.id()),
        }
    }).collect::<Vec<_>>();
    if !ids.is_empty() {
        let (respond_to, response) = oneshot::channel();
        persistence.send(PersistenceMsg::SuppressTerminalCompletions {
            ids,
            reason: crate::session::completion_delivery::CompletionSuppression::StaleRestart,
            respond_to,
        }).map_err(|_| restart_owner_closed())?;
        response.await.map_err(|_| restart_owner_closed())?
            .map_err(|error| acp::Error::internal_error().data(error.to_string()))?;
    }
    let state = completion_state(persistence).await?;
    if !state.receiver.recoverable_ids().is_empty()
        || state.obligations.iter().any(|obligation| !matches!(obligation,
            crate::session::completion_delivery::CompletionObligation::Suppressed { .. }))
    {
        return Err(acp::Error::internal_error().data("stale restart work has not durably settled"));
    }
    let admission = admission.after_user_message();
    persist_admission(persistence, admission).await?;
    recovery.acknowledge(admission);
    Ok(())
}

async fn persist_admission(
    persistence: &mpsc::UnboundedSender<PersistenceMsg>,
    admission: xai_grok_tools::types::restart_recovery::RestartAdmission,
) -> Result<(), acp::Error> {
    let (respond_to, response) = oneshot::channel();
    persistence.send(PersistenceMsg::SetRestartAdmission { admission, respond_to })
        .map_err(|_| restart_owner_closed())?;
    response.await.map_err(|_| restart_owner_closed())?
        .map_err(|error| acp::Error::internal_error().data(error.to_string()))
}

async fn completion_state(
    persistence: &mpsc::UnboundedSender<PersistenceMsg>,
) -> Result<crate::session::completion_delivery::CompletionDeliveryState, acp::Error> {
    let (respond_to, response) = oneshot::channel();
    persistence.send(PersistenceMsg::GetCompletionState { respond_to })
        .map_err(|_| restart_owner_closed())?;
    response.await.map_err(|_| restart_owner_closed())?
        .map_err(|error| acp::Error::internal_error().data(error.to_string()))
}

fn restart_owner_closed() -> acp::Error {
    acp::Error::internal_error().data("restart recovery persistence acknowledgement lost")
}
