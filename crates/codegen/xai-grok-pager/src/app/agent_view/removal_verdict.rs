//! What removing a timeline row from the model context means, or why it cannot be done.
//! The decision is pure: callers gather [`RowFacts`] once and read the verdict.

use super::AgentView;
use crate::scrollback::block::{MessageKind, RenderBlock};
use crate::scrollback::blocks::{SessionEvent, ToolCallBlock};
use crate::scrollback::entry::ScrollbackEntry;
use crate::scrollback::state::{TimelineRow, TimelineRowKind};
use std::iter;
use xai_grok_shell::session::{ContextItemRef, ToolCallId};

/// Everything the verdict needs to know about one scrollback entry.
#[derive(Debug, Clone)]
pub(crate) struct EntryFacts {
    pub kind: MessageKind,
    /// Shell prompt number; only numbered `UserPrompt` entries carry one.
    pub prompt_index: Option<usize>,
    pub is_interjection: bool,
    /// A `!` shell command: the user prompt or its execute row.
    pub is_bash: bool,
    pub is_running: bool,
    /// Context id the tracker registered for a tool row.
    pub tool_id: Option<ToolCallId>,
    /// A `SessionEvent` reporting finished compaction.
    pub is_compaction_event: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum Members {
    One(EntryFacts),
    /// A fold group is never empty, so its head is kept apart from the rest.
    Group {
        first: EntryFacts,
        rest: Vec<EntryFacts>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct RowFacts {
    pub has_session: bool,
    pub members: Members,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryRefusal {
    PromptNotNumbered,
    Interjection,
    BashCommand,
    ModelReply,
    Reasoning,
    ToolStillRunning,
    ToolNotTracked,
    BackgroundOrSubagent,
    UiOnly,
    CompactionSummary,
}

impl EntryRefusal {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::PromptNotNumbered => {
                "This prompt has no context id yet (sent with send-now, or the session predates numbering)"
            }
            Self::Interjection => {
                "Interjections cannot be removed separately; remove the whole turn"
            }
            Self::BashCommand => {
                "Shell commands (!) are stored as one summary the model sees; they cannot be addressed yet"
            }
            Self::ModelReply => {
                "Agent replies cannot be removed one by one; remove the turn, or switch off 'Agent replies' in F7"
            }
            Self::Reasoning => {
                "Reasoning cannot be removed one by one; switch off 'Reasoning' in F7"
            }
            Self::ToolStillRunning => "The tool call is still running",
            Self::ToolNotTracked => {
                "This tool call has no context id (hidden helper tool, or merged edit)"
            }
            Self::BackgroundOrSubagent => {
                "Background tasks and subagents are not separate context items; remove the spawning turn"
            }
            Self::UiOnly => "Shown here only; the model never sees this entry",
            Self::CompactionSummary => {
                "This is the summary of compacted history; switch off 'Compaction summary' in F7 to drop it"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NotRemovable {
    NoSession,
    /// The outline row no longer resolves to a scrollback entry.
    RowGone,
    Entry(EntryRefusal),
    /// Every member of a fold group is refused; `first` is the first member's reason.
    Group {
        members: usize,
        first: EntryRefusal,
    },
}

impl NotRemovable {
    pub(crate) fn text(&self) -> String {
        match self {
            Self::NoSession => "Session is not connected".to_owned(),
            Self::RowGone => "This entry no longer exists".to_owned(),
            Self::Entry(refusal) => refusal.text().to_owned(),
            Self::Group { members: 1, first } => {
                format!(
                    "The entry in this group cannot be removed: {}",
                    first.text()
                )
            }
            Self::Group { members, first } => format!(
                "None of the {} in this group can be removed: {}",
                counted(*members, "entry", "entries"),
                first.text()
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RemovalVerdict {
    /// `skipped` counts group members left out because they cannot be removed.
    Removable {
        refs: Vec<ContextItemRef>,
        skipped: usize,
    },
    NotRemovable(NotRemovable),
}

/// "1 entry" / "3 entries": the count with the noun in the right number.
pub(crate) fn counted(count: usize, one: &str, many: &str) -> String {
    match count {
        1 => format!("1 {one}"),
        count => format!("{count} {many}"),
    }
}

/// "1 item" / "3 items".
pub(crate) fn items_phrase(count: usize) -> String {
    counted(count, "item", "items")
}

/// Shortcuts-bar wording ("press again to {label}") for what confirming `refs` does.
pub(crate) fn confirm_label(refs: &[ContextItemRef]) -> &'static str {
    match refs {
        [ContextItemRef::Turn { .. }] => "remove this turn from context",
        [ContextItemRef::ToolExchange { .. }] => "remove this tool call from context",
        _ => "remove these items from context",
    }
}

fn entry_ref(entry: &EntryFacts) -> Result<ContextItemRef, EntryRefusal> {
    match entry.kind {
        MessageKind::UserPrompt => prompt_ref(entry),
        MessageKind::Execute
        | MessageKind::Read
        | MessageKind::Edit
        | MessageKind::ListDir
        | MessageKind::Search
        | MessageKind::WebFetch
        | MessageKind::WebSearch
        | MessageKind::IntegrationSearch
        | MessageKind::UseTool
        | MessageKind::MemorySearch
        | MessageKind::SentMessage
        | MessageKind::Skill
        | MessageKind::OtherTool => tool_ref(entry),
        MessageKind::AgentMessage => Err(EntryRefusal::ModelReply),
        MessageKind::Thinking => Err(EntryRefusal::Reasoning),
        MessageKind::BgTask | MessageKind::Subagent | MessageKind::Workflow => {
            Err(EntryRefusal::BackgroundOrSubagent)
        }
        MessageKind::SessionEvent if entry.is_compaction_event => {
            Err(EntryRefusal::CompactionSummary)
        }
        MessageKind::SessionEvent
        | MessageKind::System
        | MessageKind::Btw
        | MessageKind::ContextInfo
        | MessageKind::MemoryCapture
        | MessageKind::Stub => Err(EntryRefusal::UiOnly),
    }
}

fn prompt_ref(entry: &EntryFacts) -> Result<ContextItemRef, EntryRefusal> {
    match (entry.is_interjection, entry.is_bash, entry.prompt_index) {
        (true, _, _) => Err(EntryRefusal::Interjection),
        (false, true, _) => Err(EntryRefusal::BashCommand),
        (false, false, Some(prompt_index)) => Ok(ContextItemRef::Turn { prompt_index }),
        (false, false, None) => Err(EntryRefusal::PromptNotNumbered),
    }
}

fn tool_ref(entry: &EntryFacts) -> Result<ContextItemRef, EntryRefusal> {
    match (entry.is_bash, entry.is_running, &entry.tool_id) {
        (true, _, _) => Err(EntryRefusal::BashCommand),
        (false, true, _) => Err(EntryRefusal::ToolStillRunning),
        (false, false, Some(tool_call_id)) => Ok(ContextItemRef::ToolExchange {
            tool_call_id: tool_call_id.clone(),
        }),
        (false, false, None) => Err(EntryRefusal::ToolNotTracked),
    }
}

pub(crate) fn removal_verdict(row: &RowFacts) -> RemovalVerdict {
    if !row.has_session {
        return RemovalVerdict::NotRemovable(NotRemovable::NoSession);
    }
    match &row.members {
        Members::One(entry) => match entry_ref(entry) {
            Ok(item) => RemovalVerdict::Removable {
                refs: vec![item],
                skipped: 0,
            },
            Err(refusal) => RemovalVerdict::NotRemovable(NotRemovable::Entry(refusal)),
        },
        Members::Group { first, rest } => {
            let mut refs = Vec::new();
            let mut refusals = Vec::new();
            for member in iter::once(first).chain(rest) {
                match entry_ref(member) {
                    Ok(item) => refs.push(item),
                    Err(refusal) => refusals.push(refusal),
                }
            }
            match (refs.is_empty(), refusals.first().copied()) {
                (true, Some(first)) => RemovalVerdict::NotRemovable(NotRemovable::Group {
                    members: refusals.len(),
                    first,
                }),
                // Unreachable: the head member always yields a ref or a refusal.
                (true, None) => RemovalVerdict::NotRemovable(NotRemovable::RowGone),
                (false, _) => RemovalVerdict::Removable {
                    refs,
                    skipped: refusals.len(),
                },
            }
        }
    }
}

impl AgentView {
    /// Verdict for removing the outline row from the model context.
    pub(crate) fn verdict_for_row(&self, row: TimelineRow) -> RemovalVerdict {
        match self.removal_members(row) {
            Some(members) => removal_verdict(&RowFacts {
                has_session: self.session.session_id.is_some(),
                members,
            }),
            None => RemovalVerdict::NotRemovable(NotRemovable::RowGone),
        }
    }

    fn removal_members(&self, row: TimelineRow) -> Option<Members> {
        match row.kind {
            TimelineRowKind::Turn { .. } | TimelineRowKind::Entry { .. } => Some(Members::One(
                self.removal_entry_facts(self.scrollback.get_by_id(row.entry_id())?),
            )),
            TimelineRowKind::Group { first_id } => {
                let first = self.scrollback.index_of_id(first_id)?;
                let range = self.scrollback.span_at(first)?.range.clone();
                let mut facts = range
                    .filter_map(|idx| self.scrollback.get(idx))
                    .map(|entry| self.removal_entry_facts(entry));
                Some(Members::Group {
                    first: facts.next()?,
                    rest: facts.collect(),
                })
            }
        }
    }

    pub(super) fn removal_entry_facts(&self, entry: &ScrollbackEntry) -> EntryFacts {
        let mut facts = EntryFacts {
            kind: MessageKind::from(&entry.block),
            prompt_index: None,
            is_interjection: false,
            is_bash: false,
            is_running: entry.is_running,
            tool_id: None,
            is_compaction_event: false,
        };
        match &entry.block {
            RenderBlock::UserPrompt(prompt) => {
                facts.prompt_index = prompt.prompt_index;
                facts.is_interjection = prompt.is_interjection;
                facts.is_bash = prompt.is_bash;
            }
            RenderBlock::ToolCall(tool) => {
                facts.tool_id = self
                    .session
                    .tracker
                    .context_tool_call_id_for_entry(entry.id);
                facts.is_bash =
                    matches!(tool, ToolCallBlock::Execute(execute) if execute.bash_mode);
            }
            RenderBlock::SessionEvent(block) => {
                facts.is_compaction_event = matches!(
                    block.event,
                    SessionEvent::CompactionCompleted { .. }
                        | SessionEvent::CompactCompleted { .. }
                );
            }
            RenderBlock::Stub(_)
            | RenderBlock::AgentMessage(_)
            | RenderBlock::Thinking(_)
            | RenderBlock::System(_)
            | RenderBlock::BgTask(_)
            | RenderBlock::Subagent(_)
            | RenderBlock::Workflow(_)
            | RenderBlock::Btw(_)
            | RenderBlock::ContextInfo(_)
            | RenderBlock::MemoryCapture(_) => {}
        }
        facts
    }
}

#[cfg(test)]
#[path = "removal_verdict_tests.rs"]
mod tests;
