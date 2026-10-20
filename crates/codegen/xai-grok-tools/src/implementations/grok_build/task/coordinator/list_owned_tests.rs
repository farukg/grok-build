use pretty_assertions::assert_eq;

use super::super::tests::{harness, parent_backend, request};
use crate::implementations::grok_build::control_subagent::{
    ControlSubagentInput, ControlSubagentTool,
};
use crate::implementations::grok_build::list_subagents::{
    ListSubagentsInput, ListSubagentsOutput, ListSubagentsTool,
};
use crate::implementations::grok_build::task::backend::{ChannelBackend, SubagentBackend};
use crate::implementations::grok_build::task::types::{
    InterruptionCause, SubagentActor, SubagentCancelDisposition, SubagentState,
};
use crate::types::resources::{OwnerSessionId, Resources};
use crate::types::tool_metadata::test_ctx;

fn session_resources(backend: ChannelBackend, session_id: &str) -> Resources {
    let mut resources = Resources::new();
    resources.insert(backend.into_resource());
    resources.insert(OwnerSessionId(session_id.to_owned()));
    resources
}

async fn list(backend: ChannelBackend, session_id: &str) -> ListSubagentsOutput {
    xai_tool_runtime::Tool::run(
        &ListSubagentsTool,
        test_ctx(session_resources(backend, session_id).into_shared()),
        ListSubagentsInput {},
    )
    .await
    .expect("list_subagents runs")
}

#[tokio::test]
async fn a_paused_subagent_stays_listed_with_its_cause_next_to_a_running_one() {
    let mut harness = harness(false, std::time::Duration::from_secs(60));
    let backend = parent_backend(&harness);
    let mut spawns = Vec::new();
    for id in ["kept", "paused"] {
        spawns.push(tokio::spawn({
            let backend = backend.clone();
            async move { backend.spawn(request(id, true), None).await }
        }));
        harness.requests.recv().await.expect("child observed");
        assert_eq!(harness.started.recv().await.as_deref(), Some(id));
    }

    xai_tool_runtime::Tool::run(
        &ControlSubagentTool,
        test_ctx(session_resources(backend.clone(), "parent").into_shared()),
        ControlSubagentInput {
            subagent_id: "paused".to_owned(),
            action: SubagentCancelDisposition::Pause,
        },
    )
    .await
    .expect("control_subagent runs");
    harness.completions.recv().await.expect("paused child settles");

    let listed = list(backend, "parent").await;
    assert_eq!(
        listed
            .subagents
            .iter()
            .map(|subagent| (subagent.subagent_id.as_str(), subagent.state.clone()))
            .collect::<Vec<_>>(),
        [
            ("kept", SubagentState::Running),
            (
                "paused",
                SubagentState::Interrupted {
                    cause: InterruptionCause::Paused {
                        actor: SubagentActor::ParentModel {
                            session_id: "parent".to_owned()
                        }
                    }
                }
            ),
        ]
    );
    assert!(listed.to_string().ends_with(
        "Subagent was paused by parent model; send it a message to resume it."
    ));

    let foreign = ChannelBackend::for_coordinator_session(
        harness
            .backend
            .coordinator_sender()
            .expect("coordinator sender"),
        "foreign",
    );
    assert_eq!(
        list(foreign, "foreign").await.to_string(),
        "This session has no subagents."
    );
    harness.actor.abort();
}
