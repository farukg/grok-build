//! `x.ai/subagent/resume` — continue an owned subagent with a human prompt.

use agent_client_protocol as acp;
use serde::{Deserialize, Serialize};
use xai_grok_tools::implementations::grok_build::task::resume::{
    SubagentResumeError, SubagentResumeRoute,
};

use crate::agent::MvpAgent;
use crate::session::ExtMethodResult;

use super::ExtResult;

/// Wire DTO for the `x.ai/subagent/resume` request; `subagentId` accepts a full id, a child
/// session id, or a unique prefix.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeSubagentRequest {
    pub session_id: String,
    pub subagent_id: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ResumeSubagentOutcome {
    /// The prompt is queued as the next turn of the same subagent (woken first when it had finished).
    Queued { subagent_id: String, message_id: String },
    /// A continuation was spawned from the finished subagent; it reports as a new subagent.
    Resumed { source_id: String },
    Refused { reason: SubagentResumeError },
}

impl From<Result<SubagentResumeRoute, SubagentResumeError>> for ResumeSubagentOutcome {
    fn from(route: Result<SubagentResumeRoute, SubagentResumeError>) -> Self {
        match route {
            Ok(
                SubagentResumeRoute::Delivered {
                    subagent_id,
                    message_id,
                }
                | SubagentResumeRoute::Woken {
                    subagent_id,
                    message_id,
                },
            ) => Self::Queued {
                subagent_id,
                message_id,
            },
            Ok(SubagentResumeRoute::Spawn { source_id }) => Self::Resumed { source_id },
            Err(reason) => Self::Refused { reason },
        }
    }
}

pub(crate) async fn handle(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    let req: ResumeSubagentRequest = super::parse_params(args)?;
    let outcome = ResumeSubagentOutcome::from(
        agent
            .resume_subagent(&req.session_id, &req.subagent_id, req.prompt)
            .await,
    );
    ExtMethodResult::from_result(Ok::<_, String>(outcome))
        .to_ext_response()
        .map_err(|e| acp::Error::internal_error().data(e.to_string()))
}
