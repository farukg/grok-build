//! Leader-owned Context choices. Legacy session JSON is read once and never overwritten.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use xai_grok_sampling_types::{
    ContextPolicy, ContextPolicyEdit, ContextPolicyMutation, ContextPolicySelection,
    ContextPolicyVersion, ContextScope, ContextSetting, SessionContextPolicy, all_categories,
};

pub(crate) type ContextPolicyAuthority = Arc<RwLock<ContextPolicyStore>>;

#[derive(Debug, thiserror::Error)]
pub enum ContextPolicyError {
    #[error("Context storage I/O failed")]
    Io(#[from] std::io::Error),
    #[error("Context was published, but directory durability is unconfirmed")]
    Durability(#[source] std::io::Error),
    #[error("Context persistence worker result was lost")]
    Worker(#[from] tokio::task::JoinError),
    #[error("Context TOML is invalid")]
    Decode(#[from] toml::de::Error),
    #[error("Context TOML could not be encoded")]
    Encode(#[from] toml::ser::Error),
    #[error("Legacy Context JSON is invalid")]
    Legacy(#[from] serde_json::Error),
    #[error("Unsupported Context store version {0}")]
    UnsupportedVersion(u32),
    #[error("Context confirmation is stale")]
    Stale,
    #[error("Context origin session changed")]
    OriginChanged,
    #[error("Context revision exhausted")]
    RevisionExhausted,
    #[error("Context authority has not been loaded")]
    NotLoaded,
    #[error(transparent)]
    Admission(#[from] xai_chat_state::ContextPolicyAdmissionError),
}

#[derive(Debug, Clone)]
pub(crate) struct ContextPolicyBinding {
    pub authority: ContextPolicyAuthority,
    pub incarnation: String,
    pub applied_revision: Arc<std::sync::atomic::AtomicU64>,
}

impl ContextPolicyBinding {
    pub fn new(authority: ContextPolicyAuthority) -> Self {
        Self { authority, incarnation: uuid::Uuid::new_v4().to_string(), applied_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)) }
    }
}

#[cfg(test)]
impl Default for ContextPolicyBinding {
    fn default() -> Self {
        let mut store = ContextPolicyStore::new(
            std::env::temp_dir().join(format!("grok-context-test-{}", uuid::Uuid::new_v4())),
        );
        store.loaded = Loaded::Ready(Document::default());
        Self::new(Arc::new(RwLock::new(store)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: u32,
    revision: u64,
    global: Vec<ContextSetting>,
    #[serde(default)]
    sessions: BTreeMap<String, SessionContextPolicy>,
}

impl Default for Document {
    fn default() -> Self {
        Self { format: 1, revision: 0, global: Vec::new(), sessions: BTreeMap::new() }
    }
}

impl Document {
    fn global(&self) -> ContextPolicy {
        let mut policy = ContextPolicy::default();
        for setting in &self.global {
            policy.set(setting.category, setting.value);
        }
        policy
    }
}

#[derive(Debug)]
enum Loaded {
    Unread,
    Ready(Document),
}

#[derive(Debug)]
pub(crate) struct ContextPolicyStore {
    root: PathBuf,
    loaded: Loaded,
    inspected_legacy: BTreeSet<String>,
}

impl ContextPolicyStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root, loaded: Loaded::Unread, inspected_legacy: BTreeSet::new() }
    }

    pub fn authority() -> ContextPolicyAuthority {
        Arc::new(RwLock::new(Self::new(crate::util::grok_home::grok_home())))
    }

    fn read(&self) -> Result<Document, ContextPolicyError> {
        let text = match std::fs::read_to_string(self.root.join("context_policy.toml")) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Document::default()),
            Err(error) => return Err(error.into()),
        };
        let document: Document = toml::from_str(&text)?;
        if document.format != 1 {
            return Err(ContextPolicyError::UnsupportedVersion(document.format));
        }
        Ok(document)
    }

    pub fn load(&mut self) -> Result<(), ContextPolicyError> {
        if matches!(self.loaded, Loaded::Unread) {
            self.loaded = Loaded::Ready(self.read()?);
        }
        Ok(())
    }

    fn document(&self) -> Result<&Document, ContextPolicyError> {
        match &self.loaded {
            Loaded::Ready(document) => Ok(document),
            Loaded::Unread => Err(ContextPolicyError::NotLoaded),
        }
    }

    fn publish(&mut self, mut document: Document) -> Result<(), ContextPolicyError> {
        document.revision = document.revision.checked_add(1)
            .ok_or(ContextPolicyError::RevisionExhausted)?;
        let text = toml::to_string(&document)?;
        crate::util::config::atomic_replace_string(&self.root.join("context_policy.toml"), &text)?;
        self.loaded = Loaded::Ready(document);
        #[cfg(unix)]
        std::fs::File::open(&self.root).and_then(|directory| directory.sync_all())
            .map_err(ContextPolicyError::Durability)?;
        Ok(())
    }

    pub fn ensure_session(&mut self, id: &str, session_dir: &Path) -> Result<(), ContextPolicyError> {
        self.load()?;
        if self.inspected_legacy.contains(id) || self.document()?.sessions.contains_key(id) {
            return Ok(());
        }
        let legacy = match std::fs::read_to_string(session_dir.join("context_policy.json")) {
            Ok(text) => serde_json::from_str::<ContextPolicy>(&text)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.inspected_legacy.insert(id.to_owned());
                return Ok(());
            },
            Err(error) => return Err(error.into()),
        };
        let _lock = crate::util::config::acquire_init_lock(&self.root)?;
        let mut document = self.read()?;
        if !document.sessions.contains_key(id) {
            document.sessions.insert(id.to_owned(), SessionContextPolicy {
                scope: ContextScope::Session,
                overrides: all_categories().into_iter().map(|category| ContextSetting {
                    category, value: legacy.switch(category),
                }).collect(),
            });
            self.publish(document)?;
        } else {
            self.loaded = Loaded::Ready(document);
        }
        Ok(())
    }

    pub fn effective(&self, id: &str) -> Result<ContextPolicy, ContextPolicyError> {
        let document = self.document()?;
        Ok(document.sessions.get(id).cloned().unwrap_or_default().effective(&document.global()))
    }

    pub fn selection(&self, id: &str, incarnation: &str) -> Result<ContextPolicySelection, ContextPolicyError> {
        let document = self.document()?;
        Ok(ContextPolicySelection {
            version: ContextPolicyVersion { revision: document.revision, incarnation: incarnation.to_owned() },
            scope: document.sessions.get(id).map_or(ContextScope::Global, |session| session.scope),
        })
    }

    pub fn edit(&mut self, id: &str, incarnation: &str, edit: &ContextPolicyEdit) -> Result<(), ContextPolicyError> {
        if edit.expected.incarnation != incarnation {
            return Err(ContextPolicyError::OriginChanged);
        }
        let _lock = crate::util::config::acquire_init_lock(&self.root)?;
        let mut document = self.read()?;
        if edit.expected.revision != document.revision {
            self.loaded = Loaded::Ready(document);
            return Err(ContextPolicyError::Stale);
        }
        let previous = document.clone();
        match &edit.mutation {
            ContextPolicyMutation::ReplaceSession { policy } => {
                document.sessions.insert(id.to_owned(), SessionContextPolicy {
                    scope: ContextScope::Session,
                    overrides: all_categories().into_iter().map(|category| ContextSetting {
                        category, value: policy.switch(category),
                    }).collect(),
                });
            }
            ContextPolicyMutation::SelectScope { scope } => {
                if document.sessions.get(id).map_or(ContextScope::Global, |session| session.scope) != *scope {
                    document.sessions.entry(id.to_owned()).or_default().scope = *scope;
                }
            }
            ContextPolicyMutation::SetCategories { categories, value, target } => {
                match target {
                    ContextScope::Global => {
                        for &category in categories.iter() {
                            match document.global.iter_mut().find(|setting| setting.category == category) {
                                Some(setting) => setting.value = *value,
                                None => document.global.push(ContextSetting { category, value: *value }),
                            }
                        }
                    }
                    ContextScope::Session => {
                        let session = document.sessions.entry(id.to_owned()).or_default();
                        session.scope = ContextScope::Session;
                        for &category in categories.iter() {
                            session.set(category, *value);
                        }
                    }
                }
            }
        }
        if document != previous {
            self.publish(document)?;
        } else {
            self.loaded = Loaded::Ready(document);
        }
        Ok(())
    }

    pub fn inherited_choices_are_current(&self, source: &str, target: &str) -> bool {
        match &self.loaded {
            Loaded::Unread => false,
            Loaded::Ready(document) => {
                (self.inspected_legacy.contains(source) || document.sessions.contains_key(source))
                    && document.sessions.get(source).cloned().unwrap_or_default()
                        == document.sessions.get(target).cloned().unwrap_or_default()
            }
        }
    }

    pub fn inherit(&mut self, source: &str, target: &str) -> Result<(), ContextPolicyError> {
        self.load()?;
        let current = self.document()?;
        if current.sessions.get(source).cloned().unwrap_or_default() == SessionContextPolicy::default()
            && !current.sessions.contains_key(target)
        { return Ok(()); }
        let _lock = crate::util::config::acquire_init_lock(&self.root)?;
        let mut document = self.read()?;
        let selection = document.sessions.get(source).cloned().unwrap_or_default();
        if selection == SessionContextPolicy::default() && !document.sessions.contains_key(target) {
            self.loaded = Loaded::Ready(document);
            return Ok(());
        }
        if document.sessions.get(target) != Some(&selection) {
            document.sessions.insert(target.to_owned(), selection);
            self.publish(document)?;
        } else {
            self.loaded = Loaded::Ready(document);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "context_policy_store_tests.rs"]
mod tests;
