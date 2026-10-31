use super::{AppendDurability, AppendLineError, JsonlStorageAdapter, history_decoder};
use crate::sampling::ConversationItem;
use crate::session::info::Info;
use crate::session::persistence::CHAT_FORMAT_VERSION;
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Seek};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use xai_chat_state::interjection::{InterjectionIndex, InterjectionReceipt, ReceiptError};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReceiptRecord {
    pub(super) interjection_receipt: InterjectionReceipt,
}

#[derive(Serialize, Deserialize)]
pub(super) struct ProjectionRecord {
    pub(super) interjection_projection: InterjectionProjection,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "disposition", rename_all = "snake_case")]
pub enum InterjectionProjection {
    Projected { receipt: InterjectionReceipt, item: ConversationItem },
    Retired { receipt: InterjectionReceipt },
}

impl InterjectionProjection {
    pub(super) fn into_parts(self) -> Result<(InterjectionReceipt, Vec<ConversationItem>), ReceiptError> {
        use xai_chat_state::interjection::InterjectionDisposition;
        match self {
            Self::Projected { receipt, item } => {
                receipt.validate_projection(&item)?;
                if receipt.disposition() != InterjectionDisposition::Projected { return Err(ReceiptError::InvalidTransition); }
                Ok((receipt, vec![item]))
            }
            Self::Retired { receipt } => {
                if receipt.disposition() != InterjectionDisposition::Retired { return Err(ReceiptError::InvalidTransition); }
                Ok((receipt, Vec::new()))
            }
        }
    }
}

pub struct InterjectionStorage {
    path: PathBuf,
    session_id: String,
    cursor: Arc<Mutex<InterjectionCursor>>,
}

struct InterjectionCursor {
    path: PathBuf,
    index: InterjectionIndex,
    history: InterjectionHistory,
}

enum InterjectionHistory {
    Missing,
    Open { file: std::fs::File, offset: u64 },
}

pub enum InterjectionAppend {
    Appended(InterjectionReceipt),
    AlreadyPresent(InterjectionReceipt),
    Conflict(InterjectionReceipt),
}

#[derive(Debug)]
pub enum InterjectionStorageError {
    NotCommitted(io::Error),
    Indeterminate(io::Error),
    Committed(io::Error),
    Invalid(ReceiptError),
    WrongSession,
    WorkerLost(io::Error),
}

impl From<AppendLineError> for InterjectionStorageError {
    fn from(error: AppendLineError) -> Self {
        match error {
            AppendLineError::NotCommitted(error) => Self::NotCommitted(error),
            AppendLineError::Indeterminate(error) => Self::Indeterminate(error),
            AppendLineError::Committed(error) => Self::Committed(error),
        }
    }
}

impl InterjectionCursor {
    fn refresh(&mut self) -> io::Result<()> {
        let mut file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound && matches!(self.history, InterjectionHistory::Missing) => return Ok(()),
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        let offset = match &self.history {
            InterjectionHistory::Missing => 0,
            InterjectionHistory::Open { file, offset } => {
                let previous = file.metadata()?;
                if previous.dev() == metadata.dev() && previous.ino() == metadata.ino() && metadata.len() >= *offset {
                    *offset
                } else {
                    self.index = InterjectionIndex::default();
                    0
                }
            }
        };
        file.seek(io::SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let index = std::mem::take(&mut self.index);
        let decoded = history_decoder::decode_history_with_interjections(
            &bytes, CHAT_FORMAT_VERSION,
            history_decoder::HistoryLimits { items: usize::MAX, issues: usize::MAX }, index,
        );
        self.index = match decoded {
            Ok(decoded) => match decoded.interjections {
                Ok(index) => index,
                Err(error) => { self.history = InterjectionHistory::Missing; return Err(io::Error::other(error)); }
            },
            Err(_) => { self.history = InterjectionHistory::Missing; return Err(io::Error::new(io::ErrorKind::InvalidData, "history limit")); }
        };
        let offset = offset + bytes.iter().rposition(|byte| *byte == b'\n').map_or(0, |end| end as u64 + 1);
        // Holding the old inode open prevents reuse from hiding an atomic replacement.
        self.history = InterjectionHistory::Open { file, offset };
        Ok(())
    }
}

impl JsonlStorageAdapter {
    pub async fn restore_interjections(&self, info: &Info) -> Result<(InterjectionStorage, InterjectionIndex), InterjectionStorageError> {
        let path = self.chat_file(info);
        let session_id = info.id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut cursor = InterjectionCursor {
                path: path.clone(), index: InterjectionIndex::default(), history: InterjectionHistory::Missing,
            };
            let _lock = Self::lock_append(&path).map_err(InterjectionStorageError::NotCommitted)?;
            cursor.refresh().map_err(InterjectionStorageError::NotCommitted)?;
            let restored = cursor.index.clone();
            Ok((InterjectionStorage { path, session_id, cursor: Arc::new(Mutex::new(cursor)) }, restored))
        }).await.map_err(|error| InterjectionStorageError::WorkerLost(io::Error::other(error)))?
    }

    pub async fn lookup_interjection_receipt(
        &self, info: &Info, state: &InterjectionStorage, id: &str,
    ) -> Result<Option<InterjectionReceipt>, InterjectionStorageError> {
        if state.path != self.chat_file(info) || state.session_id != info.id.to_string() {
            return Err(InterjectionStorageError::WrongSession);
        }
        let owner = Arc::clone(&state.cursor);
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || {
            let mut cursor = owner.lock().map_err(|_| InterjectionStorageError::WorkerLost(io::Error::other("interjection cursor poisoned")))?;
            let _lock = Self::lock_append(&cursor.path).map_err(InterjectionStorageError::NotCommitted)?;
            cursor.refresh().map_err(InterjectionStorageError::NotCommitted)?;
            Ok(cursor.index.get(&id).cloned())
        }).await.map_err(|error| InterjectionStorageError::WorkerLost(io::Error::other(error)))?
    }

    pub async fn append_interjection_receipt(
        &self, info: &Info, state: &InterjectionStorage, receipt: &InterjectionReceipt,
    ) -> Result<InterjectionAppend, InterjectionStorageError> {
        if state.path != self.chat_file(info) || state.session_id != info.id.to_string() {
            return Err(InterjectionStorageError::WrongSession);
        }
        let owner = Arc::clone(&state.cursor);
        let receipt = receipt.clone();
        tokio::task::spawn_blocking(move || {
            let mut cursor = owner.lock().map_err(|_| InterjectionStorageError::WorkerLost(io::Error::other("interjection cursor poisoned")))?;
            let _lock = Self::lock_append(&cursor.path).map_err(InterjectionStorageError::NotCommitted)?;
            cursor.refresh().map_err(InterjectionStorageError::NotCommitted)?;
            if let Some(original) = cursor.index.get(&receipt.identity().id) {
                if !original.same_request(&receipt) {
                    return Ok(InterjectionAppend::Conflict(original.clone()));
                }
                Self::sync_interjection_retry(&cursor.path)?;
                return Ok(InterjectionAppend::AlreadyPresent(original.clone()));
            }
            if receipt.disposition() != xai_chat_state::interjection::InterjectionDisposition::Pending {
                return Err(InterjectionStorageError::Invalid(ReceiptError::InvalidTransition));
            }
            let mut line = serde_json::to_vec(&ReceiptRecord { interjection_receipt: receipt.clone() })
                .map_err(|error| InterjectionStorageError::NotCommitted(io::Error::other(error)))?;
            line.push(b'\n');
            Self::append_jsonl_line_unlocked_with(&cursor.path, line, AppendDurability::Durable,
                Self::sync_file_durable, || Self::sync_parent_directory(&cursor.path))
                .map_err(InterjectionStorageError::from)?;
            cursor.refresh().map_err(InterjectionStorageError::Committed)?;
            Ok(InterjectionAppend::Appended(receipt))
        }).await.map_err(|error| InterjectionStorageError::WorkerLost(io::Error::other(error)))?
    }

    pub async fn persist_interjection_projection(&self, info: &Info, state: &InterjectionStorage, projection: InterjectionProjection) -> Result<InterjectionAppend, InterjectionStorageError> {
        if state.path != self.chat_file(info) || state.session_id != info.id.to_string() { return Err(InterjectionStorageError::WrongSession); }
        let owner = Arc::clone(&state.cursor);
        tokio::task::spawn_blocking(move || {
            let (receipt, _) = projection.clone().into_parts().map_err(InterjectionStorageError::Invalid)?;
            let mut cursor = owner.lock().map_err(|_| InterjectionStorageError::WorkerLost(io::Error::other("interjection cursor poisoned")))?;
            let _lock = Self::lock_append(&cursor.path).map_err(InterjectionStorageError::NotCommitted)?;
            cursor.refresh().map_err(InterjectionStorageError::NotCommitted)?;
            let Some(original) = cursor.index.get(&receipt.identity().id) else { return Err(InterjectionStorageError::Invalid(ReceiptError::InvalidTransition)); };
            if !original.same_request(&receipt) || original.source() != receipt.source() { return Ok(InterjectionAppend::Conflict(original.clone())); }
            if original.disposition() == receipt.disposition() {
                Self::sync_interjection_retry(&cursor.path)?;
                return Ok(InterjectionAppend::AlreadyPresent(original.clone()));
            }
            cursor.index.classify(&receipt).map_err(InterjectionStorageError::Invalid)?;
            let mut line = serde_json::to_vec(&ProjectionRecord { interjection_projection: projection })
                .map_err(|error| InterjectionStorageError::NotCommitted(io::Error::other(error)))?;
            line.push(b'\n');
            Self::append_jsonl_line_unlocked_with(&cursor.path, line, AppendDurability::Durable,
                Self::sync_file_durable, || Self::sync_parent_directory(&cursor.path))
                .map_err(InterjectionStorageError::from)?;
            cursor.refresh().map_err(InterjectionStorageError::Committed)?;
            Ok(InterjectionAppend::Appended(receipt))
        }).await.map_err(|error| InterjectionStorageError::WorkerLost(io::Error::other(error)))?
    }

    fn sync_interjection_retry(path: &Path) -> Result<(), InterjectionStorageError> {
        let file = std::fs::File::open(path).map_err(InterjectionStorageError::Committed)?;
        Self::sync_file_durable(&file).map_err(InterjectionStorageError::Committed)?;
        Self::sync_parent_directory(path).map_err(InterjectionStorageError::Committed)
    }

    pub(super) fn retained_interjections(path: &Path) -> io::Result<InterjectionIndex> {
        let contents = match std::fs::read(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Default::default()),
            Err(error) => return Err(error),
        };
        let decoded = history_decoder::decode_history(&contents, CHAT_FORMAT_VERSION, usize::MAX)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "history limit"))?;
        decoded.interjections().cloned().map_err(io::Error::other)
    }

    pub(super) fn rewrite_with_interjections(path: &Path, items: &[ConversationItem], receipts: &InterjectionIndex) -> io::Result<()> {
        #[derive(Serialize)]
        #[serde(untagged)]
        enum Record<'a> {
            Item(&'a ConversationItem),
            Receipt { interjection_receipt: &'a InterjectionReceipt },
        }
        let records: Vec<_> = items.iter().map(Record::Item)
            .chain(receipts.receipts().map(|interjection_receipt| Record::Receipt { interjection_receipt }))
            .collect();
        crate::session::storage::write_jsonl_atomic(path, &records)
    }
}

#[cfg(test)]
#[path = "interjection_tests.rs"]
mod tests;
