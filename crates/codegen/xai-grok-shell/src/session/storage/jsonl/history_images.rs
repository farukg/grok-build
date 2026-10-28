use crate::sampling::{ContentPart, ConversationItem};

/// Max decoded size for a data-URI image loaded from persisted history.
/// Generous (20 MB): fresh images use 5 MB, but loaded ones just need sanity-checking.
pub(super) const MAX_LOADED_IMAGE_BYTES: usize = 20 * 1024 * 1024;

pub(super) struct ImageRejection {
    pub part: usize,
    cause: ImageRejectCause,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryImageIssue {
    Header,
    Size,
    Base64,
    Content,
}

enum ImageRejectCause {
    Header,
    Size,
    Base64,
    Content(String),
}

impl ImageRejection {
    pub(super) fn issue(&self) -> HistoryImageIssue {
        match &self.cause {
            ImageRejectCause::Header => HistoryImageIssue::Header,
            ImageRejectCause::Size => HistoryImageIssue::Size,
            ImageRejectCause::Base64 => HistoryImageIssue::Base64,
            ImageRejectCause::Content(_) => HistoryImageIssue::Content,
        }
    }
}

pub(super) fn invalid_images(item: &ConversationItem) -> Vec<ImageRejection> {
    image_rejections(item).collect()
}

pub(super) fn image_rejections(item: &ConversationItem) -> impl Iterator<Item = ImageRejection> + '_ {
    let parts: &[ContentPart] = match item {
        ConversationItem::User(user) => &user.content,
        ConversationItem::ToolResult(tool) => &tool.images,
        ConversationItem::System(_)
        | ConversationItem::Assistant(_)
        | ConversationItem::BackendToolCall(_)
        | ConversationItem::Reasoning(_) => &[],
    };
    parts.iter().enumerate().filter_map(|(part, content)| {
        match content {
            ContentPart::Image { url } if url.starts_with("data:") => {
                data_uri_reject_cause(url).map(|cause| ImageRejection { part, cause })
            }
            ContentPart::Image { .. } | ContentPart::Text { .. } => None,
        }
    })
}

fn data_uri_reject_cause(url: &str) -> Option<ImageRejectCause> {
    use base64::Engine as _;
    let Some(after_data) = url.strip_prefix("data:") else {
        return Some(ImageRejectCause::Header);
    };
    let Some((header, payload)) = after_data.split_once(',') else {
        return Some(ImageRejectCause::Header);
    };
    if !header.as_bytes().windows(7).any(|w| w.eq_ignore_ascii_case(b";base64")) {
        return Some(ImageRejectCause::Header);
    }
    if payload.len() * 3 / 4 > MAX_LOADED_IMAGE_BYTES {
        return Some(ImageRejectCause::Size);
    }
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(payload) else {
        return Some(ImageRejectCause::Base64);
    };
    crate::session::image_normalize::persisted_image_reject_reason(&bytes)
        .map(ImageRejectCause::Content)
}

pub(super) fn normalize_images(item: &mut ConversationItem, rejected: &[ImageRejection]) {
    for rejection in rejected {
        match &rejection.cause {
            ImageRejectCause::Content(reason) => {
                tracing::warn!(reason, "stripping unsendable image from loaded history");
            }
            ImageRejectCause::Header | ImageRejectCause::Size | ImageRejectCause::Base64 => {}
        }
    }
    let mut rejected = rejected.iter().peekable();
    match item {
        ConversationItem::User(user) => {
            for (index, part) in user.content.iter_mut().enumerate() {
                if rejected.peek().is_some_and(|r| r.part == index) {
                    *part = ContentPart::Text {
                        text: std::sync::Arc::<str>::from("[image removed \u{2014} invalid data]"),
                    };
                    rejected.next();
                }
            }
        }
        ConversationItem::ToolResult(tool) => {
            let mut index = 0;
            tool.images.retain(|_| {
                let remove = rejected.peek().is_some_and(|r| r.part == index);
                index += 1;
                if remove {
                    rejected.next();
                }
                !remove
            });
        }
        ConversationItem::System(_)
        | ConversationItem::Assistant(_)
        | ConversationItem::BackendToolCall(_)
        | ConversationItem::Reasoning(_) => {}
    }
}

/// Strip data-URI images the API would reject from loaded conversation items, so a poisoned history recovers instead of 400ing on every turn.
/// The verdicts come from [`persisted_image_reject_reason`](crate::session::image_normalize::persisted_image_reject_reason).
/// User parts become a text placeholder; `ToolResultItem.images` entries are removed.
pub(crate) fn strip_invalid_images(items: &mut [ConversationItem]) -> usize {
    let mut stripped = 0;
    for item in items {
        let rejected = invalid_images(item);
        stripped += rejected.len();
        normalize_images(item, &rejected);
    }
    stripped
}
