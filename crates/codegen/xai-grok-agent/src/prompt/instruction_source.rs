//! File-backed instruction values. Loading and activation never mutate a session.

mod loader;

pub use loader::{InstructionLoadError, load_instruction_manifest, load_instruction_snapshot};

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::agents_md::{AgentConfigFile, InstructionSource};

/// Requested provider role. Protocol converters must support or reject it without silently remapping it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionRole {
    System,
    Developer,
    User,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionPolicy {
    pub role: InstructionRole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInstructionTemplate {
    Base,
    ApplyPatch,
    Subagent,
}

impl RuntimeInstructionTemplate {
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Base => "prompt.md",
            Self::ApplyPatch => "apply_patch_prompt.md",
            Self::Subagent => "subagent_prompt.md",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeInstructionAssets {
    root: PathBuf,
}

impl RuntimeInstructionAssets {
    /// Uses the same explicit home that resolves `xai_grok_config::paths::grok_application_in`.
    pub fn in_grok_home(grok_home: &Path) -> Self {
        Self::at_root(grok_home.join("bundled").join("instructions"))
    }

    pub fn at_root(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.root.join("instructions.json")
    }

    pub fn template_path(&self, template: RuntimeInstructionTemplate) -> PathBuf {
        self.root.join("templates").join(template.file_name())
    }

    pub fn template_spec(
        &self,
        template: RuntimeInstructionTemplate,
        digest: InstructionDigest,
    ) -> InstructionFileSpec {
        InstructionFileSpec {
            id: InstructionSourceId(template.file_name().to_owned()),
            path: self.template_path(template),
            origin: InstructionOrigin::RuntimeTemplate(template),
            format: InstructionFormat::PlainText,
            policy: InstructionPolicy { role: InstructionRole::System },
            digest,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalInstructionInputKind {
    SigmaAtom,
    ProductSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionProjectionInput {
    pub kind: CanonicalInstructionInputKind,
    pub path: PathBuf,
    pub digest: InstructionDigest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "source", rename_all = "snake_case")]
pub enum InstructionOrigin {
    RuntimeTemplate(RuntimeInstructionTemplate),
    DiscoveredRule(InstructionSource),
    ConfiguredFile,
    ComposedProjection(Vec<InstructionProjectionInput>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionFormat {
    PlainText,
    RuleMarkdown,
}

impl InstructionFormat {
    fn instruction_text(self, raw: &str) -> String {
        match self {
            Self::PlainText => raw.to_owned(),
            Self::RuleMarkdown => {
                xai_grok_tools::implementations::skills::skill::extract_skill_body(raw)
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct InstructionDigest([u8; 32]);

impl InstructionDigest {
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
}

impl fmt::Display for InstructionDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for InstructionDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("instruction digest must be 64 lowercase hexadecimal digits")]
pub struct InvalidInstructionDigest;

impl FromStr for InstructionDigest {
    type Err = InvalidInstructionDigest;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 64 {
            return Err(InvalidInstructionDigest);
        }
        let mut digest = [0; 32];
        for (out, pair) in digest.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
            fn nibble(byte: u8) -> Result<u8, InvalidInstructionDigest> {
                match byte {
                    b'0'..=b'9' => Ok(byte - b'0'),
                    b'a'..=b'f' => Ok(byte - b'a' + 10),
                    _ => Err(InvalidInstructionDigest),
                }
            }
            *out = (nibble(pair[0])? << 4) | nibble(pair[1])?;
        }
        Ok(Self(digest))
    }
}

impl Serialize for InstructionDigest {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for InstructionDigest {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct InstructionSourceId(String);

impl InstructionSourceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for InstructionSourceId {
    type Error = InvalidInstructionSnapshot;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_id(&value)?;
        Ok(Self(value))
    }
}

impl From<InstructionSourceId> for String {
    fn from(id: InstructionSourceId) -> Self {
        id.0
    }
}

impl fmt::Display for InstructionSourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionFileSpec {
    pub id: InstructionSourceId,
    pub path: PathBuf,
    pub origin: InstructionOrigin,
    pub format: InstructionFormat,
    pub policy: InstructionPolicy,
    /// Pins exact file bytes, including frontmatter and line endings, to this manifest revision.
    pub digest: InstructionDigest,
}

impl InstructionFileSpec {
    /// `format` comes from discovery's named-file versus rules-directory classification.
    /// `digest` covers raw disk bytes, not `AgentConfigFile::content` (which may lack frontmatter).
    pub fn discovered_rule(
        file: &AgentConfigFile,
        format: InstructionFormat,
        digest: InstructionDigest,
    ) -> Result<Self, InvalidInstructionSnapshot> {
        Ok(Self {
            id: InstructionSourceId::try_from(file.file_path.clone())?,
            path: PathBuf::from(&file.file_path),
            origin: InstructionOrigin::DiscoveredRule(file.source),
            format,
            policy: InstructionPolicy { role: InstructionRole::User },
            digest,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "version", content = "sources", deny_unknown_fields)]
pub enum InstructionManifest {
    #[serde(rename = "1")]
    V1(Vec<InstructionFileSpec>),
}

impl InstructionManifest {
    pub fn sources(&self) -> &[InstructionFileSpec] {
        match self {
            Self::V1(sources) => sources,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionFileProvenance {
    requested_path: PathBuf,
    canonical_path: PathBuf,
    origin: InstructionOrigin,
}

impl InstructionFileProvenance {
    pub fn requested_path(&self) -> &Path {
        &self.requested_path
    }

    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    pub fn origin(&self) -> &InstructionOrigin {
        &self.origin
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivatedInstruction {
    id: InstructionSourceId,
    provenance: InstructionFileProvenance,
    format: InstructionFormat,
    policy: InstructionPolicy,
    raw_text: String,
}

impl fmt::Debug for ActivatedInstruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActivatedInstruction")
            .field("id", &self.id)
            .field("provenance", &self.provenance)
            .field("policy", &self.policy)
            .field("digest", &self.digest())
            .finish_non_exhaustive()
    }
}

impl ActivatedInstruction {
    pub fn id(&self) -> &InstructionSourceId {
        &self.id
    }

    pub fn provenance(&self) -> &InstructionFileProvenance {
        &self.provenance
    }

    pub fn policy(&self) -> InstructionPolicy {
        self.policy
    }

    pub fn format(&self) -> InstructionFormat {
        self.format
    }

    pub fn raw_text(&self) -> &str {
        &self.raw_text
    }

    pub fn instruction_text(&self) -> String {
        self.format.instruction_text(&self.raw_text)
    }

    pub fn digest(&self) -> InstructionDigest {
        InstructionDigest::of_bytes(self.raw_text.as_bytes())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "version", content = "sources", deny_unknown_fields)]
enum SnapshotData {
    #[serde(rename = "1")]
    V1(Vec<ActivatedInstruction>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SnapshotData", into = "SnapshotData")]
pub struct InstructionSnapshot {
    sources: Vec<ActivatedInstruction>,
}

#[derive(Debug, thiserror::Error)]
pub enum InvalidInstructionSnapshot {
    #[error("instruction source identity must be nonempty and have no surrounding whitespace")]
    InvalidIdentity,
    #[error("duplicate instruction source identity: {id}")]
    DuplicateIdentity { id: InstructionSourceId },
    #[error("instruction file path must be absolute: {path}")]
    RelativePath { path: PathBuf },
    #[error("instruction file path must be UTF-8: {path:?}")]
    NonUtf8Path { path: PathBuf },
    #[error("duplicate canonical instruction file: {path}")]
    DuplicateFile { path: PathBuf },
    #[error("composed instruction projection must name at least one canonical input")]
    EmptyProjection,
}

fn validate_id(id: &str) -> Result<(), InvalidInstructionSnapshot> {
    if id.is_empty() || id.trim() != id {
        return Err(InvalidInstructionSnapshot::InvalidIdentity);
    }
    Ok(())
}

fn validate_path(path: &Path) -> Result<(), InvalidInstructionSnapshot> {
    if !path.is_absolute() {
        return Err(InvalidInstructionSnapshot::RelativePath { path: path.to_path_buf() });
    }
    if path.to_str().is_none() {
        return Err(InvalidInstructionSnapshot::NonUtf8Path { path: path.to_path_buf() });
    }
    Ok(())
}

impl TryFrom<SnapshotData> for InstructionSnapshot {
    type Error = InvalidInstructionSnapshot;

    fn try_from(data: SnapshotData) -> Result<Self, Self::Error> {
        let SnapshotData::V1(sources) = data;
        let mut ids = HashSet::new();
        let mut paths = HashSet::new();
        for source in &sources {
            validate_id(source.id.as_str())?;
            if !ids.insert(&source.id) {
                return Err(InvalidInstructionSnapshot::DuplicateIdentity { id: source.id.clone() });
            }
            validate_path(&source.provenance.requested_path)?;
            validate_path(&source.provenance.canonical_path)?;
            match &source.provenance.origin {
                InstructionOrigin::RuntimeTemplate(_)
                | InstructionOrigin::DiscoveredRule(_)
                | InstructionOrigin::ConfiguredFile => {}
                InstructionOrigin::ComposedProjection(inputs) => {
                    if inputs.is_empty() {
                        return Err(InvalidInstructionSnapshot::EmptyProjection);
                    }
                    let mut input_paths = HashSet::new();
                    for input in inputs {
                        validate_path(&input.path)?;
                        if !input_paths.insert(&input.path) {
                            return Err(InvalidInstructionSnapshot::DuplicateFile {
                                path: input.path.clone(),
                            });
                        }
                    }
                }
            }
            if !paths.insert(&source.provenance.canonical_path) {
                return Err(InvalidInstructionSnapshot::DuplicateFile {
                    path: source.provenance.canonical_path.clone(),
                });
            }
        }
        Ok(Self { sources })
    }
}

impl From<InstructionSnapshot> for SnapshotData {
    fn from(snapshot: InstructionSnapshot) -> Self {
        Self::V1(snapshot.sources)
    }
}

impl InstructionSnapshot {
    pub fn start(loaded: LoadedInstructionSnapshot) -> Self {
        loaded.snapshot
    }

    pub fn sources(&self) -> &[ActivatedInstruction] {
        &self.sources
    }

    /// Identity of the ordered, persisted source values, not a provider cache key.
    pub fn digest(&self) -> InstructionDigest {
        let mut hash = Sha256::new();
        fn field(hash: &mut Sha256, bytes: &[u8]) {
            hash.update((bytes.len() as u64).to_be_bytes());
            hash.update(bytes);
        }
        field(&mut hash, b"grok-instruction-snapshot-v1");
        for source in &self.sources {
            field(&mut hash, source.id.as_str().as_bytes());
            field(&mut hash, source.provenance.requested_path.as_os_str().as_encoded_bytes());
            field(&mut hash, source.provenance.canonical_path.as_os_str().as_encoded_bytes());
            match &source.provenance.origin {
                InstructionOrigin::RuntimeTemplate(template) => {
                    field(&mut hash, b"runtime_template");
                    field(&mut hash, template.file_name().as_bytes());
                }
                InstructionOrigin::DiscoveredRule(scope) => {
                    field(&mut hash, b"discovered_rule");
                    field(&mut hash, match scope {
                        InstructionSource::Home => b"home",
                        InstructionSource::Configured => b"configured",
                        InstructionSource::Project => b"project",
                    });
                }
                InstructionOrigin::ConfiguredFile => field(&mut hash, b"configured_file"),
                InstructionOrigin::ComposedProjection(inputs) => {
                    field(&mut hash, b"composed_projection");
                    field(&mut hash, &(inputs.len() as u64).to_be_bytes());
                    for input in inputs {
                        field(&mut hash, match input.kind {
                            CanonicalInstructionInputKind::SigmaAtom => b"sigma_atom",
                            CanonicalInstructionInputKind::ProductSource => b"product_source",
                        });
                        field(&mut hash, input.path.as_os_str().as_encoded_bytes());
                        field(&mut hash, &input.digest.0);
                    }
                }
            }
            field(&mut hash, match source.format {
                InstructionFormat::PlainText => b"plain_text",
                InstructionFormat::RuleMarkdown => b"rule_markdown",
            });
            field(&mut hash, match source.policy.role {
                InstructionRole::System => b"system",
                InstructionRole::Developer => b"developer",
                InstructionRole::User => b"user",
            });
            field(&mut hash, source.raw_text.as_bytes());
        }
        InstructionDigest(hash.finalize().into())
    }
}

#[derive(Clone, Debug)]
pub struct LoadedInstructionSnapshot {
    snapshot: InstructionSnapshot,
}

impl LoadedInstructionSnapshot {
    pub fn snapshot(&self) -> &InstructionSnapshot {
        &self.snapshot
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpectedPrefixChange {
    Unchanged,
    Changed,
}

#[derive(Debug)]
pub enum InstructionSourceChange<'a> {
    Added { new_index: usize, new: &'a ActivatedInstruction },
    Removed { old_index: usize, old: &'a ActivatedInstruction },
    Changed {
        old_index: usize,
        old: &'a ActivatedInstruction,
        new_index: usize,
        new: &'a ActivatedInstruction,
    },
}

#[derive(Debug)]
pub struct InstructionActivationPlan {
    previous: InstructionSnapshot,
    candidate: InstructionSnapshot,
}

pub fn prepare_instruction_activation(
    active: &InstructionSnapshot,
    loaded: LoadedInstructionSnapshot,
) -> InstructionActivationPlan {
    InstructionActivationPlan { previous: active.clone(), candidate: loaded.snapshot }
}

#[derive(Debug, thiserror::Error)]
#[error("the session's active instruction snapshot changed after activation was prepared")]
pub struct StaleInstructionActivation;

impl InstructionActivationPlan {
    pub fn previous(&self) -> &InstructionSnapshot {
        &self.previous
    }

    pub fn candidate(&self) -> &InstructionSnapshot {
        &self.candidate
    }

    /// Conservative source-prefix comparison; rendering/protocol/cache behavior belongs downstream.
    pub fn expected_prefix_change(&self) -> ExpectedPrefixChange {
        if self.previous == self.candidate {
            ExpectedPrefixChange::Unchanged
        } else {
            ExpectedPrefixChange::Changed
        }
    }

    pub fn changes(&self) -> Vec<InstructionSourceChange<'_>> {
        let mut candidates: HashMap<_, _> = self.candidate.sources.iter().enumerate()
            .map(|(index, source)| (&source.id, (index, source)))
            .collect();
        let mut changes = Vec::new();
        for (old_index, old) in self.previous.sources.iter().enumerate() {
            match candidates.remove(&old.id) {
                Some((new_index, new)) => {
                    if old_index != new_index || old != new {
                        changes.push(InstructionSourceChange::Changed { old_index, old, new_index, new });
                    }
                }
                None => changes.push(InstructionSourceChange::Removed { old_index, old }),
            }
        }
        for (new_index, new) in self.candidate.sources.iter().enumerate() {
            if candidates.contains_key(&new.id) {
                changes.push(InstructionSourceChange::Added { new_index, new });
            }
        }
        changes
    }

    pub fn activate(
        self,
        active: &InstructionSnapshot,
    ) -> Result<InstructionSnapshot, StaleInstructionActivation> {
        if *active != self.previous {
            return Err(StaleInstructionActivation);
        }
        Ok(self.candidate)
    }
}

#[cfg(test)]
#[path = "instruction_source_tests.rs"]
mod tests;
