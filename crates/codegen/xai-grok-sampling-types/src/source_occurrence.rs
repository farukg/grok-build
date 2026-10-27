//! Local provenance; never part of a provider request.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceOccurrenceId(pub [u8; 16]);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SourceOccurrence {
    #[default]
    Unadmitted,
    Admitted { id: SourceOccurrenceId },
    /// Normalized identity-less incoming history, without evidence of historical request membership.
    LegacyNormalized { id: SourceOccurrenceId },
}

impl SourceOccurrence {
    pub fn is_unadmitted(&self) -> bool {
        match self {
            Self::Unadmitted => true,
            Self::Admitted { .. } | Self::LegacyNormalized { .. } => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationReasoningItem {
    #[serde(flatten)]
    pub inner: crate::rs::ReasoningItem,
    #[serde(default, skip_serializing_if = "SourceOccurrence::is_unadmitted")]
    pub source: SourceOccurrence,
    #[serde(default, skip_serializing_if = "ContentEvidence::is_unknown")]
    pub evidence: ContentEvidence,
}

impl From<crate::rs::ReasoningItem> for ConversationReasoningItem {
    fn from(inner: crate::rs::ReasoningItem) -> Self {
        Self { inner, source: SourceOccurrence::Unadmitted, evidence: ContentEvidence::default() }
    }
}

impl std::ops::Deref for ConversationReasoningItem {
    type Target = crate::rs::ReasoningItem;
    fn deref(&self) -> &Self::Target { &self.inner }
}

impl std::ops::DerefMut for ConversationReasoningItem {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.inner }
}

/// Source contribution to projected history, not proof that its original body survived unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedRequestMember {
    pub source: SourceOccurrence,
    pub category: crate::ContextCategory,
    #[serde(default)]
    pub evidence: ContentEvidence,
}

impl crate::ConversationItem {
    pub fn source(&self) -> SourceOccurrence {
        match self {
            Self::System(item) => item.source,
            Self::User(item) => item.source,
            Self::Assistant(item) => item.source,
            Self::ToolResult(item) => item.source,
            Self::BackendToolCall(item) => item.source,
            Self::Reasoning(item) => item.source,
        }
    }

    pub fn source_mut(&mut self) -> &mut SourceOccurrence {
        match self {
            Self::System(item) => &mut item.source,
            Self::User(item) => &mut item.source,
            Self::Assistant(item) => &mut item.source,
            Self::ToolResult(item) => &mut item.source,
            Self::BackendToolCall(item) => &mut item.source,
            Self::Reasoning(item) => &mut item.source,
        }
    }
}

impl crate::ConversationRequest {
    pub fn projected_membership(&self) -> Vec<ProjectedRequestMember> {
        self.items.iter().map(|item| ProjectedRequestMember {
            source: item.source(),
            category: item.context_category(),
            evidence: item.evidence().clone(),
        }).collect()
    }
}

/// Evidence is independent of identity. Observed operations do not establish full original-body equality.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ContentEvidence {
    #[default]
    Unknown,
    Observed {
        transforms: Vec<ContentTransform>,
        compaction: CompactionCoverage,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContentTransform {
    ToolResultHardCleared,
    ToolResultSoftTrimmed,
    CompactionToolResultOmitted,
    /// Counts at the image-budget operation input, not at original admission.
    ImagesEvicted { kept: usize, total: usize },
    AgentLabelPrepended,
    /// Counts at this projection operation, not counts in the original admission body.
    PolicyParts { kept: usize, total: usize },
    AssistantTextWithheld,
    AssistantCallsWithheld,
    SystemHeadRewritten,
}

/// Replaced is the actual set difference at replacement, not a claim of summary fidelity.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CompactionCoverage {
    #[default]
    Unknown,
    AwaitingReplacement,
    Replaced {
        sources: Vec<SourceOccurrenceId>,
        inherited: Vec<SourceOccurrenceId>,
        unidentified: usize,
        ancestry: CoverageKnowledge,
    },
}

impl ContentEvidence {
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }

    pub fn transforms(&self) -> &[ContentTransform] {
        match self {
            Self::Unknown => &[],
            Self::Observed { transforms, .. } => transforms,
        }
    }

    pub fn compaction(&self) -> &CompactionCoverage {
        match self {
            Self::Unknown => &CompactionCoverage::Unknown,
            Self::Observed { compaction, .. } => compaction,
        }
    }

    pub fn set_compaction(&mut self, coverage: CompactionCoverage) {
        match self {
            Self::Unknown => *self = Self::Observed { transforms: Vec::new(), compaction: coverage },
            Self::Observed { compaction, .. } => *compaction = coverage,
        }
    }

    /// At most one fact per transform kind; independent facts coexist.
    pub fn record(&mut self, fact: ContentTransform) {
        match self {
            Self::Unknown => *self = Self::Observed {
                transforms: vec![fact], compaction: CompactionCoverage::Unknown,
            },
            Self::Observed { transforms, .. } => {
                if let Some(prior) = transforms.iter_mut().find(|prior|
                    std::mem::discriminant(&**prior) == std::mem::discriminant(&fact)) {
                    *prior = fact;
                } else {
                    transforms.push(fact);
                }
            }
        }
    }
}

impl crate::ConversationItem {
    pub fn evidence(&self) -> &ContentEvidence {
        match self {
            Self::System(item) => &item.evidence,
            Self::User(item) => &item.evidence,
            Self::Assistant(item) => &item.evidence,
            Self::ToolResult(item) => &item.evidence,
            Self::BackendToolCall(item) => &item.evidence,
            Self::Reasoning(item) => &item.evidence,
        }
    }

    pub fn evidence_mut(&mut self) -> &mut ContentEvidence {
        match self {
            Self::System(item) => &mut item.evidence,
            Self::User(item) => &mut item.evidence,
            Self::Assistant(item) => &mut item.evidence,
            Self::ToolResult(item) => &mut item.evidence,
            Self::BackendToolCall(item) => &mut item.evidence,
            Self::Reasoning(item) => &mut item.evidence,
        }
    }
}

impl SourceOccurrence {
    pub fn id(self) -> Option<SourceOccurrenceId> {
        match self {
            Self::Unadmitted => None,
            Self::Admitted { id } | Self::LegacyNormalized { id } => Some(id),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageKnowledge {
    Complete,
    Unknown,
}
