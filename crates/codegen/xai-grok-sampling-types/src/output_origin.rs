use serde::{Deserialize, Serialize};

#[cfg(test)]
#[path = "output_origin_tests.rs"]
mod tests;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reference", content = "id", rename_all = "snake_case")]
pub enum ProviderOutputRef {
    Item(String),
    ToolCall(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputChannel { Text, Reasoning }

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "granularity", rename_all = "snake_case")]
pub enum OutputOrigin {
    #[default]
    Unknown,
    Item { reference: ProviderOutputRef },
    Aggregate { references: Vec<ProviderOutputRef> },
    ChannelAggregate { channel: OutputChannel },
    MessageChannelAggregate { message_id: String, channel: OutputChannel },
}

impl OutputOrigin {
    pub fn is_unknown(&self) -> bool { matches!(self, Self::Unknown) }

    pub fn channel_aggregate(channel: OutputChannel, message_id: Option<&str>) -> Self {
        match message_id {
            Some(message_id) => Self::MessageChannelAggregate { message_id: message_id.to_owned(), channel },
            None => Self::ChannelAggregate { channel },
        }
    }

    pub fn from_references(mut references: Vec<ProviderOutputRef>) -> Self {
        match references.len() {
            0 => Self::Unknown,
            1 => match references.pop() {
                Some(reference) => Self::Item { reference },
                None => Self::Unknown,
            },
            _ => Self::Aggregate { references },
        }
    }
}

impl std::fmt::Debug for OutputOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("OutputOrigin(<opaque>)") }
}

impl std::fmt::Debug for ProviderOutputRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("ProviderOutputRef(<opaque>)") }
}

impl crate::ConversationItem {
    pub fn output_origin(&self) -> OutputOrigin {
        match self {
            Self::Assistant(item) => {
                if item.output_origin.is_unknown() {
                    let references = item.tool_calls.iter().map(|call| ProviderOutputRef::ToolCall(call.id.to_string())).collect();
                    OutputOrigin::from_references(references)
                } else { item.output_origin.clone() }
            }
            Self::Reasoning(item) => {
                if item.id.is_empty() { OutputOrigin::ChannelAggregate { channel: OutputChannel::Reasoning } }
                else { OutputOrigin::Item { reference: ProviderOutputRef::Item(item.id.clone()) } }
            },
            Self::BackendToolCall(item) => OutputOrigin::Item { reference: ProviderOutputRef::Item(item.id().to_owned()) },
            Self::ToolResult(item) => OutputOrigin::Item { reference: ProviderOutputRef::ToolCall(item.tool_call_id.to_string()) },
            Self::System(_) | Self::User(_) => OutputOrigin::Unknown,
        }
    }
}
