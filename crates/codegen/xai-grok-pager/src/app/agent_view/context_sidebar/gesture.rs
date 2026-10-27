//! F7 keys share the Outline navigation owner.
//! Context mouse input selects only; Enter inspects; Space changes values.
//! Left/Right control disclosure independently of group values.

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
    /// Select and apply the stored global or session Message Display values.
    ToggleDisplayScope,
    ToggleContextScope,
    Inspect,
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
        KeyCode::Enter if key.modifiers.is_empty() => Some(SidebarCommand::Inspect),
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
        KeyCode::Char('g') if plain => Some(SidebarCommand::ToggleDisplayScope),
        KeyCode::Char('c') if plain => Some(SidebarCommand::ToggleContextScope),
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
        Some(CursorTarget::Row(RowTarget::Group(_))) => "Space group value · ←/→ fold · Enter details",
        Some(CursorTarget::Row(RowTarget::DockPlacement)) => "Space Off/Top/Bottom · ←/→ fold",
        Some(CursorTarget::Row(RowTarget::Context(_))) => "Space ON/OFF · Enter details · c scope · +/- all",
        Some(CursorTarget::Row(RowTarget::Display(_))) => "Space flips · +/- all · * everything",
        None => "Esc closes",
    }
}
