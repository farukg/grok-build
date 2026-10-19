use std::path::{Path, PathBuf};

use crate::compat::{CompatEnv, compat_config_cell, resolve_compat_cells};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeImport {
    Imported,
    NotImported,
}

impl ClaudeImport {
    #[must_use]
    pub fn from_marker(is_marked: bool) -> ClaudeImport {
        if is_marked {
            ClaudeImport::Imported
        } else {
            ClaudeImport::NotImported
        }
    }
}

#[must_use]
pub fn user_config_file(grok_home: &Path) -> PathBuf {
    grok_home.join(crate::USER_CONFIG_FILENAME)
}

// A missing file, a missing key, or invalid TOML counts as not imported. Turning off the Claude
// `rules` or `mcps` compat cell also skips the `.claude/` fallbacks.
pub fn is_claude_import_marked(grok_home: &Path) -> bool {
    claude_import_marked_with_env(grok_home, &CompatEnv::from_process())
}

fn claude_import_marked_with_env(grok_home: &Path, env: &CompatEnv) -> bool {
    let config = std::fs::read_to_string(user_config_file(grok_home))
        .ok()
        .and_then(|contents| toml::from_str::<toml::Value>(&contents).ok());
    let imported = config
        .as_ref()
        .and_then(|config| config.get("claude_compat"))
        .and_then(|section| section.get("imported"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);
    let claude = resolve_compat_cells(
        |cell| compat_config_cell(config.as_ref(), cell).ok().flatten(),
        env,
        None,
    )
    .claude;
    imported || !claude.rules || !claude.mcps
}

#[cfg(test)]
#[path = "claude_import_tests.rs"]
mod tests;
