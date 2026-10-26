//! The turn-end `Stop`/`SubagentStop` gate for `SessionActor`.

use super::*;
use xai_grok_hooks::event::{
    self, BackgroundTaskType, StopBackgroundTask, StopSessionCron, clip_stop_entry_text,
};
use xai_grok_hooks::{dispatcher, result};

pub const MAX_STOP_HOOK_CONTINUATIONS_PER_TURN: u32 = 8;

const SESSION_END_STOP_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

pub(super) const DECLARE_WAIT_TOOL: &str = "DeclareWait";
const TURN_WAIT_CONTRACT: &str = "When you need to wait, call DeclareWait alone \
with the chosen live owned task, monitor, subagent, or workflow ids. This ends your turn \
and suppresses goal continuation until any selected target completes or new input \
arrives. Unselected registry changes alone do not satisfy the wait; any admitted input breaks it. Ending a turn without declaring \
a wait keeps normal goal continuation. /goal pause pauses autonomous goal continuation, \
not background work or completion delivery. A user stop parks completion wakes until \
human input or /goal resume.";

/// A background target explicitly selected by the model.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum WaitOnTarget {
    BashTask { task_id: String },
    Monitor { task_id: String },
    Subagent { subagent_id: String },
    WorkflowRun { run_id: String },
}

/// The explicitly declared subset carried through the stop gate.
/// Any selected completion or new input releases the wait.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TurnWaitDeclaration {
    targets: Vec<WaitOnTarget>,
}

#[derive(Default)]
pub(crate) struct TurnWaitState {
    input_epoch: u64,
    pub(super) declaration: TurnWaitDeclaration,
}

impl TurnWaitDeclaration {
    pub(crate) fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    pub(crate) fn contains(&self, target: &WaitOnTarget) -> bool {
        self.targets.contains(target)
    }

    /// Declare a wait over exactly the given targets.
    pub(crate) fn of(targets: impl IntoIterator<Item = WaitOnTarget>) -> Self {
        Self {
            targets: targets.into_iter().collect(),
        }
    }

    fn push(&mut self, target: WaitOnTarget) {
        self.targets.push(target);
    }
}

/// The task-kind half of a [`WaitOnTarget`]: outstanding bash and monitor tasks come
/// from the same background-task registry, so both map here.
fn task_wait_target(
    kind: xai_grok_tools::computer::types::TaskKind,
    task_id: &str,
) -> WaitOnTarget {
    match kind {
        xai_grok_tools::computer::types::TaskKind::Bash => WaitOnTarget::BashTask {
            task_id: task_id.to_owned(),
        },
        xai_grok_tools::computer::types::TaskKind::Monitor => WaitOnTarget::Monitor {
            task_id: task_id.to_owned(),
        },
    }
}

fn commit_stop_report(claim: TurnReportClaim<'_>, prompt_id: &str) {
    if claim.commit() == CommitOutcome::LostToAnotherReporter {
        tracing::debug!(prompt_id, "a cancel already reported this turn");
    }
}

/// `command` is a shell-only field, so a monitor's watch command is carried in `description` instead.
fn stop_entry_from_task(task: &xai_grok_tools::types::TaskSnapshot) -> StopBackgroundTask {
    let command_text =
        clip_stop_entry_text(task.display_command.as_deref().unwrap_or(&task.command));
    let (kind, command, description) = match task.kind {
        xai_grok_tools::computer::types::TaskKind::Bash => {
            (BackgroundTaskType::Shell, Some(command_text), None)
        }
        xai_grok_tools::computer::types::TaskKind::Monitor => {
            (BackgroundTaskType::Monitor, None, Some(command_text))
        }
    };
    StopBackgroundTask {
        id: task.task_id.clone(),
        r#type: kind,
        status: "running".to_string(),
        description,
        command,
        agent_type: None,
    }
}

fn stop_entry_from_subagent(
    summary: &xai_grok_tools::implementations::grok_build::task::types::ActiveSubagentSummary,
) -> StopBackgroundTask {
    StopBackgroundTask {
        id: summary.subagent_id.clone(),
        r#type: BackgroundTaskType::Subagent,
        status: "running".to_string(),
        description: Some(clip_stop_entry_text(&summary.description)),
        command: None,
        agent_type: Some(summary.subagent_type.clone()),
    }
}

fn stop_cron_from_scheduled(
    task: &xai_grok_tools::implementations::grok_build::scheduler::types::ScheduledTask,
) -> StopSessionCron {
    StopSessionCron {
        id: task.id.clone(),
        schedule:
            xai_grok_tools::implementations::grok_build::scheduler::interval::interval_to_human(
                task.interval_secs,
            ),
        recurring: task.recurring,
        prompt: clip_stop_entry_text(&task.prompt),
    }
}

/// Downgrade `Blocked` to `Success` for the observe-only session-end fire.
/// The decision is discarded, so scrollback and telemetry must not report a block.
pub(super) fn demote_ignored_blocks(
    results: Vec<result::HookRunResult>,
) -> Vec<result::HookRunResult> {
    use xai_grok_hooks::result::HookRunResult;
    results
        .into_iter()
        .map(|result| match result {
            HookRunResult::Blocked {
                hook_name,
                elapsed,
                http_info,
                system_message,
                ..
            } => HookRunResult::Success {
                hook_name,
                elapsed,
                http_info,
                system_message,
            },
            other => other,
        })
        .collect()
}

pub(super) enum WaitToolStep {
    Proceed,
    Retry,
    Declared,
}

pub(super) fn wait_tool_spec() -> ToolSpec {
    ToolSpec {
        name: DECLARE_WAIT_TOOL.to_string(),
        description: Some(TURN_WAIT_CONTRACT.to_string()),
        parameters: serde_json::json!({
            "type": "object", "additionalProperties": false, "required": ["targets"],
            "properties": { "targets": { "type": "array", "minItems": 1, "items": {
                "oneOf": [
                    { "type": "object", "additionalProperties": false, "required": ["kind", "task_id"], "properties": { "kind": { "const": "bash_task" }, "task_id": { "type": "string", "minLength": 1 } } },
                    { "type": "object", "additionalProperties": false, "required": ["kind", "task_id"], "properties": { "kind": { "const": "monitor" }, "task_id": { "type": "string", "minLength": 1 } } },
                    { "type": "object", "additionalProperties": false, "required": ["kind", "subagent_id"], "properties": { "kind": { "const": "subagent" }, "subagent_id": { "type": "string", "minLength": 1 } } },
                    { "type": "object", "additionalProperties": false, "required": ["kind", "run_id"], "properties": { "kind": { "const": "workflow_run" }, "run_id": { "type": "string", "minLength": 1 } } }
                ]
            } } }
        }),
    }
}

impl SessionActor {
    pub(super) async fn handle_wait_tool_call(
        &self,
        calls: &mut Vec<xai_grok_sampling_types::conversation::ToolCall>,
    ) -> WaitToolStep {
        if !calls.iter().any(|call| call.name == DECLARE_WAIT_TOOL) {
            return WaitToolStep::Proceed;
        }
        if calls.len() != 1 {
            for call in calls.iter().filter(|call| call.name == DECLARE_WAIT_TOOL) {
                self.chat_state_handle.push_tool_result(ConversationItem::tool_result(
                    call.id.as_ref().to_owned(), "Call DeclareWait alone after other tools finish.",
                ));
            }
            calls.retain(|call| call.name != DECLARE_WAIT_TOOL);
            return if calls.is_empty() { WaitToolStep::Retry } else { WaitToolStep::Proceed };
        }
        let Some(call) = calls.first() else { return WaitToolStep::Proceed };
        let result = match serde_json::from_str::<TurnWaitDeclaration>(&call.arguments) {
            Ok(wait) => self.declare_turn_wait(wait).await,
            Err(error) => Err(format!("Invalid wait declaration: {error}")),
        };
        let step = match &result {
            Ok(()) => WaitToolStep::Declared,
            Err(_) => WaitToolStep::Retry,
        };
        self.chat_state_handle.push_tool_result(ConversationItem::tool_result(
            call.id.as_ref().to_owned(),
            match result {
                Ok(()) => "Wait declared. Any selected completion or new input resumes the session.".to_string(),
                Err(error) => error,
            },
        ));
        step
    }

    async fn declare_turn_wait(&self, wait: TurnWaitDeclaration) -> Result<(), String> {
        if wait.is_empty() || self.startup_hints.is_subagent {
            return Err("Declare a nonempty wait in the owning parent session.".to_string());
        }
        let input_epoch = self.turn_wait_declaration.borrow().input_epoch;
        let live = self.snapshot_turn_wait().await;
        if self.has_runnable_queued_user_row().await
            || input_epoch != self.turn_wait_declaration.borrow().input_epoch
        {
            return Err("New input arrived while validating the wait; handle it before declaring a wait.".to_string());
        }
        if let Some(target) = wait.targets.iter().find(|target| !live.contains(target)) {
            return Err(format!("Wait target is stale or not owned by this session: {target:?}"));
        }
        self.record_turn_wait(wait);
        self.set_goal_loop_active_resource(false).await;
        Ok(())
    }

    pub(super) fn observe_wait_input(&self, origin: &PromptOrigin) {
        let breaks_wait = match origin {
            PromptOrigin::User | PromptOrigin::Delivery { .. }
            | PromptOrigin::ParentAgentMessage { .. } | PromptOrigin::ParentHumanMessage { .. }
            | PromptOrigin::SchedulerFired | PromptOrigin::PlanResume
            | PromptOrigin::SubagentCompleted { .. } | PromptOrigin::TaskCompleted { .. }
            | PromptOrigin::WorkflowCompleted { .. } | PromptOrigin::NotificationDrain => true,
            PromptOrigin::GoalSummary | PromptOrigin::GoalClassifierNudge => false,
        };
        if breaks_wait {
            self.break_declared_wait();
        } else {
            let mut state = self.turn_wait_declaration.borrow_mut();
            state.input_epoch = state.input_epoch.wrapping_add(1);
        }
    }

    /// Dispatch the observe-only session-end `Stop`.
    /// It runs in stop-gate mode so exit code 2 parses as a block, but the decision is discarded (no turn left to continue).
    pub(crate) async fn dispatch_session_end_stop(&self, reason: &str) {
        if self.startup_hints.is_subagent || !self.may_have_hooks_for(event::HookEventName::Stop) {
            return;
        }
        let envelope = self.fire_hook(
            event::HookEventName::Stop,
            None,
            event::HookPayload::Stop {
                reason: reason.to_string(),
                stop_hook_active: false,
                last_assistant_message: None,
                background_tasks: None,
                session_crons: None,
            },
        );
        let Some(registry) = self.hook_registry.borrow().clone() else {
            return;
        };
        let ctx = self.hook_run_ctx();
        let dispatch =
            dispatcher::dispatch_stop(&registry, event::HookEventName::Stop, &envelope, &ctx);
        let Ok(mut result) = tokio::time::timeout(SESSION_END_STOP_BUDGET, dispatch).await else {
            tracing::warn!("session-end stop hooks exceeded the shutdown budget; skipping");
            return;
        };
        result.results = demote_ignored_blocks(result.results);
        self.send_hook_execution(&HookBatch::from_envelope(&envelope), &result.results)
            .await;
        self.emit_hook_executed_telemetry("stop", None, &result.results)
            .await;
    }

    pub(crate) async fn list_active_subagents(
        &self,
    ) -> Vec<xai_grok_tools::implementations::grok_build::task::types::ActiveSubagentSummary> {
        use xai_grok_tools::implementations::grok_build::task::types::{
            SubagentEvent, SubagentListActiveRequest,
        };
        let Some(ref event_tx) = self.tool_context.subagent_event_tx else {
            return Vec::new();
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        if event_tx
            .send(SubagentEvent::ListActive(SubagentListActiveRequest {
                parent_session_id: self.session_id_string(),
                respond_to: tx,
            }))
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Snapshot still-running background work and scheduled wakeups for the Stop hook input.
    /// Tasks owned by other sessions on the shared backend are filtered out.
    async fn stop_gate_work_snapshot(&self) -> (Vec<StopBackgroundTask>, Vec<StopSessionCron>) {
        let bridge = self.tool_bridge_handle();
        let my_session = self.session_id_string();
        let mut tasks: Vec<StopBackgroundTask> = bridge
            .list_background_tasks()
            .await
            .iter()
            .filter(|t| t.is_outstanding())
            .filter(|t| {
                t.owner_session_id
                    .as_deref()
                    .is_none_or(|owner| owner == my_session)
            })
            .map(stop_entry_from_task)
            .collect();
        tasks.extend(
            self.list_active_subagents()
                .await
                .iter()
                .map(stop_entry_from_subagent),
        );

        let now = chrono::Utc::now();
        let crons = bridge
            .list_scheduled_tasks()
            .await
            .iter()
            .filter(|t| !t.is_expired(now))
            .map(stop_cron_from_scheduled)
            .collect();
        (tasks, crons)
    }

    async fn announce_force_stop(&self, prevent: &dispatcher::StopBlock) {
        self.send_hook_annotation(&format!(
            "\u{26a0} Hook `{}` stopped the agent: {}",
            prevent.hook_name, prevent.reason
        ))
        .await;
    }

    /// Snapshot the live background work a finishing turn declares a wait on.
    /// Subagent sessions never declare: the parent drives their lifecycle.
    /// Scheduled crons are excluded: they fire their own prompts on schedule instead of completing.
    pub(super) async fn snapshot_turn_wait(&self) -> TurnWaitDeclaration {
        let mut declaration = TurnWaitDeclaration::default();
        if self.startup_hints.is_subagent {
            return declaration;
        }
        let bridge = self.tool_bridge_handle();
        let my_session = self.session_id_string();
        for task in bridge.list_background_tasks().await {
            if !task.is_outstanding()
                || task.owner_session_id.as_deref() != Some(my_session.as_str())
            {
                continue;
            }
            declaration.push(task_wait_target(task.kind, &task.task_id));
        }
        for summary in self.list_active_subagents().await {
            declaration.push(WaitOnTarget::Subagent {
                subagent_id: summary.subagent_id.clone(),
            });
        }
        let tracker = self.workflow_tracker().await;
        for run in tracker.lock().list() {
            if run.status == crate::session::workflow::tracker::WorkflowRunStatus::Active {
                declaration.push(WaitOnTarget::WorkflowRun {
                    run_id: run.run_id.clone(),
                });
            }
        }
        declaration
    }

    /// Record the wait set the stop gate carried out of a finished turn.
    pub(super) fn record_turn_wait(&self, wait: TurnWaitDeclaration) {
        self.turn_wait_declaration.borrow_mut().declaration = wait;
    }

    /// Any new user input breaks a declared wait: the session is no longer parked on background work.
    pub(super) fn break_declared_wait(&self) {
        let mut state = self.turn_wait_declaration.borrow_mut();
        state.input_epoch = state.input_epoch.wrapping_add(1);
        state.declaration = TurnWaitDeclaration::default();
    }

    /// A completion of any declared target releases the wait, even if others remain live.
    pub(super) async fn declared_wait_blocks_continuation(&self) -> bool {
        let declaration = self.turn_wait_declaration.borrow().declaration.clone();
        if declaration.is_empty() {
            return false;
        }
        let live = self.snapshot_turn_wait().await;
        if declaration.targets.iter().all(|target| live.contains(target)) {
            true
        } else {
            self.break_declared_wait();
            false
        }
    }

    fn allow_stop(&self) -> StopGateDecision {
        StopGateDecision::AllowStop {
            wait: self.turn_wait_declaration.borrow().declaration.clone(),
        }
    }

    /// Answering the gate's hook request takes a client, so a test reaches the payload here.
    #[cfg(test)]
    pub(super) async fn stop_payload_for_test(&self) -> event::HookPayload {
        self.build_stop_payload(/* stop_hook_active */ false).await
    }

    async fn build_stop_payload(&self, stop_hook_active: bool) -> event::HookPayload {
        // The report is joined so a salvaged turn's gate sees the whole answer, not the last fragment; it is identical to the last text otherwise
        let last_assistant_message = self
            .chat_state_handle
            .get_trailing_assistant_report()
            .await
            .as_deref()
            .map(event::clip_assistant_message);
        if self.startup_hints.is_subagent {
            event::HookPayload::SubagentStop {
                phase: event::SubagentStopPhase::Gate,
                subagent_id: self.session_id_string(),
                subagent_type: self.subagent_type_label().unwrap_or_default(),
                stop_hook_active: Some(stop_hook_active),
                last_assistant_message,
            }
        } else {
            let (background_tasks, session_crons) = self.stop_gate_work_snapshot().await;
            event::HookPayload::Stop {
                reason: event::END_TURN_STOP_REASON.to_string(),
                stop_hook_active,
                last_assistant_message,
                background_tasks: Some(background_tasks),
                session_crons: Some(session_crons),
            }
        }
    }

    async fn emit_stop_results(&self, batch: &HookBatch, results: &[result::HookRunResult]) {
        self.send_hook_execution(batch, results).await;
        self.emit_hook_executed_telemetry(&batch.event_name, None, results)
            .await;
    }

    /// Run the turn-end `Stop`/`SubagentStop` hook gate and decide whether the agent may stop or must keep working.
    /// Every AllowStop carries the explicitly selected wait subset.
    /// Hook failures fail open (the agent stops normally).
    pub(super) async fn run_stop_gate(
        &self,
        prompt_id: &str,
        continuations_this_turn: u32,
    ) -> StopGateDecision {
        let event = if self.startup_hints.is_subagent {
            event::HookEventName::SubagentStop
        } else {
            event::HookEventName::Stop
        };
        let wait = self.turn_wait_declaration.borrow().declaration.clone();
        if !wait.is_empty() {
            self.set_goal_loop_active_resource(false).await;
        }
        if !self.has_enabled_hooks_for(event) {
            return self.allow_stop();
        }
        // At the cap no hook is consulted or notified for this forced stop, unlike the force-stop path below which still notifies observers
        if continuations_this_turn >= MAX_STOP_HOOK_CONTINUATIONS_PER_TURN {
            tracing::warn!(
                continuations_this_turn,
                "stop hook continuation limit reached; ending the turn"
            );
            self.send_hook_annotation(&format!(
                "\u{26a0} Stop hooks kept the agent working {MAX_STOP_HOOK_CONTINUATIONS_PER_TURN} times this turn: limit reached, ending the turn"
            ))
            .await;
            return self.allow_stop();
        }

        // Claim before the hooks, not after: a turn already reported is being torn down
        // The gate then skips instead of running verification hooks for a turn the user cancelled
        let Some(claim) = self.turn_report.claim_for_gate() else {
            tracing::debug!(
                prompt_id,
                "a cancel already reported this turn; the stop gate did not run"
            );
            return self.allow_stop();
        };

        let payload = self.build_stop_payload(continuations_this_turn > 0).await;
        // The gate builds its envelope via `make_hook_envelope`, not the observe-notify `fire_hook`
        // Client hooks get the awaited `x.ai/hooks/run` request below, not a fire-and-forget event
        let envelope = self.make_hook_envelope(event, Some(prompt_id.to_string()), payload);

        let mut result = dispatcher::StopDispatchResult::default();
        // Clone out of the RefCell before the awaits so no `Ref` is held across them
        let registry = self.hook_registry.borrow().clone();
        let batch = if let Some(registry) = registry {
            let ctx = self.hook_run_ctx();
            let batch = self.announce_hook_run(&registry, &envelope, &ctx);
            result = dispatcher::dispatch_stop(&registry, event, &envelope, &ctx).await;
            batch
        } else {
            HookBatch::from_envelope(&envelope)
        };

        if let Some(prevent) = result.prevent_continuation.take() {
            // Force-stop: skip the client gate, its signals would be discarded
            // Still send the observe notification so client callbacks see the turn end
            self.emit_stop_results(&batch, &result.results).await;
            self.notify_client_hooks(&envelope);
            commit_stop_report(claim, prompt_id);
            self.announce_force_stop(&prevent).await;
            return self.allow_stop();
        }

        // Merge file and client results and emit once: one stop gate is one scrollback entry and one telemetry batch
        let client = self.run_stop_client_hooks(&envelope).await;
        let mut all_results = std::mem::take(&mut result.results);
        all_results.extend(client.results);
        if !all_results.is_empty() {
            self.emit_stop_results(&batch, &all_results).await;
        }

        result.blocks.extend(client.blocks);
        result.additional_context.extend(client.additional_context);
        if let Some(prevent) = client.prevent_continuation {
            commit_stop_report(claim, prompt_id);
            self.announce_force_stop(&prevent).await;
            return self.allow_stop();
        }

        if !result.wants_continuation() {
            // A gate whose hooks all skipped or crashed told nobody the turn ended
            // It releases the claim so a later cancel or failure can still report
            let any_hook_succeeded = all_results
                .iter()
                .any(|r| matches!(r, result::HookRunResult::Success { .. }));
            if any_hook_succeeded {
                commit_stop_report(claim, prompt_id);
            }
            return self.allow_stop();
        }

        // The turn has not ended, so a later interrupt must still be able to report it.
        drop(claim);
        self.announce_keep_working(&result.blocks, &result.additional_context)
            .await;
        StopGateDecision::KeepWorking {
            feedback: result.feedback(),
        }
    }

    /// Annotate the scrollback when a stop gate keeps the agent working.
    /// Each block gets one line (with `HookBlocked` telemetry); when only `additionalContext` was returned, the context lines are written instead.
    async fn announce_keep_working(
        &self,
        blocks: &[dispatcher::StopBlock],
        additional_context: &[String],
    ) {
        for block in blocks {
            self.send_hook_annotation(&format!(
                "\u{21a9} Stop blocked by hook `{}`, continuing: {}",
                block.hook_name, block.reason
            ))
            .await;
            xai_grok_telemetry::session_ctx::log_event(xai_grok_telemetry::events::HookBlocked {
                hook_name: block.hook_name.clone(),
                cause: xai_grok_telemetry::events::HookBlockCause::StopBlocked,
            });
        }
        if blocks.is_empty() {
            for context in additional_context {
                self.send_hook_annotation(&format!(
                    "\u{21a9} Stop hook feedback, continuing: {context}"
                ))
                .await;
            }
        }
    }
}

#[cfg(test)]
mod stop_gate_snapshot_tests {
    use super::*;

    fn task_snapshot(
        kind: xai_grok_tools::computer::types::TaskKind,
    ) -> xai_grok_tools::types::TaskSnapshot {
        xai_grok_tools::types::TaskSnapshot {
            task_id: "task-1".into(),
            command: "sandbox-exec tail -f /var/log/syslog".into(),
            display_command: Some("tail -f /var/log/syslog".into()),
            cwd: "/tmp".into(),
            start_time: std::time::SystemTime::UNIX_EPOCH,
            end_time: None,
            output: String::new(),
            output_file: std::path::PathBuf::from("/tmp/out"),
            truncated: false,
            exit_code: None,
            signal: None,
            completed: false,
            kind,
            block_waited: false,
            explicitly_killed: false,
            kill_result_delivered: false,
            owner_session_id: None,
            description: None,
            is_backgrounded: false,
            output_total_bytes: 0,
        }
    }

    #[test]
    fn task_snapshot_maps_to_stop_entry() {
        let shell = stop_entry_from_task(&task_snapshot(
            xai_grok_tools::computer::types::TaskKind::Bash,
        ));
        assert_eq!(shell.r#type, BackgroundTaskType::Shell);
        assert_eq!(shell.command.as_deref(), Some("tail -f /var/log/syslog"));
        assert!(shell.description.is_none());
        assert_eq!(shell.status, "running");
        assert!(shell.agent_type.is_none());

        let monitor = stop_entry_from_task(&task_snapshot(
            xai_grok_tools::computer::types::TaskKind::Monitor,
        ));
        assert_eq!(monitor.r#type, BackgroundTaskType::Monitor);
        assert!(monitor.command.is_none());
        assert_eq!(
            monitor.description.as_deref(),
            Some("tail -f /var/log/syslog")
        );
    }

    #[test]
    fn subagent_summary_maps_to_stop_entry() {
        let summary =
            xai_grok_tools::implementations::grok_build::task::types::ActiveSubagentSummary {
                subagent_id: "sub-1".into(),
                subagent_type: "explore".into(),
                description: "d".repeat(2000),
                elapsed_ms: 5,
            };
        let entry = stop_entry_from_subagent(&summary);
        assert_eq!(entry.r#type, BackgroundTaskType::Subagent);
        assert_eq!(entry.agent_type.as_deref(), Some("explore"));
        let description = entry.description.unwrap();
        assert!(description.ends_with("… [+1000 chars]"));
        assert!(entry.command.is_none());
    }

    #[test]
    fn scheduled_task_maps_to_stop_cron() {
        let task =
            xai_grok_tools::implementations::grok_build::scheduler::types::ScheduledTask::new(
                300,
                "check the build".into(),
                true,
                false,
            );
        let cron = stop_cron_from_scheduled(&task);
        assert_eq!(cron.schedule, "every 5 minutes");
        assert!(cron.recurring);
        assert_eq!(cron.prompt, "check the build");
    }

    #[test]
    fn demote_ignored_blocks_downgrades_only_blocked() {
        use xai_grok_hooks::result::HookRunResult;

        let results = demote_ignored_blocks(vec![
            HookRunResult::Blocked {
                hook_name: "gate".into(),
                detail: "blocked stop: run the tests".into(),
                elapsed: std::time::Duration::from_millis(5),
                http_info: None,
                system_message: None,
            },
            HookRunResult::Failed {
                hook_name: "broken".into(),
                error: "exit code 1".into(),
                elapsed: std::time::Duration::from_millis(3),
                http_info: None,
                system_message: None,
            },
            HookRunResult::Skipped {
                hook_name: "disabled".into(),
            },
        ]);

        let [ok, failed, skipped] = results.as_slice() else {
            panic!("expected three hook results: {results:?}");
        };
        assert!(
            matches!(ok, HookRunResult::Success { hook_name, .. } if hook_name == "gate"),
            "a discarded decision must read as success, got {ok:?}"
        );
        assert!(matches!(failed, HookRunResult::Failed { .. }));
        assert!(matches!(skipped, HookRunResult::Skipped { .. }));
    }
}

#[cfg(test)]
#[path = "stop_gate_tests.rs"]
mod tests;
