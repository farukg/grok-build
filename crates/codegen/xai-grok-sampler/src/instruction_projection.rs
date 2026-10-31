//! Request-bound instruction references; the captured body remains the only byte owner.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use xai_grok_sampling_types::instruction_projection::{
    ConversationInstructionSource, ConvertedInstruction, InstructionReference, InstructionSelection,
    ProviderInstructionPath, ProviderInstructionPlan, RequestCapability,
};
use xai_grok_sampling_types::{ApiBackend, SourceOccurrence};

use crate::request_capture::{RequestAttemptId, redact_json};
use crate::RequestId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderInstructionRole { System, Developer, User }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "authority", rename_all = "snake_case")]
pub enum PreparedInstructionOrigin {
    Conversation { role: ProviderInstructionRole, provenance: ConversationInstructionSource },
    /// Authority is this final prepared request, never a conversation occurrence.
    ProviderRequest { capability: RequestCapability },
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum InstructionContentKind {
    Instruction { role: ProviderInstructionRole, category: xai_grok_sampling_types::ContextCategory },
    Capability { capability: RequestCapability },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedInstructionBinding {
    pub request_id: RequestId,
    pub attempt: RequestAttemptId,
    pub backend: ApiBackend,
    pub body_digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedInstructionReference {
    pub reference: InstructionReference,
    pub origin: PreparedInstructionOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "issue", rename_all = "snake_case")]
pub enum InstructionProjectionIssue {
    MissingConversionEvidence,
    BackendMismatch,
    MissingSource { reference: InstructionReference },
    InvalidReference { reference: InstructionReference },
    InvalidField { pointer: String },
    UnmappedInstruction { pointer: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionProjectionUnavailable { NotPrepared, MalformedJson, InvalidRoot }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "coverage", rename_all = "snake_case")]
pub enum InstructionProjectionEvidence<T> {
    Complete { projection: T },
    Partial { projection: T, issues: Vec<InstructionProjectionIssue> },
    Unavailable { reason: InstructionProjectionUnavailable },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "capture", rename_all = "snake_case")]
pub enum CapturedInstructions {
    Unavailable { reason: InstructionProjectionUnavailable },
    Prepared {
        binding: PreparedInstructionBinding,
        evidence: InstructionProjectionEvidence<Vec<PreparedInstructionReference>>,
    },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PublishedInstructionFragment {
    pub source: PreparedInstructionReference,
    pub value: Value,
}

impl std::fmt::Debug for PublishedInstructionFragment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublishedInstructionFragment").field("source", &self.source).finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "representation", rename_all = "snake_case")]
pub enum InstructionRepresentation {
    /// Exact JSON values, not byte slices of the original JSON document.
    ExactLocalValues,
    Redacted { original_body_digest: [u8; 32] },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "capture", rename_all = "snake_case")]
pub enum InstructionPublication {
    Unavailable { reason: InstructionProjectionUnavailable },
    Prepared {
        binding: PreparedInstructionBinding,
        evidence: InstructionProjectionEvidence<InstructionPublicationView>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionPublicationView {
    pub fragments: Vec<PublishedInstructionFragment>,
    pub representation: InstructionRepresentation,
    /// Equality of effective ordered values/roles, not logical occurrence identity.
    pub content_digest: [u8; 32],
}

impl CapturedInstructions {
    pub(crate) fn prepared(binding: PreparedInstructionBinding, bytes: &[u8], plan: Option<&ProviderInstructionPlan>) -> Self {
        let evidence = match serde_json::from_slice::<Value>(bytes) {
            Ok(value) if value.is_object() => project(&binding.backend, &value, plan),
            Ok(_) => InstructionProjectionEvidence::Unavailable { reason: InstructionProjectionUnavailable::InvalidRoot },
            Err(_) => InstructionProjectionEvidence::Unavailable { reason: InstructionProjectionUnavailable::MalformedJson },
        };
        Self::Prepared { binding, evidence }
    }

    pub(crate) fn publication(&self, bytes: &[u8]) -> Result<InstructionPublication, serde_json::Error> {
        let (binding, evidence) = match self {
            Self::Unavailable { reason } => return Ok(InstructionPublication::Unavailable { reason: *reason }),
            Self::Prepared { binding, evidence } => (binding, evidence),
        };
        let (references, mut issues) = match evidence {
            InstructionProjectionEvidence::Complete { projection } => (projection, Vec::new()),
            InstructionProjectionEvidence::Partial { projection, issues } => (projection, issues.clone()),
            InstructionProjectionEvidence::Unavailable { reason } => return Ok(InstructionPublication::Prepared {
                binding: binding.clone(), evidence: InstructionProjectionEvidence::Unavailable { reason: *reason },
            }),
        };
        let body: Value = serde_json::from_slice(bytes)?;
        let mut fragments = Vec::with_capacity(references.len());
        let mut digest_values = Vec::with_capacity(references.len());
        let mut redacted = false;
        for source in references {
            let Some(mut value) = selected_value(&body, &source.reference) else {
                issues.push(InstructionProjectionIssue::InvalidReference { reference: source.reference.clone() });
                continue;
            };
            let kind = match &source.origin {
                PreparedInstructionOrigin::Conversation { role, provenance } => InstructionContentKind::Instruction {
                    role: *role, category: provenance.category,
                },
                PreparedInstructionOrigin::ProviderRequest { capability } => InstructionContentKind::Capability { capability: *capability },
            };
            let mut content = value.clone();
            if matches!(&source.origin, PreparedInstructionOrigin::Conversation { .. })
                && let Some(block) = content.as_object_mut()
            {
                block.remove("cache_control");
            }
            digest_values.push((kind, content));
            redacted |= redact_json(&mut value);
            fragments.push(PublishedInstructionFragment { source: source.clone(), value });
        }
        let content_digest = *blake3::hash(&serde_json::to_vec(&digest_values)?).as_bytes();
        let projection = InstructionPublicationView {
            fragments, content_digest,
            representation: if redacted {
                InstructionRepresentation::Redacted { original_body_digest: binding.body_digest }
            } else { InstructionRepresentation::ExactLocalValues },
        };
        let evidence = if issues.is_empty() {
            InstructionProjectionEvidence::Complete { projection }
        } else {
            InstructionProjectionEvidence::Partial { projection, issues }
        };
        Ok(InstructionPublication::Prepared { binding: binding.clone(), evidence })
    }
}

fn selected_value(body: &Value, reference: &InstructionReference) -> Option<Value> {
    let value = body.pointer(&reference.path.json_pointer())?;
    match reference.selection {
        InstructionSelection::Value => (!value.is_null()).then(|| value.clone()),
        InstructionSelection::TextRange { start, end } => value.as_str()?.get(start..end).map(|text| Value::String(text.into())),
    }
}

fn instruction_value_valid(body: &Value, reference: &InstructionReference) -> bool {
    use xai_grok_sampling_types::instruction_projection::InstructionContentPath;
    let Some(value) = body.pointer(&reference.path.json_pointer()) else { return false; };
    let block_type = value.get("type").and_then(Value::as_str);
    match reference.path {
        ProviderInstructionPath::MessagesSystem { .. } => block_type == Some("text") && value.get("text").is_some_and(Value::is_string),
        ProviderInstructionPath::ChatMessage { content: InstructionContentPath::Whole, .. }
        | ProviderInstructionPath::ResponsesInput { content: InstructionContentPath::Whole, .. } => value.is_string(),
        ProviderInstructionPath::ChatMessage { content: InstructionContentPath::Block(_), .. } => matches!(block_type, Some("text" | "image_url")),
        ProviderInstructionPath::ResponsesInput { content: InstructionContentPath::Block(_), .. } => matches!(block_type, Some("input_text" | "input_image")),
        ProviderInstructionPath::MessagesContent { .. } => matches!(block_type, Some("text" | "image")),
        ProviderInstructionPath::Tool { .. } | ProviderInstructionPath::ToolChoice
        | ProviderInstructionPath::ChatResponseFormat | ProviderInstructionPath::ChatSearchParameters | ProviderInstructionPath::ResponsesTextFormat
        | ProviderInstructionPath::MessagesOutputFormat => false,
    }
}

fn instruction_role(backend: &ApiBackend, body: &Value, path: ProviderInstructionPath) -> Option<ProviderInstructionRole> {
    let role = match (backend, path) {
        (ApiBackend::Messages, ProviderInstructionPath::MessagesSystem { .. }) => return Some(ProviderInstructionRole::System),
        (ApiBackend::ChatCompletions, ProviderInstructionPath::ChatMessage { index, .. })
        | (ApiBackend::Messages, ProviderInstructionPath::MessagesContent { index, .. }) => body.get("messages")?.get(index)?.get("role")?.as_str()?,
        (ApiBackend::Responses, ProviderInstructionPath::ResponsesInput { index, .. }) => body.get("input")?.get(index)?.get("role")?.as_str()?,
        (ApiBackend::ChatCompletions | ApiBackend::Responses | ApiBackend::Messages,
         ProviderInstructionPath::MessagesSystem { .. } | ProviderInstructionPath::ChatMessage { .. }
         | ProviderInstructionPath::ResponsesInput { .. } | ProviderInstructionPath::MessagesContent { .. }
         | ProviderInstructionPath::Tool { .. } | ProviderInstructionPath::ToolChoice
         | ProviderInstructionPath::ChatResponseFormat | ProviderInstructionPath::ChatSearchParameters | ProviderInstructionPath::ResponsesTextFormat
         | ProviderInstructionPath::MessagesOutputFormat) => return None,
    };
    match role {
        "system" => Some(ProviderInstructionRole::System),
        "developer" => Some(ProviderInstructionRole::Developer),
        "user" => Some(ProviderInstructionRole::User),
        _ => None,
    }
}

fn project(backend: &ApiBackend, body: &Value, plan: Option<&ProviderInstructionPlan>) -> InstructionProjectionEvidence<Vec<PreparedInstructionReference>> {
    let mut fragments = Vec::new();
    let mut issues = Vec::new();
    let converted = match plan {
        Some(plan) if &plan.backend == backend => plan.instructions.as_slice(),
        Some(_) => { issues.push(InstructionProjectionIssue::BackendMismatch); &[] }
        None => { issues.push(InstructionProjectionIssue::MissingConversionEvidence); &[] }
    };
    for ConvertedInstruction { reference, source } in converted {
        let Some(role) = instruction_role(backend, body, reference.path) else {
            issues.push(InstructionProjectionIssue::InvalidReference { reference: reference.clone() });
            continue;
        };
        if !instruction_value_valid(body, reference) || selected_value(body, reference).is_none() {
            issues.push(InstructionProjectionIssue::InvalidReference { reference: reference.clone() });
            continue;
        }
        if matches!(source.source, SourceOccurrence::Unadmitted) {
            issues.push(InstructionProjectionIssue::MissingSource { reference: reference.clone() });
        }
        fragments.push(PreparedInstructionReference {
            reference: reference.clone(), origin: PreparedInstructionOrigin::Conversation { role, provenance: source.clone() },
        });
    }
    check_unmapped(backend, body, &fragments, &mut issues);
    if let Some(tools) = body.get("tools").filter(|value| !value.is_null()) {
        match tools.as_array() {
            Some(tools) => for (index, tool) in tools.iter().enumerate() {
                let tool_shape = match backend {
                    ApiBackend::ChatCompletions => tool.get("type").and_then(Value::as_str) == Some("function")
                        && tool.get("function").is_some_and(Value::is_object),
                    ApiBackend::Responses => tool.get("type").is_some_and(Value::is_string),
                    ApiBackend::Messages => tool.get("name").is_some_and(Value::is_string)
                        && tool.get("input_schema").is_some_and(Value::is_object),
                };
                if tool_shape {
                    capability(&mut fragments, ProviderInstructionPath::Tool { index }, RequestCapability::Tool);
                } else {
                    issues.push(InstructionProjectionIssue::InvalidField { pointer: format!("/tools/{index}") });
                }
            },
            None => issues.push(InstructionProjectionIssue::InvalidField { pointer: "/tools".into() }),
        }
    }
    for (path, kind) in [
        (ProviderInstructionPath::ToolChoice, RequestCapability::ToolChoice),
        (match backend {
            ApiBackend::ChatCompletions => ProviderInstructionPath::ChatResponseFormat,
            ApiBackend::Responses => ProviderInstructionPath::ResponsesTextFormat,
            ApiBackend::Messages => ProviderInstructionPath::MessagesOutputFormat,
        }, RequestCapability::StructuredOutput),
    ] {
        if let Some(value) = body.pointer(&path.json_pointer()).filter(|value| !value.is_null()) {
            let valid = match kind {
                RequestCapability::ToolChoice => value.is_string() || value.is_object(),
                RequestCapability::StructuredOutput | RequestCapability::HostedSearch => value.is_object(),
                RequestCapability::Tool => false,
            };
            if valid {
                capability(&mut fragments, path, kind);
            } else {
                issues.push(InstructionProjectionIssue::InvalidField { pointer: path.json_pointer() });
            }
        }
    }
    if matches!(backend, ApiBackend::ChatCompletions)
        && let Some(search) = body.get("search_parameters").filter(|value| !value.is_null())
    {
        if search.is_object() {
            capability(&mut fragments, ProviderInstructionPath::ChatSearchParameters, RequestCapability::HostedSearch);
        } else {
            issues.push(InstructionProjectionIssue::InvalidField { pointer: "/search_parameters".into() });
        }
    }
    if issues.is_empty() { InstructionProjectionEvidence::Complete { projection: fragments } }
    else { InstructionProjectionEvidence::Partial { projection: fragments, issues } }
}

fn capability(fragments: &mut Vec<PreparedInstructionReference>, path: ProviderInstructionPath, capability: RequestCapability) {
    fragments.push(PreparedInstructionReference {
        reference: InstructionReference { path, selection: InstructionSelection::Value },
        origin: PreparedInstructionOrigin::ProviderRequest { capability },
    });
}

fn check_unmapped(backend: &ApiBackend, body: &Value, fragments: &[PreparedInstructionReference], issues: &mut Vec<InstructionProjectionIssue>) {
    let conversation_field = match backend {
        ApiBackend::ChatCompletions | ApiBackend::Messages => "messages",
        ApiBackend::Responses => "input",
    };
    if !body.get(conversation_field).is_some_and(Value::is_array) {
        issues.push(InstructionProjectionIssue::InvalidField { pointer: format!("/{conversation_field}") });
    }
    let mut check = |pointer: String| {
        if !fragments.iter().any(|fragment| fragment.reference.path.json_pointer() == pointer) {
            issues.push(InstructionProjectionIssue::UnmappedInstruction { pointer });
        }
    };
    match backend {
        ApiBackend::ChatCompletions | ApiBackend::Responses => {
            let field = match backend {
                ApiBackend::ChatCompletions => "messages",
                ApiBackend::Responses => "input",
                ApiBackend::Messages => return,
            };
            if let Some(items) = body.get(field).and_then(Value::as_array) {
                for (index, item) in items.iter().enumerate() {
                    if matches!(item.get("role").and_then(Value::as_str), Some("system" | "developer")) {
                        check(format!("/{field}/{index}/content"));
                    }
                }
            }
            if matches!(backend, ApiBackend::Responses) && body.get("instructions").is_some_and(|value| !value.is_null()) {
                check("/instructions".into());
            }
        }
        ApiBackend::Messages => if let Some(system) = body.get("system").filter(|value| !value.is_null()) {
            match system.as_array() {
                Some(blocks) => for index in 0..blocks.len() { check(format!("/system/{index}")); },
                None => check("/system".into()),
            }
        },
    }
}

