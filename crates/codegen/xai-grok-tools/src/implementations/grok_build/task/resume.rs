//! Subagent resume: reference resolution, typed refusal reasons, and routing.

use super::backend::SubagentBackend;
use super::types::{
    ActiveAgentMessageOperation, ActiveAgentMessageOutcome, ActiveAgentMessageRequest,
    SubagentOwner, SubagentRequest,
};
use crate::implementations::grok_build::send_subagent_message::SendSubagentMessageOutput;

/// Shortest reference accepted as an ID prefix; shorter references must match exactly.
/// UUIDv7 ids share their leading timestamp digits, so very short prefixes collide.
pub const MIN_SUBAGENT_ID_PREFIX_LEN: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentReferenceMatch {
    Unique(String),
    Ambiguous(Vec<String>),
    NoMatch,
}

/// Resolve `reference` against `(key, subagent_id)` pairs, where a key is a subagent id or a
/// child session id. An exact key wins, else a unique key prefix of at least
/// [`MIN_SUBAGENT_ID_PREFIX_LEN`] characters. Returns the subagent id.
pub fn match_subagent_reference<'a>(
    reference: &str,
    keys: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> SubagentReferenceMatch {
    let accepts_prefix = reference.len() >= MIN_SUBAGENT_ID_PREFIX_LEN;
    let mut prefixed: Vec<&str> = Vec::new();
    for (key, subagent_id) in keys {
        if key == reference {
            return SubagentReferenceMatch::Unique(subagent_id.to_owned());
        }
        if accepts_prefix && key.starts_with(reference) {
            prefixed.push(subagent_id);
        }
    }
    prefixed.sort_unstable();
    prefixed.dedup();
    match prefixed.as_slice() {
        [] => SubagentReferenceMatch::NoMatch,
        [only] => SubagentReferenceMatch::Unique((*only).to_owned()),
        many => {
            SubagentReferenceMatch::Ambiguous(many.iter().map(|id| (*id).to_owned()).collect())
        }
    }
}

/// Why a `resume_from` reference cannot be resumed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SubagentResumeError {
    NotFound {
        reference: String,
    },
    Ambiguous {
        reference: String,
        candidates: Vec<String>,
    },
    ForeignParent {
        subagent_id: String,
    },
    /// A resume spawn found its source live again.
    StillRunning {
        subagent_id: String,
    },
    /// The source is live and the prompt could not be queued to it.
    NotDelivered {
        subagent_id: String,
        delivery: SendSubagentMessageOutput,
    },
    /// The record still says running, yet no live subagent owns it in this process.
    NotTerminal {
        subagent_id: String,
    },
    MetaUnreadable {
        subagent_id: String,
        reason: String,
    },
    ParentSessionUnavailable {
        session_id: String,
    },
    CoordinatorUnavailable,
    SpawnRejected {
        reason: String,
    },
}

impl std::fmt::Display for SubagentResumeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { reference } => write!(
                f,
                "Cannot resume from subagent '{reference}': no subagent of this session matches it. \
                 Pass the full subagent_id or a unique prefix of at least {MIN_SUBAGENT_ID_PREFIX_LEN} characters."
            ),
            Self::Ambiguous {
                reference,
                candidates,
            } => write!(
                f,
                "Cannot resume from subagent '{reference}': it matches several subagents ({}). \
                 Pass more characters of the ID.",
                candidates.join(", ")
            ),
            Self::ForeignParent { subagent_id } => write!(
                f,
                "Cannot resume from subagent '{subagent_id}': it belongs to another parent session."
            ),
            Self::StillRunning { subagent_id } => write!(
                f,
                "Cannot resume from subagent '{subagent_id}': it is still running. Wait for it to \
                 finish, or message it with send_subagent_message."
            ),
            Self::NotDelivered {
                subagent_id,
                delivery,
            } => write!(
                f,
                "Cannot resume from subagent '{subagent_id}': it is still running and the prompt \
                 could not be queued to it ({delivery}). Wait for it to finish, then resume it, or \
                 message it with send_subagent_message."
            ),
            Self::NotTerminal { subagent_id } => write!(
                f,
                "Cannot resume from subagent '{subagent_id}': its record still says running, but no \
                 live subagent owns it (it was interrupted, likely by a restart). It is marked \
                 cancelled shortly; retry the resume then."
            ),
            Self::MetaUnreadable {
                subagent_id,
                reason,
            } => write!(
                f,
                "Cannot resume from subagent '{subagent_id}': its metadata is unreadable ({reason})."
            ),
            Self::ParentSessionUnavailable { session_id } => write!(
                f,
                "Cannot resume a subagent of session '{session_id}': the session is not loaded."
            ),
            Self::CoordinatorUnavailable => {
                f.write_str("Cannot resume the subagent: the subagent coordinator is unavailable.")
            }
            Self::SpawnRejected { reason } => {
                write!(f, "Cannot resume the subagent: the spawn was rejected ({reason}).")
            }
        }
    }
}

impl std::error::Error for SubagentResumeError {}

/// Where a resolved `resume_from` reference points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentResumeTarget {
    /// A live (running, pending, or queued) subagent owned by the caller.
    Running { subagent_id: String },
    /// A finished subagent, known in memory or on disk; resume spawns from it.
    Finished { subagent_id: String },
}

/// What a resume did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentResumeRoute {
    /// The prompt was queued to the running subagent as its next turn.
    Delivered {
        subagent_id: String,
        message_id: String,
    },
    /// Spawn a continuation with `resume_from = source_id`.
    Spawn { source_id: String },
}

/// Resolve `reference` and queue the prompt to a running target; a finished target is left to
/// the caller's spawn. Shared by the `task` tool and `x.ai/subagent/resume`.
pub async fn route_subagent_resume(
    backend: &dyn SubagentBackend,
    reference: &str,
    prompt: &str,
    parent_session_id: &str,
) -> Result<SubagentResumeRoute, SubagentResumeError> {
    match backend.resolve_resume(reference, parent_session_id).await? {
        SubagentResumeTarget::Finished { subagent_id } => Ok(SubagentResumeRoute::Spawn {
            source_id: subagent_id,
        }),
        SubagentResumeTarget::Running { subagent_id } => {
            let outcome = match ActiveAgentMessageRequest::try_new_with_operation(
                subagent_id.as_str(),
                prompt,
                ActiveAgentMessageOperation::Queue,
            ) {
                Ok(request) => backend.send_active_message(request).await,
                Err(outcome) => outcome,
            };
            match outcome {
                ActiveAgentMessageOutcome::Accepted { message_id } => {
                    Ok(SubagentResumeRoute::Delivered {
                        subagent_id,
                        message_id,
                    })
                }
                refused => Err(SubagentResumeError::NotDelivered {
                    subagent_id,
                    delivery: refused.into(),
                }),
            }
        }
    }
}

/// Model-facing notice for [`SubagentResumeRoute::Delivered`].
pub fn format_resume_delivered(subagent_id: &str, message_id: &str) -> String {
    format!(
        "Subagent '{subagent_id}' is still running, so the prompt was queued to it as its next turn \
         (message_id: {message_id}) instead of starting a new subagent. Its result arrives when it \
         finishes."
    )
}

/// Background continuation of `source_id` requested by a human, not a model tool call.
pub fn human_resume_request(
    source_id: String,
    prompt: String,
    parent_session_id: String,
) -> SubagentRequest {
    SubagentRequest {
        id: uuid::Uuid::now_v7().to_string(),
        prompt,
        description: format!("resume of {source_id}"),
        subagent_type: xai_tool_types::default_subagent_type(),
        parent_session_id,
        parent_prompt_id: None,
        resume_from: Some(source_id),
        cwd: None,
        runtime_overrides: Default::default(),
        run_in_background: true,
        surface_completion: true,
        await_to_completion: false,
        fork_context: false,
        owner: SubagentOwner::Task,
        cancel_token: tokio_util::sync::CancellationToken::new(),
        spawn_root: Default::default(),
        tool_call_id: None,
    }
}

#[cfg(test)]
#[path = "resume_tests.rs"]
mod tests;
