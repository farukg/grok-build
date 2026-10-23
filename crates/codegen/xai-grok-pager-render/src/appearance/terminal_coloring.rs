//! The `terminal_coloring` user setting (`terminal` | `syntax` | `both`).
//!
//! Captured text that carries terminal escape codes (command output, `.log` reads) can be colored by the escapes the producing tool chose, by the DSL/syntax grammar, or by both.
//! This crate owns the persisted value type; the renderers in `render::content_kind` apply it.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};

/// Which color source paints captured terminal text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TerminalColoring {
    /// Only the colors the producing tool emitted. Default.
    #[default]
    Terminal,
    /// Escapes are removed and the text is colored by its syntax grammar (the path's language, else DSL).
    Syntax,
    /// Escape colors win; text the tool left uncolored takes its syntax color.
    Both,
}

impl TerminalColoring {
    /// Canonical persisted string.
    pub const fn as_canonical(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Syntax => "syntax",
            Self::Both => "both",
        }
    }

    const fn as_u8(self) -> u8 {
        match self {
            Self::Terminal => 0,
            Self::Syntax => 1,
            Self::Both => 2,
        }
    }

    const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Syntax,
            2 => Self::Both,
            _ => Self::Terminal,
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(TerminalColoring::Terminal.as_u8());

/// The setting in effect, for viewers that hold no appearance config.
pub fn current() -> TerminalColoring {
    TerminalColoring::from_u8(CURRENT.load(Ordering::Relaxed))
}

/// Update the global setting (called when config is loaded/reloaded).
pub fn set_current(coloring: TerminalColoring) {
    CURRENT.store(coloring.as_u8(), Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_setting_round_trips_every_variant() {
        for coloring in [
            TerminalColoring::Both,
            TerminalColoring::Syntax,
            TerminalColoring::Terminal,
        ] {
            set_current(coloring);
            assert_eq!(current(), coloring);
        }
    }

    #[test]
    fn canonical_strings_match_serde_names() {
        for coloring in [
            TerminalColoring::Terminal,
            TerminalColoring::Syntax,
            TerminalColoring::Both,
        ] {
            let json = serde_json::to_string(&coloring).expect("serialize");
            assert_eq!(json, format!("\"{}\"", coloring.as_canonical()));
        }
    }
}
