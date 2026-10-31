//! Provider locations recorded by the conversion that emits each instruction.

use serde::{Deserialize, Serialize};

use crate::{ApiBackend, ContentEvidence, ContextCategory, ConversationItem, SourceOccurrence};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "index", rename_all = "snake_case")]
pub enum InstructionContentPath {
    Whole,
    Block(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "field", rename_all = "snake_case")]
pub enum ProviderInstructionPath {
    MessagesSystem { index: usize },
    ChatMessage { index: usize, content: InstructionContentPath },
    ResponsesInput { index: usize, content: InstructionContentPath },
    MessagesContent { index: usize, block: usize },
    Tool { index: usize },
    ToolChoice,
    ChatResponseFormat,
    ChatSearchParameters,
    ResponsesTextFormat,
    MessagesOutputFormat,
}

impl ProviderInstructionPath {
    pub fn json_pointer(self) -> String {
        let content_pointer = |prefix: String, content| match content {
            InstructionContentPath::Whole => prefix,
            InstructionContentPath::Block(index) => format!("{prefix}/{index}"),
        };
        match self {
            Self::MessagesSystem { index } => format!("/system/{index}"),
            Self::ChatMessage { index, content } => content_pointer(format!("/messages/{index}/content"), content),
            Self::ResponsesInput { index, content } => content_pointer(format!("/input/{index}/content"), content),
            Self::MessagesContent { index, block } => format!("/messages/{index}/content/{block}"),
            Self::Tool { index } => format!("/tools/{index}"),
            Self::ToolChoice => "/tool_choice".into(),
            Self::ChatResponseFormat => "/response_format".into(),
            Self::ChatSearchParameters => "/search_parameters".into(),
            Self::ResponsesTextFormat => "/text/format".into(),
            Self::MessagesOutputFormat => "/output_config/format".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "selection", rename_all = "snake_case")]
pub enum InstructionSelection {
    Value,
    /// UTF-8 offsets in a provider string formed by joining source text parts.
    TextRange { start: usize, end: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionReference {
    pub path: ProviderInstructionPath,
    pub selection: InstructionSelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestCapability {
    Tool,
    ToolChoice,
    StructuredOutput,
    HostedSearch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationInstructionSource {
    pub source: SourceOccurrence,
    pub category: ContextCategory,
    pub evidence: ContentEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvertedInstruction {
    pub reference: InstructionReference,
    pub source: ConversationInstructionSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInstructionPlan {
    pub backend: ApiBackend,
    pub instructions: Vec<ConvertedInstruction>,
}

#[derive(Clone, Copy)]
pub(crate) enum InstructionContentEncoding { JoinedText, SingleText, Blocks }

impl ProviderInstructionPlan {
    pub fn new(backend: ApiBackend) -> Self {
        Self { backend, instructions: Vec::new() }
    }

    pub(crate) fn record_item(&mut self, item: &ConversationItem, path: ProviderInstructionPath, encoding: InstructionContentEncoding) {
        match item {
            ConversationItem::System(_) => self.record(item, item.context_category(), path, InstructionSelection::Value),
            ConversationItem::User(user) => {
                let mut offset = 0;
                for (index, (part, category)) in user.content.iter().zip(user.part_categories()).enumerate() {
                    let (part_path, selection) = if matches!(encoding, InstructionContentEncoding::JoinedText) {
                        let len = match part {
                            crate::ContentPart::Text { text } => text.len(),
                            crate::ContentPart::Image { .. } => 0,
                        };
                        let selection = InstructionSelection::TextRange { start: offset, end: offset + len };
                        offset += len + 1;
                        (path, selection)
                    } else {
                        let part_path = match path {
                            ProviderInstructionPath::ChatMessage { index: message, .. } => ProviderInstructionPath::ChatMessage {
                                index: message, content: InstructionContentPath::Block(index),
                            },
                            ProviderInstructionPath::ResponsesInput { index: message, .. } => ProviderInstructionPath::ResponsesInput {
                                index: message, content: match encoding { InstructionContentEncoding::SingleText => InstructionContentPath::Whole, InstructionContentEncoding::Blocks | InstructionContentEncoding::JoinedText => InstructionContentPath::Block(index) },
                            },
                            ProviderInstructionPath::MessagesContent { index: message, .. } => ProviderInstructionPath::MessagesContent { index: message, block: index },
                            ProviderInstructionPath::MessagesSystem { .. } | ProviderInstructionPath::Tool { .. }
                            | ProviderInstructionPath::ToolChoice | ProviderInstructionPath::ChatResponseFormat
                            | ProviderInstructionPath::ChatSearchParameters
                            | ProviderInstructionPath::ResponsesTextFormat | ProviderInstructionPath::MessagesOutputFormat => path,
                        };
                        (part_path, InstructionSelection::Value)
                    };
                    if instruction_category(category) {
                        self.record(item, category, part_path, selection);
                    }
                }
            }
            ConversationItem::Assistant(_) | ConversationItem::ToolResult(_)
            | ConversationItem::BackendToolCall(_) | ConversationItem::Reasoning(_) => {}
        }
    }

    pub(crate) fn record(
        &mut self, item: &ConversationItem, category: ContextCategory,
        path: ProviderInstructionPath, selection: InstructionSelection,
    ) {
        self.instructions.push(ConvertedInstruction {
            reference: InstructionReference { path, selection },
            source: ConversationInstructionSource {
                source: item.source(), category, evidence: item.evidence().clone(),
            },
        });
    }
}

pub(crate) fn instruction_category(category: ContextCategory) -> bool {
    match category {
        ContextCategory::CoreInstructions | ContextCategory::ProjectInstructions
        | ContextCategory::Environment | ContextCategory::SkillsWorkflows
        | ContextCategory::McpCatalog | ContextCategory::ToolDefinitions => true,
        ContextCategory::UserTurns | ContextCategory::AssistantTurns | ContextCategory::Reasoning
        | ContextCategory::ToolExchanges | ContextCategory::RuntimeNotices(_)
        | ContextCategory::Memory | ContextCategory::CompactionSummary | ContextCategory::OtherRuntime => false,
    }
}
