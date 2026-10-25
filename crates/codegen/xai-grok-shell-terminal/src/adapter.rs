//! Implements `xai-grok-tools::TerminalBackend` over ACP gateway calls, for bash execution when the client serves the terminal.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::exit_watcher::{poll_for_terminal_exit, release_terminal, watch_for_exit};
use super::output_recorder::{OutputRecorder, read_log_tail};
use agent_client_protocol as acp;
use xai_acp_lib::AcpAgentGatewaySender as GatewaySender;
use xai_grok_tools::computer::types::{
    BackgroundHandle, BackgroundReason, ComputerError, KillOutcome, KillSource, TaskKind, TaskSnapshot,
    KillStage, KillProcessEffect, KillFailureReason, RunDisposition,
    TerminalBackend, TerminalRunRequest, TerminalRunResult,
};

/// A snapshot's per-completion fields, grouped to avoid transposed positional args.
#[derive(Clone)]
pub(super) struct SnapshotOutput {
    pub(super) output: String,
    pub(super) truncated: bool,
    pub(super) exit_code: Option<i32>,
    pub(super) signal: Option<String>,
}

pub(super) enum EnrollmentAttempt {
    InFlight,
    RetryAt(tokio::time::Instant),
}

pub(super) enum TerminalEnrollment {
    Running,
    Pending { snapshot: TaskSnapshot, attempt: EnrollmentAttempt },
    Enrolled,
}

#[derive(Clone, Copy)]
pub(super) enum EnrollmentSettlement {
    Acknowledged,
    Uncertain,
}

#[derive(Clone, Copy)]
enum BackgroundAnnouncement {
    Caller,
    SendNow,
}

#[derive(Clone, Copy)]
enum StopSelection {
    Foreground,
    Background,
}

enum ExecutionMode {
    Foreground {
        tool_call_id: String,
        demote: tokio::sync::oneshot::Sender<TerminalRunResult>,
    },
    ForegroundFinishing,
    Background,
}

struct KillReply {
    reply: tokio::sync::oneshot::Sender<KillOutcome>,
    source: KillSource,
    outcome: KillOutcome,
    deadline: tokio::time::Instant,
}

pub(super) struct TrackedTask {
    mode: ExecutionMode,
    kill_replies: Vec<KillReply>,
    pub(super) enrollment: TerminalEnrollment,
    pub(super) enrollment_changed: Arc<tokio::sync::Notify>,
    pub(super) waiters_changed: Arc<tokio::sync::Notify>,
    notification_handle: xai_grok_tools::notification::types::ToolNotificationHandle,
    command: String,
    display_command: Option<String>,
    cwd: String,
    /// ACP resource binding; task identities never address client resources directly.
    terminal_id: acp::TerminalId,
    output_file: PathBuf,
    start_time: std::time::SystemTime,
    /// Stamped once when the task completes, so repeated snapshots agree.
    end_time: Option<std::time::SystemTime>,
    completed: bool,
    exit_code: Option<i32>,
    signal: Option<String>,
    last_output: String,
    last_truncated: bool,
    block_waited: bool,
    explicitly_killed: bool,
    kill_result_delivered: bool,
    /// In-flight observers, not evidence of result consumption.
    pub(super) live_waiters: usize,
    kind: TaskKind,
    owner_session_id: Option<String>,
    description: Option<String>,
    output_byte_limit: usize,
}

/// Hand-written because `SystemTime` has no `Default`; call sites build tasks with `..Default::default()`.
impl Default for TrackedTask {
    fn default() -> Self {
        Self {
            mode: ExecutionMode::Background,
            kill_replies: Vec::new(),
            enrollment: TerminalEnrollment::Running,
            enrollment_changed: Arc::new(tokio::sync::Notify::new()),
            waiters_changed: Arc::new(tokio::sync::Notify::new()),
            notification_handle: xai_grok_tools::notification::types::ToolNotificationHandle::noop(),
            command: String::new(),
            display_command: None,
            cwd: String::new(),
            terminal_id: acp::TerminalId::new(""),
            output_file: PathBuf::new(),
            start_time: std::time::SystemTime::now(),
            end_time: None,
            completed: false,
            exit_code: None,
            signal: None,
            last_output: String::new(),
            last_truncated: false,
            block_waited: false,
            explicitly_killed: false,
            kill_result_delivered: false,
            live_waiters: 0,
            kind: TaskKind::Bash,
            owner_session_id: None,
            description: None,
            output_byte_limit: crate::DEFAULT_OUTPUT_BYTE_LIMIT,
        }
    }
}

impl TrackedTask {
    pub(super) fn settle_kills(&mut self, settlement: EnrollmentSettlement) {
        for pending in self.kill_replies.drain(..) {
            let acknowledged = matches!(settlement, EnrollmentSettlement::Acknowledged)
                && tokio::time::Instant::now() < pending.deadline;
            let outcome = if acknowledged {
                pending.outcome
            } else {
                KillOutcome::Indeterminate {
                    stage: KillStage::DurableEnrollment,
                    process_effect: KillProcessEffect::TerminalObserved,
                }
            };
            if pending.reply.send(outcome).is_ok() && acknowledged {
                self.kill_result_delivered |= pending.source.marks_result_delivered(false);
            }
        }
    }

    pub(super) fn mark_completed(&mut self, out: SnapshotOutput) {
        self.end_time = match &self.enrollment {
            TerminalEnrollment::Pending { snapshot, .. } => snapshot.end_time,
            TerminalEnrollment::Running | TerminalEnrollment::Enrolled => Some(std::time::SystemTime::now()),
        };
        self.enrollment = TerminalEnrollment::Enrolled;
        self.completed = true;
        self.exit_code = out.exit_code;
        self.signal = out.signal;
        self.last_output = out.output;
        self.last_truncated = out.truncated;
    }

    pub(super) fn terminal_snapshot(&self, task_id: &str) -> TaskSnapshot {
        self.to_snapshot(task_id, SnapshotOutput {
            output: self.last_output.clone(), truncated: self.last_truncated,
            exit_code: self.exit_code, signal: self.signal.clone(),
        })
    }

    fn resource_snapshot(resource_id: &str, out: SnapshotOutput) -> TaskSnapshot {
        let observed_at = std::time::SystemTime::now();
        let completed = out.exit_code.is_some() || out.signal.is_some();
        TaskSnapshot {
            task_id: resource_id.to_string(), command: String::new(), display_command: None,
            cwd: String::new(), start_time: observed_at, end_time: completed.then_some(observed_at),
            output: out.output, output_file: PathBuf::new(), truncated: out.truncated,
            output_total_bytes: 0, exit_code: out.exit_code, signal: out.signal, completed,
            kind: TaskKind::Bash, block_waited: false, explicitly_killed: false,
            kill_result_delivered: false, owner_session_id: None, description: None,
            is_backgrounded: false,
        }
    }

    pub(super) fn to_snapshot(&self, task_id: &str, out: SnapshotOutput) -> TaskSnapshot {
        let completed = self.completed || out.exit_code.is_some() || out.signal.is_some();
        TaskSnapshot {
            task_id: task_id.to_string(),
            command: self.command.clone(),
            display_command: self.display_command.clone(),
            cwd: self.cwd.clone(),
            start_time: self.start_time,
            end_time: self
                .end_time
                .or_else(|| completed.then(std::time::SystemTime::now)),
            output: out.output,
            output_file: self.output_file.clone(),
            truncated: out.truncated,
            exit_code: out.exit_code,
            signal: out.signal,
            completed,
            block_waited: self.block_waited,
            explicitly_killed: self.explicitly_killed,
            kill_result_delivered: self.kill_result_delivered,
            kind: self.kind,
            owner_session_id: self.owner_session_id.clone(),
            description: self.description.clone(),
            is_backgrounded: matches!(self.mode, ExecutionMode::Background),
            output_total_bytes: 0,
        }
    }
}

pub(super) type TaskMap = Arc<Mutex<HashMap<String, TrackedTask>>>;

#[must_use]
struct LiveWaiter<'a> {
    tasks: TaskMap,
    task_id: &'a str,
}

impl Drop for LiveWaiter<'_> {
    fn drop(&mut self) {
        if let Ok(mut tasks) = self.tasks.lock()
            && let Some(task) = tasks.get_mut(self.task_id)
        {
            task.live_waiters = task.live_waiters.saturating_sub(1);
            task.waiters_changed.notify_waiters();
        }
    }
}

fn wrap_command(command: &str) -> Result<String, ComputerError> {
    #[cfg(not(unix))]
    {
        let _ = command;
        Ok(command.to_string())
    }

    #[cfg(unix)]
    {
        let quoted = shlex::try_quote(command).map_err(|_| ComputerError::CommandNotQuoted)?;
        Ok(format!("{} -lc {quoted}", crate::default_shell_path()))
    }
}

fn to_env(env: HashMap<String, String>) -> Vec<acp::EnvVariable> {
    env.into_iter()
        .map(|(name, value)| acp::EnvVariable::new(name, value))
        .collect()
}

pub(super) fn parse_exit(
    status: &Option<acp::TerminalExitStatus>,
) -> (Option<i32>, Option<String>) {
    match status {
        Some(e) => (e.exit_code.map(|v| v as i32), e.signal.clone()),
        None => (None, None),
    }
}

/// Wraps xai-grok-shell's ACP gateway to satisfy xai-grok-tools' TerminalBackend.
#[derive(Clone)]
pub struct AcpTerminalAdapter {
    gateway: GatewaySender,
    session_id: acp::SessionId,
    tasks: TaskMap,
}

impl AcpTerminalAdapter {
    pub fn new(gateway: GatewaySender, session_id: acp::SessionId) -> Self {
        Self {
            gateway,
            session_id,
            tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn create_terminal(
        &self,
        command: String,
        request: &TerminalRunRequest,
    ) -> Result<acp::CreateTerminalResponse, ComputerError> {
        self.gateway
            .send(
                acp::CreateTerminalRequest::new(self.session_id.clone(), command)
                    .args(vec![])
                    .env(to_env(request.env.clone()))
                    .cwd(Some(request.working_directory.clone()))
                    .output_byte_limit(Some(request.output_byte_limit as u64)),
            )
            .await
            .map_err(|e| ComputerError::io(e.to_string()))
    }

    /// The client terminal id for `task_id`, read from the tracked task when it has one.
    fn terminal_id(&self, task_id: &str) -> acp::TerminalId {
        self.tasks
            .lock()
            .unwrap()
            .get(task_id)
            .map(|task| task.terminal_id.clone())
            .unwrap_or_else(|| acp::TerminalId::new(task_id))
    }

    /// How long `run` waits before it moves a still-running command to the background.
    /// This is the shorter of the request's timeout and its block budget (unset means the shared 15s default).
    fn auto_background_wait(request: &TerminalRunRequest) -> Duration {
        let budget = request.foreground_block_budget.unwrap_or_else(
            xai_grok_tools::computer::local::terminal::foreground_block_budget_from_env,
        );
        request.timeout.min(budget)
    }

    /// Keeps the registered execution identity when the foreground budget expires.
    async fn auto_background(
        &self,
        request: TerminalRunRequest,
        terminal_id: acp::TerminalId,
        task_id: String,
    ) -> TerminalRunResult {
        if let Some(task) = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_mut(&task_id) {
            task.mode = ExecutionMode::Background;
        }
        // Bounded: the block has already elapsed, so a stalled client must not hold the turn.
        let partial = match tokio::time::timeout(
            TERMINAL_OUTPUT_RPC_BUDGET,
            self.gateway.send(acp::TerminalOutputRequest::new(
                self.session_id.clone(),
                terminal_id.clone(),
            )),
        )
        .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(_)) | Err(_) => acp::TerminalOutputResponse::new(String::new(), false),
        };

        // The wait can lose the race with exit; report the completion instead of tracking a finished command.
        if partial.exit_status.is_some() {
            let (exit_code, signal) = parse_exit(&partial.exit_status);
            let mut recorder =
                OutputRecorder::new(request.output_file.clone(), request.output_byte_limit);
            recorder.initialize().await;
            if let Err(e) = recorder.append(&partial.output).await {
                tracing::warn!(task_id, error = %e, "output recorder failed to write foreground output");
            }
            self.finish_foreground(&task_id, &terminal_id, &request.notification_handle, SnapshotOutput {
                output: partial.output.clone(), truncated: partial.truncated,
                exit_code, signal: signal.clone(),
            }).await;
            let total_bytes = partial.output.len();
            return TerminalRunResult {
                disposition: RunDisposition::Foreground,
                combined_output: partial.output,
                exit_code,
                truncated: partial.truncated,
                signal,
                timed_out: false,
                output_file: request.output_file,
                total_bytes,
                pid: None,
            };
        }

        let mut recorder =
            OutputRecorder::new(request.output_file.clone(), request.output_byte_limit);
        recorder.initialize().await;
        if let Err(e) = recorder.append(&partial.output).await {
            tracing::warn!(task_id, error = %e, "output recorder failed to write pre-background output");
        }
        tokio::spawn(watch_for_exit(
            self.gateway.clone(),
            self.session_id.clone(),
            task_id.clone(),
            terminal_id,
            Arc::clone(&self.tasks),
            request.notification_handle.clone(),
            recorder,
        ));

        let total_bytes = partial.output.len();
        TerminalRunResult {
            disposition: RunDisposition::Backgrounded {
                handle: BackgroundHandle { task_id, output_file: request.output_file.clone(), pid: None },
                reason: BackgroundReason::ForegroundBudget,
            },
            combined_output: partial.output,
            exit_code: None,
            truncated: partial.truncated,
            signal: None,
            timed_out: false,
            output_file: request.output_file,
            total_bytes,
            pid: None,
        }
    }

    fn demote_foreground(&self, task_id: &str, announcement_owner: BackgroundAnnouncement) -> Option<xai_grok_tools::computer::types::BackgroundedForeground> {
        let (demote, tool_call_id, terminal_id, notification, recorder, result, announcement) = {
            let mut tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let task = tasks.get_mut(task_id)?;
            let terminal_id = task.terminal_id.clone();
            if !matches!(task.mode, ExecutionMode::Foreground { .. }) { return None; }
            let ExecutionMode::Foreground { tool_call_id, demote } = std::mem::replace(&mut task.mode, ExecutionMode::Background) else { return None; };
            let result = TerminalRunResult {
                disposition: RunDisposition::Backgrounded {
                    handle: BackgroundHandle { task_id: task_id.to_string(), output_file: task.output_file.clone(), pid: None },
                    reason: BackgroundReason::UserRedirect,
                },
                combined_output: task.last_output.clone(), exit_code: None, truncated: task.last_truncated,
                signal: None, timed_out: false, output_file: task.output_file.clone(),
                total_bytes: task.last_output.len(), pid: None,
            };
            let announcement = xai_grok_tools::notification::BashExecutionBackgrounded {
                base: xai_grok_tools::notification::BashNotificationBase {
                    tool_call_id: tool_call_id.clone(),
                    command: task.display_command.clone().unwrap_or_else(|| task.command.clone()),
                    output: Vec::new(), total_bytes: 0, truncated: false, cwd: PathBuf::from(&task.cwd),
                },
                output_file: task.output_file.clone(), task_id: task_id.to_string(),
                monitor_description: None, description: task.description.clone(),
            };
            (demote, tool_call_id, terminal_id, task.notification_handle.clone(),
                OutputRecorder::new(task.output_file.clone(), task.output_byte_limit), result, announcement)
        };
        let gateway = self.gateway.clone();
        let session_id = self.session_id.clone();
        let tasks = self.tasks.clone();
        let execution = task_id.to_string();
        match announcement_owner {
            BackgroundAnnouncement::Caller => {}
            BackgroundAnnouncement::SendNow => notification.send_backgrounded(announcement),
        }
        tokio::spawn(async move {
            recorder.initialize().await;
            watch_for_exit(gateway, session_id, execution, terminal_id, tasks, notification, recorder).await;
        });
        let _ = demote.send(result);
        Some(xai_grok_tools::computer::types::BackgroundedForeground { tool_call_id, task_id: task_id.to_string() })
    }

    async fn stop_executions(&self, selection: StopSelection, owner: Option<&str>) {
        let ids: Vec<String> = {
            let tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            tasks.iter().filter(|(_, task)| {
                let selected = match selection {
                    StopSelection::Foreground => matches!(task.mode,
                        ExecutionMode::Foreground { .. } | ExecutionMode::ForegroundFinishing),
                    StopSelection::Background => matches!(task.mode, ExecutionMode::Background),
                };
                selected && !task.completed
                    && owner.is_none_or(|owner| task.owner_session_id.as_deref() == Some(owner))
            }).map(|(id, _)| id.clone()).collect()
        };
        let _ = futures::future::join_all(ids.iter().map(|id| {
            self.kill_task_with_source(id, KillSource::Teardown)
        })).await;
    }

    fn retain_execution(&self, task_id: &str, terminal_id: acp::TerminalId, request: &TerminalRunRequest) {
        {
            let mut tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(task) = tasks.get_mut(task_id) else { return; };
            if !matches!(task.mode, ExecutionMode::ForegroundFinishing) { return; }
            task.mode = ExecutionMode::Background;
        }
        let gateway = self.gateway.clone();
        let session_id = self.session_id.clone();
        let tasks = self.tasks.clone();
        let task_id = task_id.to_string();
        let notifications = request.notification_handle.clone();
        let recorder = OutputRecorder::new(request.output_file.clone(), request.output_byte_limit);
        tokio::spawn(async move {
            recorder.initialize().await;
            watch_for_exit(gateway, session_id, task_id, terminal_id, tasks, notifications, recorder).await;
        });
    }

    async fn finish_foreground(
        &self,
        task_id: &str,
        terminal_id: &acp::TerminalId,
        notifications: &xai_grok_tools::notification::ToolNotificationHandle,
        output: SnapshotOutput,
    ) {
        let enroll = {
            let mut tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let enroll = tasks.get(task_id).is_some_and(|task| {
                task.explicitly_killed || !task.kill_replies.is_empty()
            });
            if !enroll { tasks.remove(task_id); }
            enroll
        };
        if enroll {
            super::exit_watcher::complete_and_release(
                &self.gateway, &self.session_id, terminal_id, &self.tasks,
                notifications, task_id, output,
            ).await;
        } else {
            release_terminal(&self.gateway, &self.session_id, terminal_id).await;
        }
    }

    async fn await_enrollment(&self, task_id: &str, deadline: tokio::time::Instant) -> bool {
        let changed = {
            let tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(task) = tasks.get(task_id) else { return false; };
            task.enrollment_changed.clone()
        };
        let notified = changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(task_id).is_some_and(|task| task.completed) {
            return true;
        }
        let _ = tokio::time::timeout_at(deadline, notified).await;
        self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(task_id).is_some_and(|task| task.completed)
    }

    fn observe_wait_result(&self, snapshot: Option<TaskSnapshot>) -> Option<TaskSnapshot> {
        snapshot.map(|mut snapshot| {
            if snapshot.completed {
                let mut tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(task) = tasks.get_mut(&snapshot.task_id) {
                    task.block_waited = true;
                    task.kill_result_delivered |= snapshot.explicitly_killed;
                    snapshot.block_waited = true;
                    snapshot.kill_result_delivered = task.kill_result_delivered;
                }
            }
            snapshot
        })
    }

    fn local_snapshot(&self, task_id: &str) -> Option<TaskSnapshot> {
        let tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let tracked = tasks.get(task_id)?;
        Some(tracked.to_snapshot(
            task_id,
            SnapshotOutput {
                output: tracked.last_output.clone(),
                truncated: tracked.last_truncated,
                exit_code: tracked.exit_code,
                signal: tracked.signal.clone(),
            },
        ))
    }
}

/// Per-task budget for live `terminal/output` during `list_tasks` (task/list).
/// After this expires that id falls back to local metadata; remaining ids still
/// get their own live attempt with a fresh budget.
pub(super) const TERMINAL_OUTPUT_RPC_BUDGET: Duration = Duration::from_secs(2);

impl AcpTerminalAdapter {
    async fn run_owned(&self, request: TerminalRunRequest) -> Result<TerminalRunResult, ComputerError> {
        let command = wrap_command(&request.command)?;
        let started_at = std::time::SystemTime::now();
        let task_id = uuid::Uuid::now_v7().to_string();
        let create_res = self.create_terminal(command, &request).await?;
        let (demote, mut demoted) = tokio::sync::oneshot::channel();
        self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(task_id.clone(), TrackedTask {
            mode: ExecutionMode::Foreground { tool_call_id: request.tool_call_id.clone(), demote },
            notification_handle: request.notification_handle.clone(), command: request.command.clone(),
            display_command: request.display_command.clone(),
            cwd: request.working_directory.to_string_lossy().into_owned(),
            terminal_id: create_res.terminal_id.clone(), output_file: request.output_file.clone(),
            start_time: started_at, kind: request.kind, owner_session_id: request.owner_session_id.clone(),
            description: request.description.clone(), output_byte_limit: request.output_byte_limit,
            ..Default::default()
        });

        let wait = if request.auto_background_on_timeout {
            Self::auto_background_wait(&request)
        } else {
            request.timeout
        };
        let waited = tokio::select! {
            result = tokio::time::timeout(wait, self.gateway.send(acp::WaitForTerminalExitRequest::new(
                self.session_id.clone(), create_res.terminal_id.clone(),
            ))) => result,
            result = &mut demoted => return result.map_err(|error| ComputerError::io(error.to_string())),
        };
        let demoted_meanwhile = {
            let mut tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match tasks.get_mut(&task_id) {
                Some(task) => match task.mode {
                    ExecutionMode::Foreground { .. } => {
                        task.mode = ExecutionMode::ForegroundFinishing;
                        false
                    }
                    ExecutionMode::Background => true,
                    ExecutionMode::ForegroundFinishing => false,
                },
                None => false,
            }
        };
        if demoted_meanwhile {
            return demoted.await.map_err(|error| ComputerError::io(error.to_string()));
        }
        let (timed_out, observed_status) = match waited {
            Ok(Ok(response)) => (false, Some(response.exit_status)),
            Ok(Err(error)) => {
                self.retain_execution(&task_id, create_res.terminal_id, &request);
                return Err(ComputerError::io(error.to_string()));
            }
            Err(_) if request.auto_background_on_timeout => {
                return Ok(self.auto_background(request, create_res.terminal_id, task_id).await);
            }
            Err(_) => {
                let _ = tokio::time::timeout(TERMINAL_OUTPUT_RPC_BUDGET, self.gateway.send(
                    acp::KillTerminalRequest::new(self.session_id.clone(), create_res.terminal_id.clone()),
                )).await;
                (true, None)
            }
        };

        let mut output = match tokio::time::timeout(TERMINAL_OUTPUT_RPC_BUDGET, self.gateway.send(
            acp::TerminalOutputRequest::new(self.session_id.clone(), create_res.terminal_id.clone()),
        )).await {
            Ok(Ok(output)) => output,
            Ok(Err(_)) | Err(_) => acp::TerminalOutputResponse::new(String::new(), false),
        };
        if output.exit_status.is_none() {
            output.exit_status = observed_status;
        }
        if output.exit_status.is_none() {
            self.retain_execution(&task_id, create_res.terminal_id, &request);
            return Err(ComputerError::io("terminal exit has not been observed"));
        }

        let (exit_code, signal) = parse_exit(&output.exit_status);
        let total_bytes = output.output.len();

        let mut recorder =
            OutputRecorder::new(request.output_file.clone(), request.output_byte_limit);
        recorder.initialize().await;
        if let Err(e) = recorder.append(&output.output).await {
            tracing::warn!(error = %e, "output recorder failed to write foreground output");
        }

        self.finish_foreground(&task_id, &create_res.terminal_id, &request.notification_handle, SnapshotOutput {
            output: output.output.clone(), truncated: output.truncated,
            exit_code, signal: signal.clone(),
        }).await;

        Ok(TerminalRunResult {
            disposition: RunDisposition::Foreground,
            combined_output: output.output,
            exit_code,
            truncated: output.truncated,
            signal,
            timed_out,
            output_file: request.output_file,
            total_bytes,
            pid: None,
        })
    }

    async fn run_background_owned(
        &self,
        request: TerminalRunRequest,
    ) -> Result<BackgroundHandle, ComputerError> {
        let command = wrap_command(&request.command)?;
        let notification_handle = request.notification_handle.clone();
        let display_command = request.display_command.clone();
        let cwd = request.working_directory.to_string_lossy().to_string();
        let output_file = request.output_file.clone();

        let create_res = self.create_terminal(command.clone(), &request).await?;
        let task_id = uuid::Uuid::now_v7().to_string();
        let description = request.description;

        {
            let mut tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            tasks.insert(
                task_id.clone(),
                TrackedTask {
                    notification_handle: notification_handle.clone(),
                    terminal_id: create_res.terminal_id.clone(),
                    command,
                    display_command,
                    cwd,
                    output_file: output_file.clone(),
                    kind: request.kind,
                    owner_session_id: request.owner_session_id.clone(),
                    description,
                    output_byte_limit: request.output_byte_limit,
                    ..Default::default()
                },
            );
        }

        let recorder = OutputRecorder::new(output_file.clone(), request.output_byte_limit);
        recorder.initialize().await;
        tokio::spawn(watch_for_exit(
            self.gateway.clone(),
            self.session_id.clone(),
            task_id.clone(),
            create_res.terminal_id,
            Arc::clone(&self.tasks),
            notification_handle,
            recorder,
        ));

        Ok(BackgroundHandle {
            task_id,
            output_file,
            pid: None,
        })
    }

}

#[async_trait::async_trait]
impl TerminalBackend for AcpTerminalAdapter {
    async fn run(&self, request: TerminalRunRequest) -> Result<TerminalRunResult, ComputerError> {
        let owner = self.clone();
        let (reply, response) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = owner.run_owned(request).await;
            let _ = reply.send(result);
        });
        response.await.map_err(|error| ComputerError::io(error.to_string()))?
    }

    async fn run_background(&self, request: TerminalRunRequest) -> Result<BackgroundHandle, ComputerError> {
        let owner = self.clone();
        let (reply, response) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = owner.run_background_owned(request).await;
            let _ = reply.send(result);
        });
        response.await.map_err(|error| ComputerError::io(error.to_string()))?
    }

    async fn get_task(&self, task_id: &str) -> Option<TaskSnapshot> {
        let live = tokio::time::timeout(TERMINAL_OUTPUT_RPC_BUDGET, self.gateway
            .send(acp::TerminalOutputRequest::new(
                self.session_id.clone(),
                self.terminal_id(task_id),
            )))
            .await
            .ok()
            .and_then(Result::ok);

        // The std Mutex guard cannot be held across the await below, so resolve under the lock and read the log file after releasing it
        enum Resolved {
            Ready(TaskSnapshot),
            FromLog {
                snapshot: TaskSnapshot,
                output_file: PathBuf,
                limit: usize,
            },
            Missing,
        }
        let resolved = {
            let tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match (live, tasks.get(task_id)) {
                (Some(output), Some(tracked)) => {
                    let (exit_code, signal) = parse_exit(&output.exit_status);
                    Resolved::Ready(tracked.to_snapshot(
                        task_id,
                        SnapshotOutput {
                            output: output.output,
                            truncated: output.truncated,
                            exit_code,
                            signal,
                        },
                    ))
                }
                (Some(output), None) => {
                    let (exit_code, signal) = parse_exit(&output.exit_status);
                    Resolved::Ready(TrackedTask::resource_snapshot(
                        task_id,
                        SnapshotOutput {
                            output: output.output,
                            truncated: output.truncated,
                            exit_code,
                            signal,
                        },
                    ))
                }
                // Live poll failed: a completed task keeps its authoritative last_output; only a still-running task falls back to the log
                (None, Some(tracked)) => {
                    let snapshot = tracked.to_snapshot(
                        task_id,
                        SnapshotOutput {
                            output: tracked.last_output.clone(),
                            truncated: tracked.last_truncated,
                            exit_code: tracked.exit_code,
                            signal: tracked.signal.clone(),
                        },
                    );
                    if snapshot.completed {
                        Resolved::Ready(snapshot)
                    } else {
                        Resolved::FromLog {
                            snapshot,
                            output_file: tracked.output_file.clone(),
                            limit: tracked.output_byte_limit,
                        }
                    }
                }
                (None, None) => Resolved::Missing,
            }
        };

        match resolved {
            Resolved::Ready(snapshot) => {
                if snapshot.completed {
                    let owned = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner).contains_key(task_id);
                    if owned {
                        let deadline = tokio::time::Instant::now()
                            + xai_grok_tools::notification::handle::TERMINAL_ACK_BUDGET;
                        if !self.await_enrollment(task_id, deadline).await { return None; }
                        return self.local_snapshot(task_id);
                    }
                }
                Some(snapshot)
            },
            Resolved::Missing => None,
            Resolved::FromLog {
                mut snapshot,
                output_file,
                limit,
            } => {
                if let Some(tail) = read_log_tail(&output_file, limit).await {
                    snapshot.output = tail.text;
                    snapshot.truncated = tail.truncated;
                }
                Some(snapshot)
            }
        }
    }

    async fn kill_task(&self, task_id: &str) -> KillOutcome {
        self.kill_task_with_source(task_id, KillSource::ModelTool)
            .await
    }

    async fn kill_task_with_source(&self, task_id: &str, source: KillSource) -> KillOutcome {
        let deadline = tokio::time::Instant::now()
            + xai_grok_tools::notification::handle::TERMINAL_ACK_BUDGET;
        self.demote_foreground(task_id, BackgroundAnnouncement::Caller);
        let tracked = {
            let mut tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            tasks.get_mut(task_id).map(|task| {
                if matches!(task.enrollment, TerminalEnrollment::Running) && !task.completed {
                    task.explicitly_killed = true;
                }
                (task.completed, matches!(task.enrollment, TerminalEnrollment::Pending { .. }))
            })
        };
        let already_exited = tracked.is_some_and(|(completed, pending)| completed || pending);
        if tracked.is_none() {
            let probe = tokio::time::timeout_at(deadline, self.gateway.send(
                acp::TerminalOutputRequest::new(self.session_id.clone(), self.terminal_id(task_id)),
            )).await;
            match probe {
                Ok(Err(error)) if error.code == acp::Error::resource_not_found(None::<String>).code => return KillOutcome::NotFound,
                Ok(Err(error)) if error.code == acp::Error::method_not_found().code => return KillOutcome::Failed {
                    stage: KillStage::ProcessTermination, reason: KillFailureReason::Unsupported,
                },
                Ok(Err(_)) | Err(_) => {},
                Ok(Ok(output)) if output.exit_status.is_some() => return KillOutcome::AlreadyExited,
                Ok(Ok(_)) => {}
            }
        }
        if !already_exited {
            match tokio::time::timeout_at(deadline, self.gateway.send(
                acp::KillTerminalRequest::new(self.session_id.clone(), self.terminal_id(task_id)),
            )).await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) if error.code == acp::Error::resource_not_found(None::<String>).code && tracked.is_none() => return KillOutcome::NotFound,
                Ok(Err(error)) if error.code == acp::Error::method_not_found().code => return KillOutcome::Failed {
                    stage: KillStage::ProcessTermination, reason: KillFailureReason::Unsupported,
                },
                Ok(Err(_)) | Err(_) => return KillOutcome::Indeterminate {
                    stage: KillStage::ProcessTermination, process_effect: KillProcessEffect::Unknown,
                },
            }
        }
        if tracked.is_none() {
            // A resource probe has no execution ownership or durable enrollment authority.
            return KillOutcome::Indeterminate {
                stage: KillStage::ProcessTermination, process_effect: KillProcessEffect::StopRequested,
            };
        }
        let outcome = if already_exited { KillOutcome::AlreadyExited } else { KillOutcome::Killed };
        let (reply, receiver) = tokio::sync::oneshot::channel();
        {
            let mut tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(task) = tasks.get_mut(task_id) {
                if task.completed {
                    task.kill_result_delivered |= source.marks_result_delivered(false);
                    return outcome;
                }
                task.kill_replies.retain(|pending| !pending.reply.is_closed());
                task.kill_replies.push(KillReply { reply, source, outcome, deadline });
            }
        }
        match tokio::time::timeout_at(deadline, receiver).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) | Err(_) => {
                let tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let terminal_observed = tasks.get(task_id).is_some_and(|task| {
                    matches!(task.enrollment, TerminalEnrollment::Pending { .. } | TerminalEnrollment::Enrolled)
                });
                KillOutcome::Indeterminate {
                    stage: if terminal_observed { KillStage::DurableEnrollment } else { KillStage::ProcessTermination },
                    process_effect: if terminal_observed { KillProcessEffect::TerminalObserved } else { KillProcessEffect::StopRequested },
                }
            }
        }
    }

    async fn wait_for_completion(
        &self,
        task_id: &str,
        timeout: Option<Duration>,
    ) -> Option<TaskSnapshot> {
        let timeout = timeout.unwrap_or(Duration::from_secs(30));

        enum WaitStart {
            Immediate,
            Block,
            ProbeThenMaybeBlock,
        }
        let start = {
            let mut tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match tasks.get_mut(task_id) {
                Some(task) => {
                    if task.completed {
                        WaitStart::Immediate
                    } else {
                        task.live_waiters = task.live_waiters.saturating_add(1);
                        WaitStart::Block
                    }
                }
                None => WaitStart::ProbeThenMaybeBlock,
            }
        };
        match start {
            WaitStart::Immediate => return self.observe_wait_result(self.get_task(task_id).await),
            WaitStart::ProbeThenMaybeBlock => {
                // Not tracked as running: one output probe A dead or unknown terminal must not burn the wait budget on WaitForTerminalExit Match kill's untracked probe: `get_task` maps every TerminalOutput error to None
                // So a transport/channel blip on a still-live resumed terminal must not look like not-found Only a client that answered and disowned the id is a definitive miss
                let probe = self
                    .gateway
                    .send(acp::TerminalOutputRequest::new(
                        self.session_id.clone(),
                        self.terminal_id(task_id),
                    ))
                    .await;
                match probe {
                    Err(err) if err.code == acp::Error::resource_not_found(None::<String>).code => return None,
                    Err(_) => {}
                    Ok(output) if output.exit_status.is_some() => {
                        let (exit_code, signal) = parse_exit(&output.exit_status);
                        return Some(TrackedTask::resource_snapshot(
                            task_id,
                            SnapshotOutput {
                                output: output.output,
                                truncated: output.truncated,
                                exit_code,
                                signal,
                            },
                        ));
                    }
                    Ok(_) => {}
                }
            }
            WaitStart::Block => {
                let _waiter = LiveWaiter { tasks: self.tasks.clone(), task_id };
                let deadline = tokio::time::Instant::now() + timeout;
                if !self.await_enrollment(task_id, deadline).await {
                    return None;
                }
                return self.observe_wait_result(self.local_snapshot(task_id));
            }
        }

        let gateway_result = tokio::time::timeout(
            timeout,
            self.gateway.send(acp::WaitForTerminalExitRequest::new(
                self.session_id.clone(),
                self.terminal_id(task_id),
            )),
        )
        .await;

        match &gateway_result {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                let completed_meanwhile = {
                    let tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    tasks.get(task_id).is_some_and(|task| task.completed)
                };
                if !completed_meanwhile {
                    tracing::warn!(task_id, error = %e, "gateway error waiting for terminal exit, falling back to polling");
                    let deadline = tokio::time::Instant::now() + timeout;
                    poll_for_terminal_exit(
                        &self.gateway,
                        &self.session_id,
                        &self.terminal_id(task_id),
                        Some(deadline),
                    )
                    .await;
                }
            }
            Err(_) => {
                tracing::debug!(task_id, "timeout waiting for terminal exit");
            }
        }

        self.observe_wait_result(self.get_task(task_id).await)
    }

    async fn list_tasks(&self) -> Vec<TaskSnapshot> {
        // Preserve x.ai/task/list output for running rows via get_task (live terminal/output or log tail).
        // Bound each RPC so a hung client cannot stall enumeration indefinitely; only the timed-out id
        // falls back, and the remaining ids still get a live attempt with their own budget.
        let task_ids: Vec<String> = {
            let tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            tasks.keys().cloned().collect()
        };
        let mut snapshots = Vec::new();
        for task_id in task_ids {
            match tokio::time::timeout(TERMINAL_OUTPUT_RPC_BUDGET, self.get_task(&task_id)).await {
                Ok(Some(snapshot)) => snapshots.push(snapshot),
                Ok(None) => {}
                Err(_) => {
                    tracing::warn!(
                        task_id = %task_id,
                        budget_ms = TERMINAL_OUTPUT_RPC_BUDGET.as_millis() as u64,
                        "terminal/output RPC timed out; using local snapshot for this id only"
                    );
                    if let Some(local) = self.local_snapshot(&task_id) {
                        snapshots.push(local);
                    }
                }
            }
        }
        snapshots
    }

    async fn list_tasks_metadata(&self) -> Vec<TaskSnapshot> {
        // Snapshot path: one lock, no terminal/output RPCs, no log reads.
        let tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        tasks
            .iter()
            .map(|(task_id, tracked)| {
                tracked.to_snapshot(
                    task_id,
                    SnapshotOutput {
                        output: String::new(),
                        truncated: false,
                        exit_code: tracked.exit_code,
                        signal: tracked.signal.clone(),
                    },
                )
            })
            .collect()
    }

    async fn kill_all_background_tasks(&self) {
        self.stop_executions(StopSelection::Background, None).await;
    }

    async fn kill_all_background_tasks_by_owner(&self, owner_session_id: &str) {
        self.stop_executions(StopSelection::Background, Some(owner_session_id)).await;
    }

    async fn kill_foreground_commands_by_owner(&self, owner_session_id: &str) {
        self.stop_executions(StopSelection::Foreground, Some(owner_session_id)).await;
    }

    async fn background_foreground_command(&self, tool_call_id: &str) -> bool {
        let task_id = {
            let tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            tasks.iter().find_map(|(id, task)| match &task.mode {
                ExecutionMode::Foreground { tool_call_id: call, .. } if call == tool_call_id => Some(id.clone()),
                ExecutionMode::Foreground { .. }
                | ExecutionMode::ForegroundFinishing
                | ExecutionMode::Background => None,
            })
        };
        task_id.is_some_and(|id| self.demote_foreground(&id, BackgroundAnnouncement::Caller).is_some())
    }

    async fn background_foreground_commands(&self, owner_session_id: Option<&str>) -> Vec<xai_grok_tools::computer::types::BackgroundedForeground> {
        let ids: Vec<String> = {
            let tasks = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            tasks.iter().filter(|(_, task)| {
                matches!(task.mode, ExecutionMode::Foreground { .. })
                    && owner_session_id.is_none_or(|owner| task.owner_session_id.as_deref() == Some(owner))
            }).map(|(id, _)| id.clone()).collect()
        };
        ids.into_iter().filter_map(|id| self.demote_foreground(&id, BackgroundAnnouncement::SendNow)).collect()
    }

    async fn kill_foreground_commands(&self) {
        self.stop_executions(StopSelection::Foreground, None).await;
    }
}

#[cfg(test)]
#[path = "adapter_tests.rs"]
mod tests;

