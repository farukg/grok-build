pub use super::history_images::HistoryImageIssue;
use super::history_images::{ImageRejection, image_rejections, normalize_images};
use crate::sampling::ConversationItem;
use crate::sampling::types::ChatRequestMessage;
use crate::session::persistence::CHAT_FORMAT_VERSION;
use std::collections::HashSet;
pub use xai_grok_sampling_types::LegacyReasoningIssue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryIssue {
    InvalidJson {
        line: usize,
    },
    InvalidRecord {
        line: usize,
    },
    InvalidLegacyReasoning {
        line: usize,
        cause: LegacyReasoningIssue,
    },
    InvalidImage {
        line: usize,
        part: usize,
        cause: HistoryImageIssue,
    },
}

struct DroppedLine {
    line: usize,
    cause: LineRejectCause,
}

enum LineRejectCause {
    Json(serde_json::Error),
    Record(serde_json::Error),
}

struct RejectedImages {
    line: usize,
    item: usize,
    rejected: Vec<ImageRejection>,
}

pub struct DecodedHistory {
    pub(super) items: Vec<ConversationItem>,
    pub(super) interjections: Result<xai_chat_state::interjection::InterjectionIndex, xai_chat_state::interjection::ReceiptError>,
    dropped: Vec<DroppedLine>,
    images: Vec<RejectedImages>,
    legacy_issues: Vec<HistoryIssue>,
    pub(super) upgraded_reasoning_count: usize,
    pub(super) upgraded_btc_count: usize,
}

impl DecodedHistory {
    pub fn interjections(&self) -> Result<&xai_chat_state::interjection::InterjectionIndex, xai_chat_state::interjection::ReceiptError> {
        self.interjections.as_ref().map_err(|error| *error)
    }

    pub fn items(&self) -> &[ConversationItem] {
        &self.items
    }

    /// Categorical source locations only; payload-bearing diagnostics stay private to normal loading.
    pub fn issues(&self) -> impl Iterator<Item = HistoryIssue> + '_ {
        self.dropped
            .iter()
            .map(|dropped| match &dropped.cause {
                LineRejectCause::Json(_) => HistoryIssue::InvalidJson { line: dropped.line },
                LineRejectCause::Record(_) => HistoryIssue::InvalidRecord { line: dropped.line },
            })
            .chain(self.images.iter().flat_map(|images| {
                images
                    .rejected
                    .iter()
                    .map(|image| HistoryIssue::InvalidImage {
                        line: images.line,
                        part: image.part,
                        cause: image.issue(),
                    })
            }))
            .chain(self.legacy_issues.iter().copied())
    }

    pub(super) fn skipped_lines(&self) -> usize {
        self.dropped.len()
    }

    pub(super) fn first_skipped(&self) -> Option<(usize, &serde_json::Error)> {
        self.dropped.first().map(|dropped| {
            let error = match &dropped.cause {
                LineRejectCause::Json(error) | LineRejectCause::Record(error) => error,
            };
            (dropped.line, error)
        })
    }

    pub(super) fn normalize_images(&mut self) -> usize {
        let mut stripped = 0;
        for images in &self.images {
            if let Some(item) = self.items.get_mut(images.item) {
                normalize_images(item, &images.rejected);
                stripped += images.rejected.len();
            }
        }
        stripped
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryItemLimitExceeded {
    pub line: usize,
    pub max_items: usize,
}

/// Decode persisted history without I/O, logging, quarantine or image replacement.
/// Empty lines and version-dependent fallback order match native tolerant loading.
/// Image part offsets are zero-based; line numbers are one-based.
pub fn decode_history(
    contents: &[u8],
    chat_format_version: u8,
    max_items: usize,
) -> Result<DecodedHistory, HistoryItemLimitExceeded> {
    decode_history_bounded(
        contents,
        chat_format_version,
        HistoryLimits {
            items: max_items,
            issues: usize::MAX,
        },
    )
    .map_err(|limit| match limit {
        HistoryDecodeLimit::Items {
            limit,
            observed_issues: _,
        } => limit,
        HistoryDecodeLimit::Issues { line, observed: _ } => {
            HistoryItemLimitExceeded { line, max_items }
        }
    })
}

#[derive(Clone, Copy)]
pub struct HistoryLimits {
    pub items: usize,
    pub issues: usize,
}

#[derive(Debug)]
pub enum HistoryDecodeLimit {
    Items {
        limit: HistoryItemLimitExceeded,
        observed_issues: usize,
    },
    Issues {
        line: usize,
        observed: usize,
    },
}

struct Diagnostics {
    observed: usize,
    limit: usize,
}

impl Diagnostics {
    fn issue(&mut self, line: usize) -> Result<(), HistoryDecodeLimit> {
        self.observed += 1;
        if self.observed > self.limit {
            return Err(HistoryDecodeLimit::Issues {
                line,
                observed: self.observed,
            });
        }
        Ok(())
    }
}

pub fn decode_history_bounded(
    contents: &[u8],
    chat_format_version: u8,
    limits: HistoryLimits,
) -> Result<DecodedHistory, HistoryDecodeLimit> {
    decode_history_with_interjections(contents, chat_format_version, limits, Default::default())
}

pub(super) fn decode_history_with_interjections(
    contents: &[u8],
    chat_format_version: u8,
    limits: HistoryLimits,
    interjections: xai_chat_state::interjection::InterjectionIndex,
) -> Result<DecodedHistory, HistoryDecodeLimit> {
    let max_items = limits.items;
    let mut diagnostics = Diagnostics {
        observed: 0,
        limit: limits.issues,
    };
    let mut sibling_btc_ids_seen = HashSet::new();
    let mut decoded = DecodedHistory {
        items: Vec::new(),
        interjections: Ok(interjections),
        dropped: Vec::new(),
        images: Vec::new(),
        legacy_issues: Vec::new(),
        upgraded_reasoning_count: 0,
        upgraded_btc_count: 0,
    };
    for (line_idx, line) in contents.split(|b| *b == b'\n').enumerate() {
        let line = line.trim_ascii();
        if line.is_empty() {
            continue;
        }
        let mut raw: serde_json::Value = match serde_json::from_slice(line) {
            Ok(raw) => raw,
            Err(error) => {
                diagnostics.issue(line_idx + 1)?;
                decoded.dropped.push(DroppedLine {
                    line: line_idx + 1,
                    cause: LineRejectCause::Json(error),
                });
                continue;
            }
        };
        if raw.get("interjection_projection").is_some() {
            let projection = serde_json::from_value::<super::interjection::ProjectionRecord>(raw.clone())
                .map_err(|_| xai_chat_state::interjection::ReceiptError::InvalidRecord)
                .and_then(|record| record.interjection_projection.into_parts());
            match projection {
                Ok((receipt, items)) => {
                    use xai_chat_state::interjection::ReceiptRestore;
                    let restored = match &mut decoded.interjections {
                        Ok(index) => index.restore(receipt),
                        Err(_) => continue,
                    };
                    match restored {
                        Ok(ReceiptRestore::New | ReceiptRestore::Transition) => {}
                        Ok(ReceiptRestore::AlreadyPresent) => continue,
                        Err(error) => { decoded.interjections = Err(error); continue; }
                    }
                    let Some(item) = items.into_iter().next() else { continue; };
                    match serde_json::to_value(item) {
                        Ok(item) => raw = item,
                        Err(_) => { decoded.interjections = Err(xai_chat_state::interjection::ReceiptError::InvalidRecord); continue; }
                    }
                }
                Err(error) => { decoded.interjections = Err(error); continue; }
            }
        }
        if raw.get("interjection_receipt").is_some() {
            let receipt = serde_json::from_value::<super::interjection::ReceiptRecord>(raw);
            if let Ok(index) = &mut decoded.interjections {
                match receipt {
                    Ok(record) => {
                        if let Err(error) = index.restore(record.interjection_receipt) {
                            decoded.interjections = Err(error);
                        }
                    }
                    Err(_) => decoded.interjections = Err(xai_chat_state::interjection::ReceiptError::InvalidRecord),
                }
            }
            continue;
        }
        let item_result = if chat_format_version >= CHAT_FORMAT_VERSION {
            serde_json::from_value::<ConversationItem>(raw.clone()).or_else(|e| {
                serde_json::from_value::<ChatRequestMessage>(raw.clone())
                    .map(ConversationItem::from)
                    .map_err(|_| e)
            })
        } else {
            serde_json::from_value::<ChatRequestMessage>(raw.clone())
                .map(ConversationItem::from)
                .or_else(|e| serde_json::from_value::<ConversationItem>(raw.clone()).map_err(|_| e))
        };
        let item = match item_result {
            Ok(item) => item,
            Err(error) => {
                diagnostics.issue(line_idx + 1)?;
                decoded.dropped.push(DroppedLine {
                    line: line_idx + 1,
                    cause: LineRejectCause::Record(error),
                });
                continue;
            }
        };
        // Bound the upgrader's sibling/ID allocations before calling its shared implementation.
        if limits.issues != usize::MAX
            && let Some(candidates) = raw.get("raw_output").and_then(serde_json::Value::as_array)
            && candidates.len()
                > max_items
                    .saturating_sub(decoded.items.len())
                    .saturating_sub(1)
        {
            return Err(HistoryDecodeLimit::Items {
                limit: HistoryItemLimitExceeded {
                    line: line_idx + 1,
                    max_items,
                },
                observed_issues: diagnostics.observed,
            });
        }
        let mut report_limit = None;
        let siblings = xai_grok_sampling_types::upgrade_legacy_reasoning_reporting(
            &raw,
            &mut sibling_btc_ids_seen,
            |cause| {
                if report_limit.is_none() {
                    match diagnostics.issue(line_idx + 1) {
                        Ok(()) => {
                            decoded
                                .legacy_issues
                                .push(HistoryIssue::InvalidLegacyReasoning {
                                    line: line_idx + 1,
                                    cause,
                                })
                        }
                        Err(limit) => {
                            report_limit = Some(limit);
                        }
                    }
                }
            },
        );
        if let Some(limit) = report_limit {
            return Err(limit);
        }
        for sibling in siblings {
            match &sibling {
                ConversationItem::Reasoning(_) => decoded.upgraded_reasoning_count += 1,
                ConversationItem::BackendToolCall(_) => decoded.upgraded_btc_count += 1,
                ConversationItem::System(_)
                | ConversationItem::User(_)
                | ConversationItem::Assistant(_)
                | ConversationItem::ToolResult(_) => {}
            }
            if decoded.items.len() >= max_items {
                return Err(HistoryDecodeLimit::Items {
                    limit: HistoryItemLimitExceeded {
                        line: line_idx + 1,
                        max_items,
                    },
                    observed_issues: diagnostics.observed,
                });
            }
            decoded.items.push(sibling);
        }
        if decoded.items.len() >= max_items {
            return Err(HistoryDecodeLimit::Items {
                limit: HistoryItemLimitExceeded {
                    line: line_idx + 1,
                    max_items,
                },
                observed_issues: diagnostics.observed,
            });
        }
        if let ConversationItem::BackendToolCall(backend) = &item {
            sibling_btc_ids_seen.insert(backend.id().to_string());
        }
        let item_index = decoded.items.len();
        decoded.items.push(item);
        if let Some(item) = decoded.items.last() {
            let mut rejected = Vec::new();
            for rejection in image_rejections(item) {
                diagnostics.issue(line_idx + 1)?;
                rejected.push(rejection);
            }
            if !rejected.is_empty() {
                decoded.images.push(RejectedImages {
                    line: line_idx + 1,
                    item: item_index,
                    rejected,
                });
            }
        }
    }
    Ok(decoded)
}
