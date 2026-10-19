//! Navigation keys, bulk gestures and clicks of the outline.
//!
//! | gesture                          | reaches                                   |
//! |----------------------------------|-------------------------------------------|
//! | Shift+Enter, Shift+click         | the cursor row's siblings (same parent)   |
//! | Ctrl+Enter, Ctrl+click           | the cursor row and everything below it    |
//! | `*`                              | every branch of the outline               |
//! | `+` / `-`                        | siblings, forced open / closed            |
//! | Shift+Right / Shift+Left         | subtree, forced open / closed             |
//!
//! A toggle opens everything it reaches when any of it is closed and otherwise closes it all, so repeating it reverts it.
//! Terminals often swallow Shift+Enter, Ctrl+Enter and modified clicks; the plain keys reach the same targets.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::views::tree_nav::{BulkDirection, BulkScope, TreeKey};

/// A tree-navigation key of the panel; the page size is only known where the panel is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelNav {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
}

impl PanelNav {
    pub const fn tree_key(self, page: usize) -> TreeKey {
        match self {
            Self::Up => TreeKey::Up,
            Self::Down => TreeKey::Down,
            Self::Left => TreeKey::Left,
            Self::Right => TreeKey::Right,
            Self::Home => TreeKey::Home,
            Self::End => TreeKey::End,
            Self::PageUp => TreeKey::PageUp(page),
            Self::PageDown => TreeKey::PageDown(page),
        }
    }
}

/// What a bulk gesture does to the branches it reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkGoal {
    /// Open when any reached branch is closed, else close.
    Toggle,
    Set(BulkDirection),
}

impl BulkGoal {
    /// Why a gesture changed nothing.
    pub const fn nothing_to_do(self) -> &'static str {
        match self {
            Self::Toggle => "No folds to open or close here",
            Self::Set(BulkDirection::Expand) => "Nothing to open here",
            Self::Set(BulkDirection::Collapse) => "Nothing to close here",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BulkRequest {
    pub scope: BulkScope,
    pub goal: BulkGoal,
}

impl BulkRequest {
    pub const fn toggle(scope: BulkScope) -> Self {
        Self {
            scope,
            goal: BulkGoal::Toggle,
        }
    }

    pub const fn set(scope: BulkScope, direction: BulkDirection) -> Self {
        Self {
            scope,
            goal: BulkGoal::Set(direction),
        }
    }
}

/// A left click on an outline row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowClick {
    /// Only moves the cursor.
    Select,
    /// Opens or closes the clicked branch.
    Toggle,
    Bulk(BulkRequest),
}

impl RowClick {
    /// Ctrl reaches the subtree and Shift the siblings, anywhere on the row; a plain click toggles on the disclosure glyph only.
    pub fn of(modifiers: KeyModifiers, on_glyph: bool) -> Self {
        if modifiers.contains(KeyModifiers::CONTROL) {
            Self::Bulk(BulkRequest::toggle(BulkScope::Subtree))
        } else if modifiers.contains(KeyModifiers::SHIFT) {
            Self::Bulk(BulkRequest::toggle(BulkScope::Siblings))
        } else if on_glyph {
            Self::Toggle
        } else {
            Self::Select
        }
    }
}

/// The bulk gesture a key stands for. Other chords stay with the global routing.
pub fn bulk_request_for(key: &KeyEvent) -> Option<BulkRequest> {
    let modifiers = key.modifiers;
    let ctrl_only = modifiers == KeyModifiers::CONTROL;
    let shift_only = modifiers == KeyModifiers::SHIFT;
    let plain = modifiers.difference(KeyModifiers::SHIFT).is_empty();
    match key.code {
        KeyCode::Enter if ctrl_only => Some(BulkRequest::toggle(BulkScope::Subtree)),
        KeyCode::Enter if shift_only => Some(BulkRequest::toggle(BulkScope::Siblings)),
        KeyCode::Right if shift_only => {
            Some(BulkRequest::set(BulkScope::Subtree, BulkDirection::Expand))
        }
        KeyCode::Left if shift_only => Some(BulkRequest::set(
            BulkScope::Subtree,
            BulkDirection::Collapse,
        )),
        KeyCode::Char('*') if plain => Some(BulkRequest::toggle(BulkScope::All)),
        KeyCode::Char('+') if plain => {
            Some(BulkRequest::set(BulkScope::Siblings, BulkDirection::Expand))
        }
        KeyCode::Char('-') if plain => Some(BulkRequest::set(
            BulkScope::Siblings,
            BulkDirection::Collapse,
        )),
        _ => None,
    }
}
