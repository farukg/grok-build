use super::*;
use xai_chat_state::InputProjection;
use xai_chat_state::input_projection::{InputReceiptIdentity, ReceiptSources, RetainedInputSource};

#[derive(Clone, Copy)]
enum ReceiptScope { Completions, Notifications }

pub(in crate::session::storage::jsonl) struct CompletionHistoryProjection {
    pub(in crate::session::storage::jsonl) identity: InputReceiptIdentity,
    pub(in crate::session::storage::jsonl) source: xai_grok_sampling_types::SourceOccurrence,
    receipts: Vec<xai_chat_state::CompletionReceipt>,
}

impl JsonlStorageAdapter {
    pub(in crate::session::storage::jsonl) fn read_completion_projections(path: &Path) -> io::Result<Vec<CompletionHistoryProjection>> {
        Self::read_receipt_projections(path, ReceiptScope::Completions)
    }

    pub(in crate::session::storage::jsonl) fn read_notification_projections(path: &Path) -> io::Result<Vec<CompletionHistoryProjection>> {
        Self::read_receipt_projections(path, ReceiptScope::Notifications)
    }

    fn read_receipt_projections(path: &Path, scope: ReceiptScope) -> io::Result<Vec<CompletionHistoryProjection>> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut projections = Vec::new();
        for line in bytes.split_inclusive(|byte| *byte == b'\n') {
            if !line.ends_with(b"\n") { break; }
            if line.trim_ascii().is_empty() { continue; }
            let value: serde_json::Value = serde_json::from_slice(line).map_err(io::Error::other)?;
            let actual = if value.get("_notification_batch").is_some() {
                InputProjection::Notification(serde_json::from_value(value).map_err(io::Error::other)?)
            } else if value.get("_subagent_completion").is_some() && matches!(scope, ReceiptScope::Completions) {
                InputProjection::Subagent(serde_json::from_value(value).map_err(io::Error::other)?)
            } else { continue; };
            let identity = actual.receipt_identity();
            let source = actual.item().source();
            let receipts = match actual {
                InputProjection::Subagent(projection) => std::iter::once(projection.receipt).chain(projection.additional_receipts).collect(),
                InputProjection::Notification(projection) => projection.receipts,
                InputProjection::Delivery(_) => Vec::new(),
            };
            projections.push(CompletionHistoryProjection { identity, source, receipts });
        }
        Ok(projections)
    }

    pub(in crate::session::storage::jsonl) fn projection_receipts(projections: &[CompletionHistoryProjection]) -> Vec<xai_chat_state::CompletionReceipt> {
        projections.iter().flat_map(|projection| projection.receipts.iter().cloned()).collect()
    }

    pub(in crate::session::storage::jsonl) fn retained_completion_sources(projections: &[CompletionHistoryProjection], expected: InputReceiptIdentity) -> ReceiptSources {
        let mut retained = Vec::new();
        for actual in projections {
            let matches = match &expected {
                InputReceiptIdentity::Completion { completions } | InputReceiptIdentity::CompletionToolResult { completions, .. } => completions.iter().any(|expected| actual.receipts.iter().any(|receipt| &receipt.completion == expected)),
                InputReceiptIdentity::Notification { batch_id, .. } => match &actual.identity {
                    InputReceiptIdentity::Notification { batch_id: actual, .. } => actual == batch_id,
                    InputReceiptIdentity::Completion { .. } | InputReceiptIdentity::CompletionToolResult { .. } | InputReceiptIdentity::Delivery { .. } => false,
                },
                InputReceiptIdentity::Delivery { .. } => false,
            };
            if matches { retained.push(RetainedInputSource { identity: actual.identity.clone(), source: actual.source }); }
        }
        let missing = match expected {
            InputReceiptIdentity::Completion { completions } => {
                let completions = Self::missing_completion_sources(completions, &retained);
                if completions.is_empty() { Vec::new() } else { vec![InputReceiptIdentity::Completion { completions }] }
            }
            InputReceiptIdentity::CompletionToolResult { completions, tool_call_id } => {
                let completions = Self::missing_completion_sources(completions, &retained);
                if completions.is_empty() { Vec::new() } else { vec![InputReceiptIdentity::CompletionToolResult { completions, tool_call_id }] }
            }
            identity @ (InputReceiptIdentity::Notification { .. } | InputReceiptIdentity::Delivery { .. }) => {
                if retained.is_empty() { vec![identity] } else { Vec::new() }
            }
        };
        ReceiptSources { retained, missing }
    }

    fn missing_completion_sources(
        expected: Vec<xai_chat_state::completion::CompletionIdentity>, retained: &[RetainedInputSource],
    ) -> Vec<xai_chat_state::completion::CompletionIdentity> {
        expected.into_iter().filter(|expected| !retained.iter().any(|retained| match &retained.identity {
            InputReceiptIdentity::Completion { completions } | InputReceiptIdentity::CompletionToolResult { completions, .. }
            | InputReceiptIdentity::Notification { completions, .. } => completions.contains(expected),
            InputReceiptIdentity::Delivery { .. } => false,
        })).collect()
    }
}
