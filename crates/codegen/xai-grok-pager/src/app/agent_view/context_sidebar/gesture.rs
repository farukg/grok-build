//! Keys and clicks of the F7 sidebar, mapped once into commands.
//!
//! | gesture                       | section title               | Message display row            | Context row                      |
//! |-------------------------------|-----------------------------|--------------------------------|----------------------------------|
//! | Enter, Space, click           | fold or unfold the section  | flip this kind                 | ask the shell to flip the switch |
//! | Shift+Enter, Shift+click      | toggle all other sections   | toggle all other kinds         | toggle all other sections        |
//! | `+` / `-`                     | open / close all sections   | bulk kinds full / on one line  | open / close all sections        |
//! | Ctrl+Enter, Ctrl+click, `*`   | toggle everything else      | toggle everything else         | toggle everything else           |
//! | `g`                           | save to global / this session (same on every row; the footer shows the target)    |
//!
//! A toggle opens everything it reaches when any of it is closed (a section folded, a kind on one line) and
//! otherwise closes it all, so repeating it reverts it. "Everything" is every other section and every other
//! bulk kind. A modifier activation never changes the line it starts from, and user and assistant message
//! rows are never part of a bulk gesture: they are turned one by one. Context switches never change in bulk.
//! Terminals often do not deliver Shift+Enter, Ctrl+Enter or modified clicks, hence the plain `+`, `-` and `*` keys.
//!
//! Navigation: Up/Down (k/j) over visible lines, Home/End, PageUp/PageDown, Right (l) unfolds a folded title or steps
//! to its first row, Left (h) folds an unfolded title or steps from a row to its title. Tab leaves for the chat, Esc closes.

use super::RowTarget;
use super::outline::CursorTarget;
use crate::views::tree_nav::{BulkAnchor, BulkDirection, TreeKey};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Which lines a bulk gesture reaches from the cursor line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Spread {
    /// The cursor line's peers: all sections, or all message kinds.
    Peers,
    /// Every section and every message kind.
    Everything,
}

/// What a bulk gesture does to what it reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Goal {
    /// Open when anything is closed, else close.
    Toggle,
    Set(BulkDirection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SidebarCommand {
    Tree(TreeKey),
    Activate,
    Bulk {
        spread: Spread,
        goal: Goal,
        /// Whether the cursor line itself is part of the change.
        anchor: BulkAnchor,
    },
    /// Flip where edits are saved: globally or for this session only.
    ToggleEditLayer,
    FocusChat,
    Close,
}

/// `None` lets the key continue to the chat. `page` is the number of lines a page key moves.
pub(super) fn command_for(key: &KeyEvent, page: usize) -> Option<SidebarCommand> {
    let plain = !key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    let bulk = |spread, goal| {
        Some(SidebarCommand::Bulk {
            spread,
            goal,
            anchor: BulkAnchor::Include,
        })
    };
    match key.code {
        KeyCode::Up => Some(SidebarCommand::Tree(TreeKey::Up)),
        KeyCode::Down => Some(SidebarCommand::Tree(TreeKey::Down)),
        KeyCode::Left => Some(SidebarCommand::Tree(TreeKey::Left)),
        KeyCode::Right => Some(SidebarCommand::Tree(TreeKey::Right)),
        KeyCode::Home => Some(SidebarCommand::Tree(TreeKey::Home)),
        KeyCode::End => Some(SidebarCommand::Tree(TreeKey::End)),
        KeyCode::PageUp => Some(SidebarCommand::Tree(TreeKey::PageUp(page))),
        KeyCode::PageDown => Some(SidebarCommand::Tree(TreeKey::PageDown(page))),
        KeyCode::Enter | KeyCode::Char(' ') => Some(activation(key.modifiers)),
        KeyCode::Esc => Some(SidebarCommand::Close),
        KeyCode::Tab => Some(SidebarCommand::FocusChat),
        KeyCode::Char('k') if plain => Some(SidebarCommand::Tree(TreeKey::Up)),
        KeyCode::Char('j') if plain => Some(SidebarCommand::Tree(TreeKey::Down)),
        KeyCode::Char('h') if plain => Some(SidebarCommand::Tree(TreeKey::Left)),
        KeyCode::Char('l') if plain => Some(SidebarCommand::Tree(TreeKey::Right)),
        KeyCode::Char('+') if plain => bulk(Spread::Peers, Goal::Set(BulkDirection::Expand)),
        KeyCode::Char('-') if plain => bulk(Spread::Peers, Goal::Set(BulkDirection::Collapse)),
        KeyCode::Char('*') if plain => bulk(Spread::Everything, Goal::Toggle),
        KeyCode::Char('g') if plain => Some(SidebarCommand::ToggleEditLayer),
        _ => None,
    }
}

/// Enter, Space and left clicks: the modifier decides how far the activation reaches.
/// A modifier activation changes the rows around the cursor line and leaves that line alone.
pub(super) fn activation(modifiers: KeyModifiers) -> SidebarCommand {
    if modifiers.contains(KeyModifiers::CONTROL) {
        SidebarCommand::Bulk {
            spread: Spread::Everything,
            goal: Goal::Toggle,
            anchor: BulkAnchor::Exclude,
        }
    } else if modifiers.contains(KeyModifiers::SHIFT) {
        SidebarCommand::Bulk {
            spread: Spread::Peers,
            goal: Goal::Toggle,
            anchor: BulkAnchor::Exclude,
        }
    } else {
        SidebarCommand::Activate
    }
}

/// The footer hint for the line under the cursor.
pub(super) fn hint_for(target: Option<CursorTarget>) -> &'static str {
    match target {
        Some(CursorTarget::Title(_)) => "Space folds · +/- all · * everything",
        Some(CursorTarget::Row(RowTarget::DockPlacement)) => "Space Off/Top/Bottom · ←/→ fold",
        Some(CursorTarget::Row(RowTarget::Context(_))) => "Space ON/OFF · ←/→ fold · +/- all",
        Some(CursorTarget::Row(RowTarget::Display(_))) => "Space flips · +/- all · * everything",
        None => "Esc closes",
    }
}
