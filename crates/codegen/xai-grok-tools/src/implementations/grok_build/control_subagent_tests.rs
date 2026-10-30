use pretty_assertions::assert_eq;

use super::*;
use crate::implementations::grok_build::task::backend::ChannelBackend;
use crate::implementations::grok_build::task::types::{
    InterruptionCause, SubagentCancelTarget, SubagentEvent, SubagentState,
};
use crate::types::output::ToolOutput;
use crate::types::resources::Resources;
use crate::types::tool_metadata::test_ctx;

/// Runs the tool as session `root` against a coordinator stand-in that answers with the cause the request asked for.
async fn control(action: SubagentCancelDisposition) -> (SubagentCancelTarget, ToolOutput) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut resources = Resources::new();
    resources.insert(ChannelBackend::for_session(tx, "root").into_resource());
    resources.insert(OwnerSessionId("root".to_owned()));
    let run = xai_tool_runtime::Tool::run(
        &ControlSubagentTool,
        test_ctx(resources.into_shared()),
        ControlSubagentInput {
            subagent_id: "child".to_owned(),
            action,
        },
    );
    let coordinator = async move {
        let Some(SubagentEvent::Cancel(request)) = rx.recv().await else {
            panic!("expected a cancel request");
        };
        let SubagentCancelTarget::SubagentId {
            actor, disposition, ..
        } = request.target.clone()
        else {
            panic!("expected a cancel by subagent id");
        };
        let cause = match disposition {
            SubagentCancelDisposition::Stop => InterruptionCause::ExplicitStop { actor },
            SubagentCancelDisposition::Pause => InterruptionCause::Paused { actor },
        };
        let _ = request
            .respond_to
            .send(SubagentCancelOutcome::Cancelled { cause });
        request.target
    };
    let (output, target) = tokio::join!(run, coordinator);
    let output = ToolOutput::ControlSubagent(output.expect("control_subagent runs"));
    (target, output)
}

#[tokio::test]
async fn pause_is_requested_for_the_parent_model_and_tells_how_to_resume() {
    let (target, output) = control(SubagentCancelDisposition::Pause).await;
    let SubagentCancelTarget::SubagentId {
        id,
        actor,
        disposition,
        binding,
    } = target
    else {
        panic!("expected a cancel by subagent id");
    };
    assert!(matches!(binding, crate::implementations::grok_build::task::types::SubagentCancelBinding::Current));
    assert_eq!(
        (id.as_str(), actor, disposition),
        (
            "child",
            SubagentActor::ParentModel {
                session_id: "root".to_owned()
            },
            SubagentCancelDisposition::Pause
        )
    );
    assert_eq!(
        output.to_prompt_format(),
        "Subagent interruption requested (paused by parent model); awaiting terminal confirmation. After confirmation, send it a message to resume it."
    );
    assert!(!output.is_error());
}

#[tokio::test]
async fn stop_is_requested_as_a_stop_and_answers_the_completion_notice() {
    let (target, output) = control(SubagentCancelDisposition::Stop).await;
    assert!(matches!(
        target,
        SubagentCancelTarget::SubagentId {
            disposition: SubagentCancelDisposition::Stop,
            ..
        }
    ));
    assert_eq!(
        output.to_prompt_format(),
        "Subagent interruption requested (stopped by parent model); awaiting terminal confirmation."
    );
    assert_eq!(
        crate::reminders::task_completion::consumed_completion_ids(&output),
        ["child"]
    );
}

#[test]
fn an_already_paused_subagent_reports_its_pause() {
    let output = ToolOutput::ControlSubagent(ControlSubagentOutput {
        subagent_id: "child".to_owned(),
        outcome: SubagentCancelOutcome::AlreadyFinished {
            state: SubagentState::Interrupted {
                cause: InterruptionCause::Paused {
                    actor: SubagentActor::Human,
                },
            },
        },
    });
    assert_eq!(
        output.to_prompt_format(),
        "Subagent had already ended. Subagent was paused by human user; send it a message to resume it."
    );
}

#[test]
fn an_unknown_subagent_is_an_error_that_answers_no_notice() {
    let output = ToolOutput::ControlSubagent(ControlSubagentOutput {
        subagent_id: "child".to_owned(),
        outcome: SubagentCancelOutcome::NotFound,
    });
    assert!(output.is_error());
    assert!(crate::reminders::task_completion::consumed_completion_ids(&output).is_empty());
}

#[tokio::test]
async fn control_subagent_transport_loss_is_an_error_not_an_absence() {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    drop(rx);
    let mut resources = Resources::new();
    resources.insert(ChannelBackend::for_session(tx, "root").into_resource());
    resources.insert(OwnerSessionId("root".to_owned()));
    let error = xai_tool_runtime::Tool::run(
        &ControlSubagentTool,
        test_ctx(resources.into_shared()),
        ControlSubagentInput {
            subagent_id: "child".to_owned(),
            action: SubagentCancelDisposition::Stop,
        },
    ).await.expect_err("unavailable coordinator must not return NotFound");
    assert!(error.to_string().contains("not confirmed"));
    assert!(error.to_string().contains("coordinator unavailable"));
}

#[test]
fn action_accepts_exactly_pause_and_stop() {
    let parse = |action: &str| {
        serde_json::from_value::<ControlSubagentInput>(serde_json::json!({
            "subagent_id": "child",
            "action": action,
        }))
        .map(|input| input.action)
        .ok()
    };
    assert_eq!(
        ["pause", "stop", "resume"].map(parse),
        [
            Some(SubagentCancelDisposition::Pause),
            Some(SubagentCancelDisposition::Stop),
            None
        ]
    );
}
