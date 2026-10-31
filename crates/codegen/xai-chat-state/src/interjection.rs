//! Session-bound original interjection admissions, independent of retained model input.
use std::collections::HashMap;
use agent_client_protocol::ImageContent;
use serde::{Deserialize, Serialize};
use xai_grok_sampling_types::{InterjectionIdentity, SourceOccurrence};

#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "ReceiptWire")]
pub struct InterjectionReceipt {
    identity: InterjectionIdentity,
    images: Vec<ImageContent>,
    source: SourceOccurrence,
    disposition: InterjectionDisposition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterjectionDisposition { Pending, Projected, Retired }

#[derive(Deserialize)]
struct ReceiptWire {
    identity: InterjectionIdentity,
    images: Vec<ImageContent>,
    source: SourceOccurrence,
    disposition: InterjectionDisposition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptError { InvalidRecord, InvalidIdentity, InvalidSource, Conflict, InvalidTransition }
impl std::fmt::Display for ReceiptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "interjection receipt: {self:?}") }
}
impl std::error::Error for ReceiptError {}

impl TryFrom<ReceiptWire> for InterjectionReceipt {
    type Error = ReceiptError;
    fn try_from(wire: ReceiptWire) -> Result<Self, Self::Error> {
        let mut receipt = Self::new(wire.identity, wire.images, wire.source)?;
        receipt.disposition = wire.disposition;
        Ok(receipt)
    }
}

impl InterjectionReceipt {
    pub fn new(identity: InterjectionIdentity, images: Vec<ImageContent>, source: SourceOccurrence) -> Result<Self, ReceiptError> {
        if identity.id.is_empty() { return Err(ReceiptError::InvalidIdentity); }
        match source {
            SourceOccurrence::Admitted { .. } => {}
            SourceOccurrence::Unadmitted | SourceOccurrence::LegacyNormalized { .. } => return Err(ReceiptError::InvalidSource),
        }
        Ok(Self { identity, images, source, disposition: InterjectionDisposition::Pending })
    }
    pub fn identity(&self) -> &InterjectionIdentity { &self.identity }
    pub fn images(&self) -> &[ImageContent] { &self.images }
    pub fn source(&self) -> SourceOccurrence { self.source }
    pub fn disposition(&self) -> InterjectionDisposition { self.disposition }
    pub fn with_disposition(&self, disposition: InterjectionDisposition) -> Result<Self, ReceiptError> {
        match (self.disposition, disposition) {
            (InterjectionDisposition::Pending, InterjectionDisposition::Pending | InterjectionDisposition::Projected | InterjectionDisposition::Retired)
            | (InterjectionDisposition::Projected, InterjectionDisposition::Projected | InterjectionDisposition::Retired)
            | (InterjectionDisposition::Retired, InterjectionDisposition::Retired) => {}
            (InterjectionDisposition::Projected | InterjectionDisposition::Retired, InterjectionDisposition::Pending)
            | (InterjectionDisposition::Retired, InterjectionDisposition::Projected) => return Err(ReceiptError::InvalidTransition),
        }
        let mut receipt = self.clone();
        receipt.disposition = disposition;
        Ok(receipt)
    }
    pub fn validate_projection(&self, item: &xai_grok_sampling_types::ConversationItem) -> Result<(), ReceiptError> {
        match item {
            xai_grok_sampling_types::ConversationItem::User(user) if user.source == self.source && user.interjection.as_ref() == Some(&self.identity) => Ok(()),
            xai_grok_sampling_types::ConversationItem::User(_) | xai_grok_sampling_types::ConversationItem::System(_) | xai_grok_sampling_types::ConversationItem::Assistant(_) | xai_grok_sampling_types::ConversationItem::ToolResult(_) | xai_grok_sampling_types::ConversationItem::BackendToolCall(_) | xai_grok_sampling_types::ConversationItem::Reasoning(_) => Err(ReceiptError::InvalidSource),
        }
    }
    pub fn same_request(&self, other: &Self) -> bool {
        self.identity == other.identity && self.images == other.images
    }
}

#[derive(Clone, Default)]
pub struct InterjectionIndex {
    receipts: HashMap<String, InterjectionReceipt>,
}

pub enum ReceiptDecision<'a> { New, AlreadyPresent(&'a InterjectionReceipt), Transition }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptRestore { New, Transition, AlreadyPresent }

impl InterjectionIndex {
    pub fn receipts(&self) -> impl Iterator<Item = &InterjectionReceipt> { self.receipts.values() }
    pub fn get(&self, id: &str) -> Option<&InterjectionReceipt> { self.receipts.get(id) }
    pub fn classify(&self, receipt: &InterjectionReceipt) -> Result<ReceiptDecision<'_>, ReceiptError> {
        if let Some(original) = self.get(&receipt.identity.id) {
            if !original.same_request(receipt) || original.source != receipt.source { return Err(ReceiptError::Conflict); }
            if original.disposition == receipt.disposition { return Ok(ReceiptDecision::AlreadyPresent(original)); }
            original.with_disposition(receipt.disposition)?;
            return Ok(ReceiptDecision::Transition);
        }
        Ok(ReceiptDecision::New)
    }
    pub fn restore(&mut self, receipt: InterjectionReceipt) -> Result<ReceiptRestore, ReceiptError> {
        let restored = match self.classify(&receipt)? {
            ReceiptDecision::AlreadyPresent(_) => return Ok(ReceiptRestore::AlreadyPresent),
            ReceiptDecision::New => ReceiptRestore::New,
            ReceiptDecision::Transition => ReceiptRestore::Transition,
        };
        self.receipts.insert(receipt.identity.id.clone(), receipt);
        Ok(restored)
    }
}
