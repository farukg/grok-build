use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use super::{
    ActivatedInstruction, InstructionDigest, InstructionFileProvenance, InstructionManifest,
    InstructionOrigin, InstructionSnapshot, InvalidInstructionSnapshot, LoadedInstructionSnapshot, SnapshotData,
    validate_id, validate_path,
};

#[derive(Debug, thiserror::Error)]
pub enum InstructionLoadError {
    #[error("instruction file is missing during {operation}: {path}")]
    Missing {
        path: PathBuf,
        operation: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("cannot {operation} instruction file: {path}")]
    Io {
        path: PathBuf,
        operation: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("invalid instruction manifest: {path}")]
    InvalidManifest {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid instruction source values")]
    InvalidSource(#[from] InvalidInstructionSnapshot),
    #[error("instruction file is not a regular file: {path}")]
    NotRegularFile { path: PathBuf },
    #[error("instruction file is not UTF-8: {path}")]
    InvalidUtf8 {
        path: PathBuf,
        #[source]
        source: std::string::FromUtf8Error,
    },
    #[error("instruction file differs from its manifest digest: {path} (expected {expected}, got {actual})")]
    DigestMismatch {
        path: PathBuf,
        expected: InstructionDigest,
        actual: InstructionDigest,
    },
    #[error("instruction source changed while loading: {path}")]
    ChangedDuringLoad { path: PathBuf },
}

fn file_error(path: &Path, operation: &'static str, source: io::Error) -> InstructionLoadError {
    if source.kind() == io::ErrorKind::NotFound {
        InstructionLoadError::Missing { path: path.to_path_buf(), operation, source }
    } else {
        InstructionLoadError::Io { path: path.to_path_buf(), operation, source }
    }
}

fn canonical_file(path: &Path) -> Result<PathBuf, InstructionLoadError> {
    let canonical = dunce::canonicalize(path)
        .map_err(|source| file_error(path, "resolve", source))?;
    validate_path(&canonical)?;
    let metadata = std::fs::metadata(&canonical)
        .map_err(|source| file_error(path, "inspect", source))?;
    if !metadata.is_file() {
        return Err(InstructionLoadError::NotRegularFile { path: path.to_path_buf() });
    }
    Ok(canonical)
}

fn read_file(path: &Path) -> Result<Vec<u8>, InstructionLoadError> {
    std::fs::read(path).map_err(|source| file_error(path, "read", source))
}

fn read_pinned_file(
    path: &Path,
    expected: InstructionDigest,
) -> Result<(PathBuf, Vec<u8>), InstructionLoadError> {
    validate_path(path)?;
    let canonical = canonical_file(path)?;
    let bytes = read_file(&canonical)?;
    let actual = InstructionDigest::of_bytes(&bytes);
    if actual != expected {
        return Err(InstructionLoadError::DigestMismatch {
            path: path.to_path_buf(), expected, actual,
        });
    }
    if canonical_file(path)? != canonical {
        return Err(InstructionLoadError::ChangedDuringLoad { path: path.to_path_buf() });
    }
    Ok((canonical, bytes))
}

/// Relative source paths are resolved only against this manifest's directory, never process cwd.
/// Manifest digests pin the complete ordered set; a partial or mixed revision is rejected.
pub fn load_instruction_manifest(
    path: &Path,
) -> Result<LoadedInstructionSnapshot, InstructionLoadError> {
    validate_path(path)?;
    let canonical = canonical_file(path)?;
    let bytes = read_file(&canonical)?;
    let mut manifest: InstructionManifest = serde_json::from_slice(&bytes)
        .map_err(|source| InstructionLoadError::InvalidManifest { path: path.to_path_buf(), source })?;
    let InstructionManifest::V1(sources) = &mut manifest;
    let parent = canonical.parent().ok_or_else(|| InstructionLoadError::NotRegularFile {
        path: canonical.clone(),
    })?;
    for source in sources {
        if source.path.is_relative() {
            source.path = parent.join(&source.path);
        }
        match &mut source.origin {
            InstructionOrigin::RuntimeTemplate(_)
            | InstructionOrigin::DiscoveredRule(_)
            | InstructionOrigin::ConfiguredFile => {}
            InstructionOrigin::ComposedProjection(inputs) => {
                for input in inputs {
                    if input.path.is_relative() {
                        input.path = parent.join(&input.path);
                    }
                }
            }
        }
    }
    let loaded = load_instruction_snapshot(&manifest)?;
    if canonical_file(path)? != canonical || read_file(&canonical)? != bytes {
        return Err(InstructionLoadError::ChangedDuringLoad { path: path.to_path_buf() });
    }
    Ok(loaded)
}

/// Reads data only. No fallback, discovery, environment lookup, activation or session mutation.
pub fn load_instruction_snapshot(
    manifest: &InstructionManifest,
) -> Result<LoadedInstructionSnapshot, InstructionLoadError> {
    let mut ids = HashSet::new();
    for spec in manifest.sources() {
        validate_id(spec.id.as_str())?;
        validate_path(&spec.path)?;
        if !ids.insert(&spec.id) {
            return Err(InvalidInstructionSnapshot::DuplicateIdentity { id: spec.id.clone() }.into());
        }
    }
    let mut sources = Vec::with_capacity(manifest.sources().len());
    let mut paths = HashSet::new();
    for spec in manifest.sources() {
        let (canonical, bytes) = read_pinned_file(&spec.path, spec.digest)?;
        if !paths.insert(canonical.clone()) {
            return Err(InvalidInstructionSnapshot::DuplicateFile { path: canonical }.into());
        }
        let mut origin = spec.origin.clone();
        match &mut origin {
            InstructionOrigin::RuntimeTemplate(_)
            | InstructionOrigin::DiscoveredRule(_)
            | InstructionOrigin::ConfiguredFile => {}
            InstructionOrigin::ComposedProjection(inputs) => {
                for input in inputs {
                    let (canonical_input, _) = read_pinned_file(&input.path, input.digest)?;
                    input.path = canonical_input;
                }
            }
        }
        let raw_text = String::from_utf8(bytes).map_err(|source| InstructionLoadError::InvalidUtf8 {
            path: spec.path.clone(), source,
        })?;
        sources.push(ActivatedInstruction {
            id: spec.id.clone(),
            provenance: InstructionFileProvenance {
                requested_path: spec.path.clone(),
                canonical_path: canonical,
                origin,
            },
            format: spec.format,
            policy: spec.policy,
            raw_text,
        });
    }
    for spec in manifest.sources() {
        match &spec.origin {
            InstructionOrigin::RuntimeTemplate(_)
            | InstructionOrigin::DiscoveredRule(_)
            | InstructionOrigin::ConfiguredFile => {}
            InstructionOrigin::ComposedProjection(inputs) => {
                for input in inputs {
                    read_pinned_file(&input.path, input.digest)?;
                }
            }
        }
    }
    for source in &sources {
        let path = source.provenance.requested_path();
        if canonical_file(path)? != source.provenance.canonical_path()
            || InstructionDigest::of_bytes(&read_file(source.provenance.canonical_path())?) != source.digest()
        {
            return Err(InstructionLoadError::ChangedDuringLoad { path: path.to_path_buf() });
        }
    }
    Ok(LoadedInstructionSnapshot {
        snapshot: InstructionSnapshot::try_from(SnapshotData::V1(sources))?,
    })
}
