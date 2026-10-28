//! Recognizes `send_subagent_message` tool calls and maps them to scrollback blocks.
//!
//! The block takes the subagent id and text straight from the deserialized input, with no re-validation.
//! A rejected send therefore still shows the exact destination and text that was attempted.
//! The input is read through a pager-local lenient view rather than the tool's own type, so a
//! `delivery` value this pager predates still renders the destination and text.
//! The id is resolved to a display label through the tracker's registry; an unknown id falls back to the raw id.

use agent_client_protocol as acp;
use serde::Deserialize;
use xai_grok_tools::implementations::grok_build::send_subagent_message::{
    SEND_SUBAGENT_MESSAGE_TOOL_NAME, SendSubagentMessageDisposition, SendSubagentMessageOutput,
};
use xai_grok_tools::tool_taxonomy::{CanonicalToolMeta, TOOL_META_KEY, TOOL_META_VERSION};
use xai_grok_tools::types::output::ToolOutput;
use xai_grok_tools::types::tool::ToolKind;

use crate::acp::subagent_label_registry::SubagentLabelRegistry;
use crate::scrollback::block::RenderBlock;
use crate::scrollback::blocks::tool::{
    SentMessageDelivery, SentMessageInput, SentMessagePresentation, SentMessageTarget,
    SentMessageToolCallBlock, ToolCallBlock,
};

/// The wire alias a child-depth sender uses for its parent; matched on the untrimmed value, as the shell does,
/// before any registry lookup.
const PARENT_TARGET: &str = "parent";

/// Unknown fields are ignored, so the direct input and the `variant`-tagged `ToolInput`
/// envelope both parse.
#[derive(Deserialize)]
struct SentMessageWireInput {
    /// A missing id still renders the text; the row then names a bare `subagent`.
    #[serde(default)]
    subagent_id: Option<String>,
    text: String,
    #[serde(default)]
    delivery: Option<serde_json::Value>,
    /// Legacy flag; the tool consults it only when `delivery` is absent.
    #[serde(default)]
    queue: bool,
}

impl SentMessageWireInput {
    /// `None` for a value this pager does not know; a newer shell may accept one.
    fn recognized_delivery(&self) -> Option<SentMessageDelivery> {
        match &self.delivery {
            None if self.queue => Some(SentMessageDelivery::Queue),
            None => Some(SentMessageDelivery::Steer),
            Some(serde_json::Value::String(delivery)) => match delivery.as_str() {
                "steer" => Some(SentMessageDelivery::Steer),
                "queue" => Some(SentMessageDelivery::Queue),
                "interject" => Some(SentMessageDelivery::Interject),
                _ => None,
            },
            Some(_) => None,
        }
    }
}

pub(super) fn is_tool(tool_call: &acp::ToolCall) -> bool {
    match tool_call
        .meta
        .as_ref()
        .and_then(|meta| meta.get(TOOL_META_KEY))
    {
        Some(meta) => serde_json::from_value::<CanonicalToolMeta>(meta.clone()).is_ok_and(|meta| {
            meta.version == TOOL_META_VERSION && meta.kind == ToolKind::ActiveAgentMessage
        }),
        None => tool_call.title == SEND_SUBAGENT_MESSAGE_TOOL_NAME,
    }
}

pub(super) fn to_block(tool_call: &acp::ToolCall, labels: &SubagentLabelRegistry) -> RenderBlock {
    let input = tool_call
        .raw_input
        .as_ref()
        .and_then(|input| SentMessageWireInput::deserialize(input).ok());
    let output =
        tool_call
            .raw_output
            .clone()
            .and_then(
                |output| match serde_json::from_value::<ToolOutput>(output).ok()? {
                    ToolOutput::SendSubagentMessage(output) => Some(output),
                    _ => None,
                },
            );
    let presentation = presentation(tool_call, output);
    let input = input.map(|input| {
        let delivery = input.recognized_delivery();
        let target = match input.subagent_id.as_deref() {
            Some(PARENT_TARGET) => SentMessageTarget::Parent,
            // Trimmed once here so the noun, the expanded id line, and the search index agree on one value.
            Some(subagent_id) => labels.resolve(subagent_id.trim().to_owned()),
            None => SentMessageTarget::Unresolved {
                subagent_id: String::new(),
            },
        };
        SentMessageInput {
            target,
            delivery,
            text: input.text,
        }
    });

    RenderBlock::ToolCall(ToolCallBlock::SentMessage(SentMessageToolCallBlock::new(
        presentation,
        input,
    )))
}

fn presentation(
    tool_call: &acp::ToolCall,
    output: Option<SendSubagentMessageOutput>,
) -> SentMessagePresentation {
    if !is_terminal(tool_call.status) {
        return SentMessagePresentation::Sending;
    }
    match output {
        Some(output) => match output.disposition() {
            SendSubagentMessageDisposition::Accepted => SentMessagePresentation::Sent,
            SendSubagentMessageDisposition::Rejected => SentMessagePresentation::Rejected {
                reason: output.to_string(),
            },
            SendSubagentMessageDisposition::Unconfirmed => SentMessagePresentation::Unconfirmed {
                reason: output.to_string(),
            },
        },
        None => SentMessagePresentation::Rejected {
            reason: content_text(tool_call).unwrap_or_else(|| {
                "Message was not accepted or delivery details are unavailable.".to_owned()
            }),
        },
    }
}

fn is_terminal(status: acp::ToolCallStatus) -> bool {
    matches!(
        status,
        acp::ToolCallStatus::Completed | acp::ToolCallStatus::Failed
    )
}

fn content_text(tool_call: &acp::ToolCall) -> Option<String> {
    let text = tool_call
        .content
        .iter()
        .filter_map(|content| match content {
            acp::ToolCallContent::Content(acp::Content {
                content: acp::ContentBlock::Text(text),
                ..
            }) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
#[path = "subagent_message_tests.rs"]
mod tests;

#[cfg(test)]
mod recovery_presentation_tests {
    use super::*;
    use xai_grok_tools::implementations::grok_build::task::{native_recovery::MessageRecoveryRef, root_control::AgentMessageGeneration, types::{ActiveAgentMessageOperation, ActiveMessageReceiver}};

    #[test]
    fn unconfirmed_native_output_preserves_exact_observe_reference_in_the_real_block() {
        let reference = MessageRecoveryRef {
            delivery_id: "parent-message/exact-message".into(), message_id: "exact-message".into(),
            prompt_id: "parent-message-exact-message".into(), payload_digest: "ab".repeat(32),
            receiver: ActiveMessageReceiver { session_id: "exact-receiver".into(), generation: AgentMessageGeneration::mint(42) },
            operation: ActiveAgentMessageOperation::Queue,
        };
        let wire = serde_json::to_string(&reference).unwrap();
        let call = acp::ToolCall::new("native-message", SEND_SUBAGENT_MESSAGE_TOOL_NAME)
            .status(acp::ToolCallStatus::Completed)
            .raw_input(Some(serde_json::json!({"subagent_id":"exact-receiver","text":"literal message"})))
            .raw_output(Some(serde_json::to_value(ToolOutput::SendSubagentMessage(
                SendSubagentMessageOutput::AdmissionUncertain { recovery_ref: reference.into() },
            )).unwrap()));
        let RenderBlock::ToolCall(ToolCallBlock::SentMessage(block)) = to_block(&call, &SubagentLabelRegistry::default()) else {
            panic!("native message block");
        };
        let SentMessagePresentation::Unconfirmed { reason } = block.presentation else { panic!("uncertainty must not become rejection"); };
        assert!(reason.contains(&wire));
        assert!(reason.contains("Observe"));
        assert!(!reason.to_lowercase().contains("resend"));
    }
    #[test]
    fn stored_uncorrelated_output_is_not_relabelled_as_a_rejection() {
        let call = acp::ToolCall::new("historical-message", SEND_SUBAGENT_MESSAGE_TOOL_NAME)
            .status(acp::ToolCallStatus::Completed)
            .raw_input(Some(serde_json::json!({"subagent_id":"receiver","text":"old literal message"})))
            .raw_output(Some(serde_json::json!({"type":"SendSubagentMessage","outcome":"admission_uncertain"})));
        let RenderBlock::ToolCall(ToolCallBlock::SentMessage(block)) = to_block(&call, &SubagentLabelRegistry::default()) else { panic!("stored message block"); };
        assert!(matches!(block.presentation, SentMessagePresentation::Unconfirmed { reason }
            if reason.contains("no recovery reference was recorded") && !reason.to_lowercase().contains("resend")));
    }

}
