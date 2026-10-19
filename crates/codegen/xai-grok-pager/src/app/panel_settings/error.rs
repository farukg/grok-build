//! Typed failures of loading and saving panel settings.

use std::io;
use std::path::PathBuf;

use super::types::Layer;

/// Why part of a settings file was not used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadProblem {
    #[error("file is unreadable ({kind})")]
    Unreadable { kind: io::ErrorKind },
    #[error("file is not valid TOML: {0}")]
    Malformed(toml_edit::TomlError),
    #[error("file was written by a newer format version {found}; unknown parts are preserved")]
    NewerVersion { found: i64 },
    #[error("ignored unrecognized or invalid entry `{key}`")]
    IgnoredEntry { key: String },
}

/// A [`LoadProblem`] with the layer file it came from. Callers log it with `tracing::warn!`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{:?} panel settings {}: {}", .layer, .path.display(), .problem)]
pub struct LoadWarning {
    pub layer: Layer,
    pub path: PathBuf,
    pub problem: LoadProblem,
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("no session is attached, so session settings cannot be stored")]
    NoSession,
    #[error("cannot write panel settings file {}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// What a save did to the layer file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveOutcome {
    Written,
    /// The file was unusable; its bytes were moved to `backup` and a fresh file was written.
    Repaired {
        backup: PathBuf,
    },
    /// The layer already held the value; nothing was written.
    Unchanged,
}
