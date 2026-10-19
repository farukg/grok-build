//! Row styling vocabulary for the F6 timeline outline and the F7 context sidebar.
//!
//! Pure mapping from row semantics to [`Style`]; every color is read from the [`Theme`], so the rows follow the active palette.

use ratatui::style::{Modifier, Style};

use crate::glyphs;
use crate::scrollback::block::RenderBlock;
use crate::scrollback::blocks::{BgTaskKind, SubagentBlockKind, WorkflowBlockStatus};
use crate::scrollback::types::DisplayForm;
use crate::theme::Theme;

/// Branch marker of a tree row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disclosure {
    Expanded,
    Collapsed,
    Leaf,
}

impl Disclosure {
    /// Maps the `Some(expanded)` / `None` branch encoding of `tree_nav::TreeRow`.
    pub const fn of_branch(branch: Option<bool>) -> Self {
        match branch {
            Some(true) => Self::Expanded,
            Some(false) => Self::Collapsed,
            None => Self::Leaf,
        }
    }

    /// `▾` / `▸` (ASCII fallback on legacy Windows consoles) and `·` for a leaf.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Expanded => glyphs::disclosure_open(),
            Self::Collapsed => glyphs::disclosure_closed(),
            Self::Leaf => "\u{00B7}",
        }
    }
}

/// Semantic class of an F6 outline row; decides the row's color and weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
pub enum OutlineClass {
    Turn,
    Group,
    UserPrompt,
    AgentMessage,
    Thinking,
    ToolCall,
    /// A tool call, background task, subagent or workflow that ended in failure.
    Failed,
    /// Notes the harness writes: system text, session events, context snapshots.
    SystemNote,
    /// Subagent or workflow run.
    Delegated,
    BackgroundTask,
}

impl OutlineClass {
    pub fn of_block(block: &RenderBlock) -> Self {
        match block {
            RenderBlock::UserPrompt(_) => Self::UserPrompt,
            RenderBlock::AgentMessage(_) | RenderBlock::Btw(_) => Self::AgentMessage,
            RenderBlock::Thinking(_) => Self::Thinking,
            RenderBlock::ToolCall(tool) => {
                if tool.is_success() {
                    Self::ToolCall
                } else {
                    Self::Failed
                }
            }
            RenderBlock::Subagent(block) => match block.kind {
                SubagentBlockKind::Failed { .. } => Self::Failed,
                SubagentBlockKind::Started
                | SubagentBlockKind::Completed { .. }
                | SubagentBlockKind::Cancelled { .. } => Self::Delegated,
            },
            RenderBlock::Workflow(block) => match block.status {
                WorkflowBlockStatus::Failed { .. } => Self::Failed,
                WorkflowBlockStatus::Running
                | WorkflowBlockStatus::Done { .. }
                | WorkflowBlockStatus::Cancelled { .. }
                | WorkflowBlockStatus::Paused { .. } => Self::Delegated,
            },
            RenderBlock::BgTask(block) => match block.kind {
                BgTaskKind::Failed { .. } => Self::Failed,
                BgTaskKind::Started | BgTaskKind::Completed { .. } => Self::BackgroundTask,
            },
            RenderBlock::Stub(_)
            | RenderBlock::System(_)
            | RenderBlock::SessionEvent(_)
            | RenderBlock::ContextInfo(_)
            | RenderBlock::MemoryCapture(_) => Self::SystemNote,
        }
    }

    /// Structure rows are neutral; entries carry the color so a glance tells kinds apart.
    fn base(self, theme: &Theme) -> Style {
        match self {
            Self::Turn => theme.primary().add_modifier(Modifier::BOLD),
            Self::Group => theme.muted().add_modifier(Modifier::BOLD),
            Self::UserPrompt => theme.primary(),
            Self::AgentMessage => theme.fg(theme.accent_assistant),
            Self::Thinking => theme.fg(theme.gray_bright).add_modifier(Modifier::ITALIC),
            Self::ToolCall => theme.fg(theme.accent_success),
            Self::Failed => theme.fg(theme.accent_error).add_modifier(Modifier::BOLD),
            Self::SystemNote => theme.muted(),
            Self::Delegated => theme.fg(theme.accent_model),
            Self::BackgroundTask => theme.fg(theme.warning),
        }
    }
}

/// Whether the model sees an entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ContextPresence {
    /// Part of the context the model receives.
    #[default]
    InContext,
    /// Shown to the user only.
    DisplayOnly,
    /// Dropped from the model context by the user.
    Removed,
}

impl ContextPresence {
    pub fn marker_glyph(self) -> &'static str {
        match self {
            Self::InContext => glyphs::filled_dot(),
            Self::DisplayOnly => "\u{25CB}",
            Self::Removed => "\u{2298}",
        }
    }

    fn marker_style(self, theme: &Theme) -> Style {
        match self {
            Self::InContext => theme.fg(theme.accent_success),
            Self::DisplayOnly => theme.dim(),
            Self::Removed => theme.fg(theme.accent_error).add_modifier(Modifier::DIM),
        }
    }
}

/// State of an outline row that is independent of its class.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RowState {
    pub running: bool,
    pub presence: ContextPresence,
}

/// Styles for the parts of one outline row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutlineStyle {
    /// Disclosure glyph.
    pub glyph: Style,
    pub label: Style,
    /// Trailing detail such as size or duration.
    pub meta: Style,
    /// [`ContextPresence::marker_glyph`].
    pub marker: Style,
}

/// Style an outline row. Cursor and hover overlays are applied by the caller on top.
pub fn outline_style(class: OutlineClass, state: RowState, theme: &Theme) -> OutlineStyle {
    let mut label = class.base(theme);
    let mut meta = theme.muted();
    if state.running {
        label = label.fg(theme.running).add_modifier(Modifier::BOLD);
        meta = theme.fg(theme.running);
    }
    OutlineStyle {
        glyph: fade(label, state.presence, Modifier::DIM),
        label: fade(label, state.presence, Modifier::DIM | Modifier::CROSSED_OUT),
        meta: fade(meta, state.presence, Modifier::DIM | Modifier::CROSSED_OUT),
        marker: state.presence.marker_style(theme),
    }
}

/// Removed rows get `removal`; the glyph stays unstruck so the tree structure remains readable.
fn fade(style: Style, presence: ContextPresence, removal: Modifier) -> Style {
    match presence {
        ContextPresence::Removed => style.add_modifier(removal),
        ContextPresence::InContext | ContextPresence::DisplayOnly => style,
    }
}

/// Semantic class of an F7 sidebar row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
pub enum SidebarRowClass {
    SectionTitle,
    SwitchOn,
    SwitchOff,
    /// Message Display row showing the full form.
    FormExpanded,
    /// Message Display row showing the one-line form.
    FormOneLine,
    /// A row the user cannot change right now.
    Unavailable,
}

impl From<DisplayForm> for SidebarRowClass {
    fn from(form: DisplayForm) -> Self {
        match form {
            DisplayForm::Expanded => Self::FormExpanded,
            DisplayForm::Collapsed => Self::FormOneLine,
        }
    }
}

impl SidebarRowClass {
    pub fn glyph(self) -> Option<&'static str> {
        match self {
            Self::SectionTitle => None,
            Self::SwitchOn => Some(glyphs::filled_dot()),
            Self::SwitchOff => Some("\u{25CB}"),
            Self::FormExpanded => Some(Disclosure::Expanded.glyph()),
            Self::FormOneLine => Some(Disclosure::Collapsed.glyph()),
            Self::Unavailable => Some("\u{2013}"),
        }
    }
}

/// Styles for the parts of one sidebar row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarStyle {
    pub glyph: Style,
    /// Left text.
    pub label: Style,
    /// Right text: state or token count.
    pub value: Style,
}

pub fn sidebar_style(class: SidebarRowClass, theme: &Theme) -> SidebarStyle {
    match class {
        SidebarRowClass::SectionTitle => {
            let title = theme.fg(theme.accent_system).add_modifier(Modifier::BOLD);
            SidebarStyle {
                glyph: title,
                label: title,
                value: theme.muted(),
            }
        }
        SidebarRowClass::SwitchOn => {
            let on = theme.fg(theme.accent_success);
            SidebarStyle {
                glyph: on,
                label: theme.primary(),
                value: on.add_modifier(Modifier::BOLD),
            }
        }
        SidebarRowClass::SwitchOff => {
            let off = theme.fg(theme.warning);
            SidebarStyle {
                glyph: off,
                label: theme.fg(theme.text_secondary),
                value: off,
            }
        }
        SidebarRowClass::FormExpanded => {
            let form = theme.fg(theme.accent_system);
            SidebarStyle {
                glyph: form,
                label: theme.primary(),
                value: form,
            }
        }
        SidebarRowClass::FormOneLine => {
            let form = theme.fg(theme.gray_bright);
            SidebarStyle {
                glyph: form,
                label: theme.primary(),
                value: form,
            }
        }
        SidebarRowClass::Unavailable => {
            let off = theme.muted().add_modifier(Modifier::DIM);
            SidebarStyle {
                glyph: off,
                label: off,
                value: off,
            }
        }
    }
}

#[cfg(test)]
#[path = "row_style_tests.rs"]
mod tests;
