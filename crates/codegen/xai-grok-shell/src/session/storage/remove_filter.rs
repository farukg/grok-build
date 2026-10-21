use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use crate::sampling::ConversationItem;
use crate::session::{ContextItemRef, RemoveContextItemsOutcome, SkipReason, SkippedItem};

/// What one removal pass took out of the conversation and what it left in.
/// `removed` lists exactly the refs whose items are gone; every distinct requested ref is in one of the two lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationRemoval {
    pub removed: Vec<ContextItemRef>,
    pub skipped: Vec<SkippedItem>,
}

impl ConversationRemoval {
    /// The outcome of a pass that removed nothing: `WouldOrphan` when some ref matched an exchange that could not be taken out whole, else `NotFound`.
    pub(crate) fn refusal(&self) -> RemoveContextItemsOutcome {
        if self
            .skipped
            .iter()
            .any(|skipped| skipped.reason != SkipReason::NotInContext)
        {
            return RemoveContextItemsOutcome::WouldOrphan;
        }
        RemoveContextItemsOutcome::NotFound
    }
}

/// How often one tool-call id occurs in the conversation, per item shape.
#[derive(Debug, Clone, Copy, Default)]
struct ExchangeFacts {
    calls: usize,
    results: usize,
    backend_calls: usize,
}

enum ExchangeVerdict {
    ClientPair,
    BackendCall,
    Skip(SkipReason),
}

impl ExchangeFacts {
    fn of(items: &[ConversationItem], id: &str) -> Self {
        items.iter().fold(Self::default(), |mut facts, item| {
            match item {
                ConversationItem::Assistant(assistant) => {
                    facts.calls += assistant
                        .tool_calls
                        .iter()
                        .filter(|call| call.id.as_ref() == id)
                        .count();
                }
                ConversationItem::ToolResult(result) => {
                    facts.results += usize::from(result.tool_call_id == id)
                }
                ConversationItem::BackendToolCall(call) => {
                    facts.backend_calls += usize::from(call.id() == id)
                }
                ConversationItem::System(_)
                | ConversationItem::User(_)
                | ConversationItem::Reasoning(_) => {}
            }
            facts
        })
    }

    fn verdict(self) -> ExchangeVerdict {
        match (self.calls, self.results, self.backend_calls) {
            (1, 1, 0) => ExchangeVerdict::ClientPair,
            (0, 0, 1) => ExchangeVerdict::BackendCall,
            (0, 0, 0) => ExchangeVerdict::Skip(SkipReason::NotInContext),
            (1, 0, 0) => ExchangeVerdict::Skip(SkipReason::CallWithoutResult),
            _ => ExchangeVerdict::Skip(SkipReason::AmbiguousId),
        }
    }
}

/// The user item with `prompt_index` up to the next user item with a larger prompt index.
fn turn_range(items: &[ConversationItem], prompt_index: usize) -> Option<Range<usize>> {
    let start = items.iter().position(|item| matches!(item, ConversationItem::User(user) if user.prompt_index == Some(prompt_index)))?;
    let end = items
        .iter()
        .enumerate()
        .skip(start + 1)
        .find_map(|(idx, item)| match item {
            ConversationItem::User(user)
                if user.prompt_index.is_some_and(|index| index > prompt_index) =>
            {
                Some(idx)
            }
            _ => None,
        })
        .unwrap_or(items.len());
    Some(start..end)
}

/// Everything the accepted refs take out of the conversation.
#[derive(Default)]
struct RemovalPlan {
    ranges: Vec<Range<usize>>,
    tool_ids: HashSet<Arc<str>>,
    backend_ids: HashSet<String>,
}

impl RemovalPlan {
    fn add_turn(&mut self, items: &[ConversationItem], range: Range<usize>) {
        for item in items.iter().take(range.end).skip(range.start) {
            if let ConversationItem::Assistant(assistant) = item {
                self.tool_ids
                    .extend(assistant.tool_calls.iter().map(|call| call.id.clone()));
            }
        }
        self.ranges.push(range);
    }

    fn apply(&self, items: &mut Vec<ConversationItem>) {
        let mut dropped = vec![false; items.len()];
        for range in &self.ranges {
            dropped
                .iter_mut()
                .take(range.end)
                .skip(range.start)
                .for_each(|flag| *flag = true);
        }
        for (item, flag) in items.iter_mut().zip(dropped.iter_mut()) {
            match item {
                ConversationItem::ToolResult(result) => {
                    *flag |= self.tool_ids.contains(result.tool_call_id.as_str())
                }
                ConversationItem::BackendToolCall(call) => {
                    *flag |= self.backend_ids.contains(call.id())
                }
                ConversationItem::Assistant(assistant) => {
                    let original_calls = assistant.tool_calls.len();
                    assistant
                        .tool_calls
                        .retain(|call| !self.tool_ids.contains(call.id.as_ref()));
                    let removed_call = assistant.tool_calls.len() < original_calls;
                    *flag |= removed_call
                        && assistant.content.is_empty()
                        && assistant.tool_calls.is_empty();
                }
                ConversationItem::System(_)
                | ConversationItem::User(_)
                | ConversationItem::Reasoning(_) => {}
            }
        }
        self.drop_backend_reasoning(items, &mut dropped);
        drop_unanswered_reasoning(items, &mut dropped);
        let mut dropped = dropped.into_iter();
        items.retain(|_| !dropped.next().unwrap_or(false));
    }

    /// A removed backend call takes its sealed `tco_*` blob wherever it sits among the response's siblings.
    fn drop_backend_reasoning(&self, items: &[ConversationItem], dropped: &mut [bool]) {
        for (index, item) in items.iter().enumerate() {
            let ConversationItem::Reasoning(reasoning) = item else {
                continue;
            };
            if self
                .backend_ids
                .iter()
                .any(|call_id| is_sealed_blob_of(&reasoning.id, call_id))
            {
                mark(dropped, index);
            }
        }
    }
}

/// The server mints a backend call `<kind>_<response>_call-<n>` and its sealed blob `tco_<response>_call-<n>`,
/// so the blob belongs to the call whose id has the same part after the first underscore.
fn is_sealed_blob_of(reasoning_id: &str, call_id: &str) -> bool {
    let Some(blob_part) = reasoning_id.strip_prefix("tco_") else {
        return false;
    };
    call_id
        .split_once('_')
        .is_some_and(|(_, call_part)| call_part == blob_part)
}

/// A response is `[Reasoning | BackendToolCall]* Assistant`. When its assistant is dropped and no backend call of the response is kept,
/// its reasoning goes too: it would otherwise bind to the next response in the request (same rule as `ContextPolicy::project`).
fn drop_unanswered_reasoning(items: &[ConversationItem], dropped: &mut [bool]) {
    for (assistant_at, item) in items.iter().enumerate() {
        if !matches!(item, ConversationItem::Assistant(_))
            || !dropped.get(assistant_at).is_some_and(|flag| *flag)
        {
            continue;
        }
        let run = items
            .iter()
            .enumerate()
            .take(assistant_at)
            .rev()
            .take_while(|(_, sibling)| {
                matches!(
                    sibling,
                    ConversationItem::Reasoning(_) | ConversationItem::BackendToolCall(_)
                )
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let answered = run.iter().any(|index| {
            matches!(
                items.get(*index),
                Some(ConversationItem::BackendToolCall(_))
            ) && dropped.get(*index).is_some_and(|flag| !*flag)
        });
        if answered {
            continue;
        }
        for index in run
            .into_iter()
            .filter(|index| matches!(items.get(*index), Some(ConversationItem::Reasoning(_))))
        {
            mark(dropped, index);
        }
    }
}

fn mark(dropped: &mut [bool], index: usize) {
    if let Some(flag) = dropped.get_mut(index) {
        *flag = true;
    }
}

/// Classify each requested ref against the conversation, then drop exactly what the accepted refs cover.
/// `Turn` takes the user item through the next prompt-indexed user item, with the results of the calls inside it.
/// `ToolExchange` takes one call and its result, or one backend call and its `tco_*` reasoning blob; sibling calls stay.
/// A ref repeated in the request counts once.
/// Reasoning left without an output item of its response goes with it.
pub(crate) fn remove_from_conversation(
    items: &mut Vec<ConversationItem>,
    refs: &[ContextItemRef],
) -> ConversationRemoval {
    let mut plan = RemovalPlan::default();
    let mut removal = ConversationRemoval {
        removed: Vec::new(),
        skipped: Vec::new(),
    };
    let mut seen = HashSet::new();
    for reference in refs.iter().filter(|reference| seen.insert(*reference)) {
        let skip = match reference {
            ContextItemRef::Turn { prompt_index } => match turn_range(items, *prompt_index) {
                Some(range) => {
                    plan.add_turn(items, range);
                    None
                }
                None => Some(SkipReason::NotInContext),
            },
            ContextItemRef::ToolExchange { tool_call_id } => {
                match ExchangeFacts::of(items, tool_call_id.as_str()).verdict() {
                    ExchangeVerdict::ClientPair => {
                        plan.tool_ids.insert(Arc::from(tool_call_id.as_str()));
                        None
                    }
                    ExchangeVerdict::BackendCall => {
                        plan.backend_ids.insert(tool_call_id.as_str().to_owned());
                        None
                    }
                    ExchangeVerdict::Skip(reason) => Some(reason),
                }
            }
        };
        match skip {
            None => removal.removed.push(reference.clone()),
            Some(reason) => removal.skipped.push(SkippedItem {
                item: reference.clone(),
                reason,
            }),
        }
    }
    plan.apply(items);
    removal
}
