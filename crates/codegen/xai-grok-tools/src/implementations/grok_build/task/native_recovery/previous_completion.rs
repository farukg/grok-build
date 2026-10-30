use super::super::coordinator_state::{
    CompletedChild, CompletionReplyTarget, TerminalPublicationState, instant_to_epoch_ms,
};
use super::super::types::{
    AgentAddress, SpawnRootSpan, SubagentOwner, SubagentRequest, SubagentResult,
    SubagentRuntimeOverrides, SubagentState,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PreviousRequest {
    description: String,
    subagent_type: String,
    parent_session_id: String,
    parent_prompt_id: Option<String>,
    resume_from: Option<String>,
    cwd: Option<String>,
    runtime_overrides: SubagentRuntimeOverrides,
    run_in_background: bool,
    surface_completion: bool,
    await_to_completion: bool,
    fork_context: bool,
    traceparent: Option<String>,
    tool_call_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PreviousResult {
    subagent_id: String,
    child_session_id: String,
    state: SubagentState,
    tool_calls: u32,
    turns: u32,
    duration_ms: u64,
    tokens_used: u64,
    output_tokens_used: u64,
    total_tokens_used: u64,
    output_usage_incomplete: bool,
    worktree_path: Option<String>,
    subagent_type: String,
    backgrounded: bool,
    resume_fallback: Option<xai_tool_types::SubagentResumeFallback>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviousCompletion {
    pub attempt_id: String,
    request: PreviousRequest,
    result: PreviousResult,
    started_at_epoch_ms: u64,
    pub persona: Option<String>,
    pub resumed_from: Option<String>,
    child_session_id: String,
    pub child_cwd: String,
    pub worktree_path: Option<String>,
    pub snapshot_ref: Option<String>,
    pub persisted_output_ref: Option<String>,
    pub effective_model_id: String,
    agent_address: Option<String>,
    spawner_session_id: Option<String>,
    wake_eligible: bool,
}

impl PreviousCompletion {
    pub fn parent_session_id(&self) -> &str {
        &self.request.parent_session_id
    }
    pub fn state(&self) -> &SubagentState {
        &self.result.state
    }

    pub(in crate::implementations::grok_build::task) fn capture(child: &CompletedChild) -> Self {
        let request = &child.request;
        let result = &child.result;
        let mut runtime_overrides = request.runtime_overrides.clone();
        runtime_overrides.child_submission = Default::default();
        Self {
            attempt_id: child.attempt_id.to_string(),
            request: PreviousRequest {
                description: request.description.clone(),
                subagent_type: request.subagent_type.clone(),
                parent_session_id: request.parent_session_id.clone(),
                parent_prompt_id: request.parent_prompt_id.clone(),
                resume_from: request.resume_from.clone(),
                cwd: request.cwd.clone(),
                runtime_overrides,
                run_in_background: request.run_in_background,
                surface_completion: request.surface_completion,
                await_to_completion: request.await_to_completion,
                fork_context: request.fork_context,
                traceparent: request.spawn_root.traceparent().map(str::to_owned),
                tool_call_id: request.tool_call_id.clone(),
            },
            result: PreviousResult {
                subagent_id: result.subagent_id.clone(),
                child_session_id: result.child_session_id.clone(),
                state: result.state.clone(),
                tool_calls: result.tool_calls,
                turns: result.turns,
                duration_ms: result.duration_ms,
                tokens_used: result.tokens_used,
                output_tokens_used: result.output_tokens_used,
                total_tokens_used: result.total_tokens_used,
                output_usage_incomplete: result.output_usage_incomplete,
                worktree_path: result.worktree_path.clone(),
                subagent_type: result.subagent_type.clone(),
                backgrounded: result.backgrounded,
                resume_fallback: result.resume_fallback.clone(),
            },
            started_at_epoch_ms: instant_to_epoch_ms(child.started_at),
            persona: child.persona.clone(),
            resumed_from: child.resumed_from.clone(),
            child_session_id: child.child_session_id.clone(),
            child_cwd: child.child_cwd.clone(),
            worktree_path: child.worktree_path.clone(),
            snapshot_ref: child.snapshot_ref.clone(),
            persisted_output_ref: child.persisted_output_ref.clone(),
            effective_model_id: child.effective_model_id.clone(),
            agent_address: child.agent_address.as_ref().map(ToString::to_string),
            spawner_session_id: child.spawner_session_id.clone(),
            wake_eligible: child.wake_eligible,
        }
    }

    pub(in crate::implementations::grok_build::task) fn restore(
        &self,
        id: &str,
        prompt: String,
        completion_age: u64,
    ) -> Option<CompletedChild> {
        self.persisted_output_ref.as_ref()?;
        if self.child_session_id != id
            || self.result.subagent_id != id
            || self.result.child_session_id != id
        {
            return None;
        }
        match &self.result.state {
            SubagentState::Running => return None,
            SubagentState::Completed
            | SubagentState::Failed { .. }
            | SubagentState::Interrupted { .. } => {}
        }
        let attempt_id = xai_message_delivery_core::AttemptId::parse(&self.attempt_id)?;
        let started = std::time::UNIX_EPOCH
            .checked_add(std::time::Duration::from_millis(self.started_at_epoch_ms))?;
        let elapsed = std::time::SystemTime::now().duration_since(started).ok()?;
        let started_at = std::time::Instant::now().checked_sub(elapsed)?;
        let agent_address = match &self.agent_address {
            Some(address) => Some(AgentAddress::parse(address)?),
            None => None,
        };
        let request = &self.request;
        let result = &self.result;
        Some(CompletedChild {
            attempt_id,
            completion_age,
            publication: TerminalPublicationState::Published,
            reply_target: CompletionReplyTarget::Absent,
            request: SubagentRequest {
                id: id.to_owned(),
                prompt,
                description: request.description.clone(),
                subagent_type: request.subagent_type.clone(),
                parent_session_id: request.parent_session_id.clone(),
                parent_prompt_id: request.parent_prompt_id.clone(),
                resume_from: request.resume_from.clone(),
                cwd: request.cwd.clone(),
                runtime_overrides: request.runtime_overrides.clone(),
                run_in_background: request.run_in_background,
                surface_completion: request.surface_completion,
                await_to_completion: request.await_to_completion,
                fork_context: request.fork_context,
                owner: SubagentOwner::Task,
                cancel_token: tokio_util::sync::CancellationToken::new(),
                spawn_root: SpawnRootSpan::from_traceparent(request.traceparent.clone()),
                tool_call_id: request.tool_call_id.clone(),
            },
            started_at,
            child_session_id: self.child_session_id.clone(),
            persona: self.persona.clone(),
            resumed_from: self.resumed_from.clone(),
            child_cwd: self.child_cwd.clone(),
            worktree_path: self.worktree_path.clone(),
            snapshot_ref: self.snapshot_ref.clone(),
            persisted_output_ref: self.persisted_output_ref.clone(),
            effective_model_id: self.effective_model_id.clone(),
            agent_address,
            spawner_session_id: self.spawner_session_id.clone(),
            wake_eligible: self.wake_eligible,
            result: SubagentResult {
                state: result.state.clone(),
                output: std::sync::Arc::from(""),
                subagent_id: result.subagent_id.clone(),
                child_session_id: result.child_session_id.clone(),
                tool_calls: result.tool_calls,
                turns: result.turns,
                duration_ms: result.duration_ms,
                tokens_used: result.tokens_used,
                output_tokens_used: result.output_tokens_used,
                total_tokens_used: result.total_tokens_used,
                output_usage_incomplete: result.output_usage_incomplete,
                worktree_path: result.worktree_path.clone(),
                subagent_type: result.subagent_type.clone(),
                backgrounded: result.backgrounded,
                resume_fallback: result.resume_fallback.clone(),
            },
        })
    }
}
