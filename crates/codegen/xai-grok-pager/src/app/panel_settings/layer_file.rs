//! One layer's TOML file: load it into typed values, save edits into it atomically.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use toml_edit::DocumentMut;
use xai_grok_config::create_dir_all_owner_only;
use xai_grok_config::fs_atomic::write_user_file_atomically;

use super::codec;
use super::error::{LoadProblem, LoadWarning, SaveError, SaveOutcome};
use super::store::LayerReload;
use super::types::{Edit, Layer, LayerSettings};

/// Settings files hold session ids and view choices; keep them private to the user.
const OWNER_ONLY_FILE_MODE: u32 = 0o600;

/// What a settings path holds.
enum FileContent {
    Absent,
    Document(DocumentMut),
    Corrupt(LoadProblem),
}

enum SaveSource {
    Healthy,
    SyntaxRepair,
    SemanticRepair,
}

pub(super) struct LayerFile {
    layer: Layer,
    path: PathBuf,
    settings: LayerSettings,
}

impl LayerFile {
    /// Reads `path`. An absent file is an empty layer; an unusable one is an empty layer plus a warning and stays on disk.
    pub(super) fn load(layer: Layer, path: PathBuf) -> (Self, Vec<LoadWarning>) {
        let (settings, problems) = match read(&path) {
            Ok(FileContent::Absent) => (LayerSettings::default(), Vec::new()),
            Ok(FileContent::Document(doc)) => {
                let decoded = codec::decode(layer, &doc);
                (decoded.settings, decoded.problems)
            }
            Ok(FileContent::Corrupt(problem)) => (LayerSettings::default(), vec![problem]),
            Err(err) => (
                LayerSettings::default(),
                vec![LoadProblem::Unreadable { kind: err.kind() }],
            ),
        };
        let warnings = problems
            .into_iter()
            .map(|problem| LoadWarning {
                layer,
                path: path.clone(),
                problem,
            })
            .collect();
        (
            Self {
                layer,
                path,
                settings,
            },
            warnings,
        )
    }

    pub(super) fn settings(&self) -> &LayerSettings {
        &self.settings
    }

    /// Re-reads the file, adopting its values only when they are usable. An absent
    /// file is an empty layer (its overrides really are gone); a damaged or
    /// unreadable one keeps the last-known values and reports why, so another
    /// window's broken write never resets this process's user-chosen state. A later
    /// healthy read adopts the file again.
    pub(super) fn reload(&mut self) -> LayerReload {
        let (loaded, problems): (Option<LayerSettings>, Vec<LoadProblem>) = match read(&self.path) {
            Ok(FileContent::Absent) => (Some(LayerSettings::default()), Vec::new()),
            Ok(FileContent::Document(doc)) => {
                let decoded = codec::decode(self.layer, &doc);
                (if decoded.has_invalid_known() { None } else { Some(decoded.settings) }, decoded.problems)
            }
            Ok(FileContent::Corrupt(problem)) => (None, vec![problem]),
            Err(err) => (None, vec![LoadProblem::Unreadable { kind: err.kind() }]),
        };
        let warnings = problems
            .into_iter()
            .map(|problem| LoadWarning {
                layer: self.layer,
                path: self.path.clone(),
                problem,
            })
            .collect();
        match loaded {
            Some(settings) => {
                let changed = settings != self.settings;
                self.settings = settings;
                LayerReload { changed, warnings }
            }
            None => LayerReload {
                changed: false,
                warnings,
            },
        }
    }

    /// Applies `edits` to the file as it is on disk right now. Read-modify-write: re-read it, apply only the edits that change something there,
    /// and publish atomically. The held values become what the file now holds; a failed write leaves them as they were.
    /// Two processes editing the same file in the same instant can still lose one edit; see the module doc.
    pub(super) fn save(&mut self, edits: &[Edit]) -> Result<SaveOutcome, SaveError> {
        let io_error = |source| SaveError::Io {
            path: self.path.clone(),
            source,
        };
        let (mut doc, mut current, source) = match read(&self.path).map_err(io_error)? {
            FileContent::Absent => (DocumentMut::new(), LayerSettings::default(), SaveSource::Healthy),
            FileContent::Document(doc) => {
                let decoded = codec::decode(self.layer, &doc);
                let source = if decoded.has_invalid_known() { SaveSource::SemanticRepair } else { SaveSource::Healthy };
                (doc, decoded.settings, source)
            }
            FileContent::Corrupt(_) => (DocumentMut::new(), LayerSettings::default(), SaveSource::SyntaxRepair),
        };
        let applied: Vec<&Edit> = edits.iter().filter(|edit| {
            let changed = current.apply(edit);
            changed || matches!(source, SaveSource::SemanticRepair)
        }).collect();
        for edit in &applied {
            codec::encode(&mut doc, edit);
        }
        let decoded = codec::decode(self.layer, &doc);
        if let Some(key) = decoded.invalid_known_key() {
            return Err(SaveError::InvalidKnown { path: self.path.clone(), key: key.to_owned() });
        }
        current = decoded.settings;
        if applied.is_empty() && matches!(source, SaveSource::Healthy) {
            self.settings = current;
            return Ok(SaveOutcome::Unchanged);
        }
        let backup = match source {
            SaveSource::Healthy => None,
            SaveSource::SyntaxRepair | SaveSource::SemanticRepair => {
                let backup = backup_path(&self.path);
                std::fs::rename(&self.path, &backup).map_err(io_error)?;
                Some(backup)
            }
        };
        codec::stamp_version(&mut doc);
        if let Some(parent) = self.path.parent() {
            create_dir_all_owner_only(parent).map_err(io_error)?;
        }
        write_user_file_atomically(&self.path, &doc.to_string(), Some(OWNER_ONLY_FILE_MODE))
            .map_err(io_error)?;
        self.settings = current;
        Ok(match backup {
            Some(backup) => SaveOutcome::Repaired { backup },
            None => SaveOutcome::Written,
        })
    }
}

/// `Err` only for failures that say nothing about the content (permissions, I/O), so the file is not touched.
fn read(path: &Path) -> io::Result<FileContent> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(match text.parse::<DocumentMut>() {
            Ok(doc) => FileContent::Document(doc),
            Err(err) => FileContent::Corrupt(LoadProblem::Malformed(err)),
        }),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(FileContent::Absent),
        Err(err) if err.kind() == io::ErrorKind::InvalidData => {
            Ok(FileContent::Corrupt(LoadProblem::Unreadable {
                kind: err.kind(),
            }))
        }
        Err(err) => Err(err),
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(OsString::from).unwrap_or_default();
    name.push(".bad");
    path.with_file_name(name)
}
