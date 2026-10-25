//! Awaits `wait_for_exit` on an ACP background terminal while polling `terminal/output` into the [`OutputRecorder`].
//! On exit it completes the task and releases the terminal.

use std::time::Duration;

use agent_client_protocol as acp;
use xai_acp_lib::AcpAgentGatewaySender as GatewaySender;
use xai_grok_tools::notification::types::ToolNotificationHandle;

use super::adapter::{EnrollmentAttempt, EnrollmentSettlement, SnapshotOutput, TaskMap, TerminalEnrollment, parse_exit};
use super::output_recorder::OutputRecorder;

const RECORDER_POLL: Duration = Duration::from_millis(250);

const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(500);

const GATEWAY_LOST_AFTER: Duration = Duration::from_secs(30);

/// Same lifetime cap the local terminal applies to background tasks.
pub(super) const BACKGROUND_MAX_RUNTIME: Duration = Duration::from_secs(36_000);

fn max_poll_errors(cadence: Duration) -> u32 {
    (GATEWAY_LOST_AFTER.as_millis() / cadence.as_millis().max(1)).max(1) as u32
}

enum PollStep {
    Output(Box<acp::TerminalOutputResponse>),
    Retry,
    GaveUp,
}

async fn poll_terminal_output(
    gateway: &GatewaySender,
    session_id: &acp::SessionId,
    terminal_id: &acp::TerminalId,
    consecutive_errors: &mut u32,
    max_errors: u32,
) -> PollStep {
    match tokio::time::timeout(super::adapter::TERMINAL_OUTPUT_RPC_BUDGET, gateway
        .send(acp::TerminalOutputRequest::new(
            session_id.clone(),
            terminal_id.clone(),
        )))
        .await
    {
        Ok(Ok(output)) => {
            *consecutive_errors = 0;
            PollStep::Output(Box::new(output))
        }
        result @ (Ok(Err(_)) | Err(_)) => {
            *consecutive_errors += 1;
            if *consecutive_errors >= max_errors {
                tracing::error!(
                    terminal_id = %terminal_id.0,
                    error = ?result,
                    "gateway unreachable after consecutive poll failures"
                );
                PollStep::GaveUp
            } else {
                PollStep::Retry
            }
        }
    }
}

enum Exit {
    WithOutput(Box<acp::TerminalOutputResponse>),
    NeedFetch(acp::TerminalExitStatus),
}

/// `terminal_id` addresses the client resource; `task_id` identifies the execution.
pub(super) async fn watch_for_exit(
    gateway: GatewaySender,
    session_id: acp::SessionId,
    task_id: String,
    terminal_id: acp::TerminalId,
    tasks: TaskMap,
    notification_handle: ToolNotificationHandle,
    mut recorder: OutputRecorder,
) {
    let wait = gateway.send(acp::WaitForTerminalExitRequest::new(
        session_id.clone(),
        terminal_id.clone(),
    ));
    tokio::pin!(wait);
    let mut wait_pending = true;
    let mut consecutive_errors = 0u32;
    let poll_error_budget = max_poll_errors(RECORDER_POLL);
    let cap = tokio::time::sleep(BACKGROUND_MAX_RUNTIME);
    tokio::pin!(cap);
    let mut stop_requested = false;
    let exit = loop {
        tokio::select! {
            _ = &mut cap, if !stop_requested => {
                tracing::warn!(task_id, "background task exceeded max runtime, killing");
                let _ = tokio::time::timeout(
                    GATEWAY_LOST_AFTER,
                    gateway.send(acp::KillTerminalRequest::new(
                        session_id.clone(),
                        terminal_id.clone(),
                    )),
                )
                .await;
                stop_requested = true;
            }
            res = &mut wait, if wait_pending => match res {
                Ok(response) => break Exit::NeedFetch(response.exit_status),
                Err(e) => {
                    tracing::warn!(
                        task_id,
                        error = %e,
                        "watch_for_exit: gateway error waiting for terminal exit, polling until exit"
                    );
                    wait_pending = false;
                }
            },
            _ = tokio::time::sleep(RECORDER_POLL) => {
                match poll_terminal_output(
                    &gateway,
                    &session_id,
                    &terminal_id,
                    &mut consecutive_errors,
                    poll_error_budget,
                )
                .await
                {
                    PollStep::Output(output) => {
                        if let Err(e) = recorder.append(&output.output).await {
                            tracing::debug!(task_id, error = %e, "output recorder append failed; retrying next poll");
                        }
                        if output.exit_status.is_some() {
                            break Exit::WithOutput(output);
                        }
                    }
                    PollStep::Retry => {}
                    PollStep::GaveUp => {
                        consecutive_errors = 0;
                    }
                }
            }
        }
    };

    let output = match exit {
        Exit::WithOutput(output) => *output,
        Exit::NeedFetch(status) => match tokio::time::timeout(super::adapter::TERMINAL_OUTPUT_RPC_BUDGET, gateway
            .send(acp::TerminalOutputRequest::new(
                session_id.clone(),
                terminal_id.clone(),
            )))
            .await
        {
            Ok(Ok(mut output)) => {
                if output.exit_status.is_none() { output.exit_status = Some(status); }
                output
            }
            // The wait supplied terminal evidence even when the final output fetch failed.
            Ok(Err(_)) | Err(_) => acp::TerminalOutputResponse::new(recorder.mirrored().to_string(), false)
                .exit_status(Some(status)),
        },
    };

    let (exit_code, signal) = parse_exit(&output.exit_status);
    if let Err(e) = recorder.append(&output.output).await {
        tracing::warn!(task_id, error = %e, "output recorder failed to write final output");
    }
    complete_and_release(
        &gateway,
        &session_id,
        &terminal_id,
        &tasks,
        &notification_handle,
        &task_id,
        SnapshotOutput {
            output: output.output,
            truncated: output.truncated,
            exit_code,
            signal,
        },
    )
    .await;
}

/// Releases even when the task is gone, so the client terminal is not leaked.
pub(super) async fn complete_and_release(
    gateway: &GatewaySender,
    session_id: &acp::SessionId,
    terminal_id: &acp::TerminalId,
    tasks: &TaskMap,
    notification_handle: &ToolNotificationHandle,
    task_id: &str,
    out: SnapshotOutput,
) {
    let pending = {
        let mut guard = tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.get_mut(task_id).and_then(|task| {
            match &task.enrollment {
                TerminalEnrollment::Running => {
                    let mut snapshot = task.to_snapshot(task_id, out.clone());
                    snapshot.completed = true;
                    snapshot.end_time = Some(std::time::SystemTime::now());
                    task.enrollment = TerminalEnrollment::Pending {
                        snapshot: snapshot.clone(),
                        attempt: EnrollmentAttempt::RetryAt(tokio::time::Instant::now()),
                    };
                    Some(task.enrollment_changed.clone())
                }
                // The existing watcher alone owns the frozen fact and its retries.
                TerminalEnrollment::Pending { .. } | TerminalEnrollment::Enrolled => None,
            }
        })
    };
    let Some(changed) = pending else {
        let missing = !tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner).contains_key(task_id);
        if missing { release_terminal(gateway, session_id, terminal_id).await; }
        return;
    };
    enum EnrollmentCommand {
        Finished(Result<(), xai_grok_tools::notification::NotificationAcknowledgementError>),
    }
    let (commands, mut completions) = tokio::sync::mpsc::channel(1);
    loop {
        let retry_at = {
            let guard = tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match guard.get(task_id).map(|task| &task.enrollment) {
                Some(TerminalEnrollment::Pending { attempt: EnrollmentAttempt::RetryAt(at), .. }) => Some(*at),
                Some(TerminalEnrollment::Pending { attempt: EnrollmentAttempt::InFlight, .. }) => None,
                Some(TerminalEnrollment::Running | TerminalEnrollment::Enrolled) | None => return,
            }
        };
        tokio::select! {
            command = completions.recv() => {
                let Some(EnrollmentCommand::Finished(result)) = command else { return; };
                match result {
                    Ok(()) => break,
                    Err(error) => {
                        tracing::error!(task_id, %error, "terminal retained until durable completion enrollment");
                        let mut guard = tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        if let Some(task) = guard.get_mut(task_id) {
                            task.settle_kills(EnrollmentSettlement::Uncertain);
                            if let TerminalEnrollment::Pending { attempt, .. } = &mut task.enrollment {
                                *attempt = EnrollmentAttempt::RetryAt(tokio::time::Instant::now() + Duration::from_secs(1));
                            }
                        }
                        changed.notify_waiters();
                    }
                }
            }
            _ = async {
                match retry_at {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                let frozen = {
                    let mut guard = tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    guard.get_mut(task_id).and_then(|task| match &mut task.enrollment {
                        TerminalEnrollment::Pending { snapshot, attempt } => {
                            *attempt = EnrollmentAttempt::InFlight;
                            Some(snapshot.clone())
                        }
                        TerminalEnrollment::Running | TerminalEnrollment::Enrolled => None,
                    })
                };
                let Some(frozen) = frozen else { return; };
                let deadline = tokio::time::Instant::now()
                    + xai_grok_tools::notification::handle::TERMINAL_ACK_BUDGET;
                let acknowledgement = notification_handle.enroll_task_completion(frozen);
                let commands = commands.clone();
                tokio::spawn(async move {
                    let result = acknowledgement.wait_until(deadline).await;
                    let _ = commands.send(EnrollmentCommand::Finished(result)).await;
                });
            }
        }
    }
    {
        let mut guard = tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(task) = guard.get_mut(task_id) {
            task.mark_completed(out);
            task.settle_kills(EnrollmentSettlement::Acknowledged);
        }
    }
    changed.notify_waiters();
    let waiter_deadline = tokio::time::Instant::now()
        + xai_grok_tools::notification::handle::TERMINAL_ACK_BUDGET;
    loop {
        let waiter_change = {
            let guard = tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.get(task_id).map(|task| task.waiters_changed.clone())
        };
        let Some(waiter_change) = waiter_change else { break; };
        let notified = waiter_change.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let pending = tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(task_id).is_some_and(|task| task.live_waiters > 0);
        if !pending || tokio::time::timeout_at(waiter_deadline, notified).await.is_err() {
            break;
        }
    }
    let publication = {
        let guard = tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.get(task_id).map(|task| task.terminal_snapshot(task_id))
    };
    release_terminal(gateway, session_id, terminal_id).await;
    if let Some(publication) = publication { notification_handle.send_task_complete(publication); }
}

pub(super) async fn release_terminal(
    gateway: &GatewaySender,
    session_id: &acp::SessionId,
    terminal_id: &acp::TerminalId,
) {
    match tokio::time::timeout(super::adapter::TERMINAL_OUTPUT_RPC_BUDGET, gateway
        .send(acp::ReleaseTerminalRequest::new(
            session_id.clone(),
            terminal_id.clone(),
        )))
        .await
    {
        Ok(Ok(_)) => {}
        result @ (Ok(Err(_)) | Err(_)) => {
            tracing::debug!(terminal_id = %terminal_id.0, error = ?result, "release_terminal failed");
        }
    }
}

/// Fallback exit detector for the blocking `wait_for_completion` path.
/// Unlike [`watch_for_exit`] it only detects exit and does not mirror output.
/// Returns `true` on exit, `false` on deadline or [`GATEWAY_LOST_AFTER`] of failures.
pub(super) async fn poll_for_terminal_exit(
    gateway: &GatewaySender,
    session_id: &acp::SessionId,
    terminal_id: &acp::TerminalId,
    deadline: Option<tokio::time::Instant>,
) -> bool {
    let mut consecutive_errors = 0u32;
    loop {
        if let Some(dl) = deadline
            && tokio::time::Instant::now() >= dl
        {
            return false;
        }
        tokio::time::sleep(EXIT_POLL_INTERVAL).await;
        match poll_terminal_output(
            gateway,
            session_id,
            terminal_id,
            &mut consecutive_errors,
            max_poll_errors(EXIT_POLL_INTERVAL),
        )
        .await
        {
            PollStep::Output(output) => {
                if output.exit_status.is_some() {
                    return true;
                }
            }
            PollStep::Retry => {}
            PollStep::GaveUp => return false,
        }
    }
}
