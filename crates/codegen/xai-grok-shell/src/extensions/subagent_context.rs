//! The context a spawned subagent actually started with, carried on `SubagentSpawned` and in `meta.json`.

use std::fmt;

/// On the wire this stays the historic flat trio (`effective_context_source`, `context_normalized`, `fork_copy_error`), so replayed `updates.jsonl` and old `meta.json` keep parsing.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(from = "SubagentContextWire", into = "SubagentContextWire")]
pub enum SubagentContext {
    /// Written by a shell that did not record the source.
    Unreported,
    /// No fork was requested.
    Fresh,
    /// A fork was requested, but nothing could be inherited, so the child started fresh.
    ForkFailed(ForkFailure),
    Forked(ForkMode),
    /// Continues a completed peer subagent's transcript.
    Resumed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkMode {
    /// Parent items mirrored byte-for-byte.
    Verbatim,
    /// Parent history rendered into a `<background_context>` summary.
    Summarized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForkFailure {
    EmptyParent,
    NoInheritableContent,
    ParentUnavailable,
    CopyFailed(String),
}

impl SubagentContext {
    pub fn is_summarized_fork(&self) -> bool {
        matches!(self, Self::Forked(ForkMode::Summarized))
    }

    pub fn fork_failure(&self) -> Option<&ForkFailure> {
        match self {
            Self::ForkFailed(failure) => Some(failure),
            Self::Unreported | Self::Fresh | Self::Forked(_) | Self::Resumed => None,
        }
    }
}

impl fmt::Display for ForkFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyParent => f.write_str("empty parent conversation"),
            Self::NoInheritableContent => {
                f.write_str("no inheritable parent content after filtering")
            }
            Self::ParentUnavailable => f.write_str("parent conversation unavailable"),
            Self::CopyFailed(error) => f.write_str(error),
        }
    }
}

impl ForkFailure {
    /// Older shells wrote three spellings for the empty-after-filtering case.
    fn from_wire(reason: String) -> Self {
        match reason.as_str() {
            "empty parent conversation" => Self::EmptyParent,
            "no inheritable parent content after filtering"
            | "no inheritable parent content"
            | "forked parent conversation has no inheritable content" => {
                Self::NoInheritableContent
            }
            "parent conversation unavailable" => Self::ParentUnavailable,
            _ => Self::CopyFailed(reason),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SubagentContextWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    effective_context_source: Option<ContextSourceWire>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    context_normalized: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fork_copy_error: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum ContextSourceWire {
    New,
    Forked,
    Resumed,
    #[serde(other)]
    Unknown,
}

impl From<SubagentContextWire> for SubagentContext {
    fn from(wire: SubagentContextWire) -> Self {
        let SubagentContextWire {
            effective_context_source,
            context_normalized,
            fork_copy_error,
        } = wire;
        match (effective_context_source, fork_copy_error) {
            (None | Some(ContextSourceWire::Unknown), _) => Self::Unreported,
            (Some(ContextSourceWire::Resumed), _) => Self::Resumed,
            (Some(ContextSourceWire::Forked), _) if context_normalized => {
                Self::Forked(ForkMode::Summarized)
            }
            (Some(ContextSourceWire::Forked), _) => Self::Forked(ForkMode::Verbatim),
            (Some(ContextSourceWire::New), None) => Self::Fresh,
            (Some(ContextSourceWire::New), Some(reason)) => {
                Self::ForkFailed(ForkFailure::from_wire(reason))
            }
        }
    }
}

impl From<SubagentContext> for SubagentContextWire {
    fn from(context: SubagentContext) -> Self {
        let (source, fork_copy_error) = match &context {
            SubagentContext::Unreported => (None, None),
            SubagentContext::Fresh => (Some(ContextSourceWire::New), None),
            SubagentContext::ForkFailed(failure) => {
                (Some(ContextSourceWire::New), Some(failure.to_string()))
            }
            SubagentContext::Forked(_) => (Some(ContextSourceWire::Forked), None),
            SubagentContext::Resumed => (Some(ContextSourceWire::Resumed), None),
        };
        Self {
            effective_context_source: source,
            context_normalized: context.is_summarized_fork(),
            fork_copy_error,
        }
    }
}

#[cfg(test)]
#[path = "subagent_context_tests.rs"]
mod tests;
