//! Aggregate parent actions through existing preference owners.

use super::gesture::Goal;
use super::groups::{Aggregate, GroupId, display_value, next_context, next_display};
use super::{AgentView, ContextSidebar, form_label, label, consequence};
use crate::app::app_view::InputOutcome;
use crate::scrollback::block::MessageKind;
use crate::scrollback::types::DisplayForm;
use crate::views::tree_nav::BulkDirection;
use xai_grok_shell::sampling::{ContextCategory, ContextPolicyCategories, ContextPolicyMutation, ContextSwitch};

impl AgentView {
    pub(super) fn activate_sidebar_group(&mut self, group: GroupId, goal: Option<Goal>) -> InputOutcome {
        match group {
            GroupId::Display(group) => {
                let value = display_value(group, self.scrollback.display_defaults());
                let form = match goal {
                    Some(Goal::Set(BulkDirection::Expand)) => DisplayForm::Expanded,
                    Some(Goal::Set(BulkDirection::Collapse)) => DisplayForm::Collapsed,
                    Some(Goal::Toggle) | None => next_display(value),
                };
                self.scrollback.choose_kind_defaults(&group.kinds().collect::<Vec<_>>(), form);
                InputOutcome::Changed
            }
            GroupId::Context(group) => {
                let ContextSidebar::Open(open) = &self.context_sidebar else { return InputOutcome::Unchanged; };
                let shown = match open.policy.changeable() {
                    Ok(shown) => shown,
                    Err(reason) => { self.show_toast(reason); return InputOutcome::Changed; }
                };
                let Some(selection) = &shown.selection else {
                    self.show_toast("Context scope has not been confirmed by the Shell.");
                    return InputOutcome::Changed;
                };
                let categories = group.categories();
                let aggregate = Aggregate::of(categories.iter().map(|&category| shown.policy.switch(category)));
                let value = match goal {
                    Some(Goal::Set(BulkDirection::Expand)) => ContextSwitch::Included,
                    Some(Goal::Set(BulkDirection::Collapse)) => ContextSwitch::Excluded,
                    Some(Goal::Toggle) | None => next_context(aggregate),
                };
                let Ok(categories) = ContextPolicyCategories::try_from(categories) else { return InputOutcome::Unchanged; };
                let target = selection.scope;
                self.request_context_change(ContextPolicyMutation::SetCategories { categories, value, target })
            }
        }
    }

    pub(super) fn inspect_sidebar_group(&mut self, group: GroupId) -> InputOutcome {
        match group {
            GroupId::Context(context) => self.inspect_context_categories(group.label(), &context.categories()),
            GroupId::Display(display) => self.inspect_display_kinds(group.label(), &display.kinds().collect::<Vec<_>>()),
        }
    }

    pub(super) fn inspect_context_categories(&mut self, title: &str, categories: &[ContextCategory]) -> InputOutcome {
        let ContextSidebar::Open(open) = &self.context_sidebar else { return InputOutcome::Unchanged; };
        let content = categories.iter().map(|&category| {
            let value = open.policy.shown().map_or("Unavailable", |report| {
                if report.policy.includes(category) { "ON" } else { "OFF" }
            });
            let counter = open.policy.shown().and_then(|report| report.usage.iter().find(|usage| usage.category == category))
                .map_or_else(|| "Token estimate unavailable".to_owned(), |usage| format!("Estimated tokens: {}", usage.tokens));
            format!("{}: {value}\n{counter}\n{}", label(category), consequence(category))
        }).collect::<Vec<_>>().join("\n\n");
        self.show_block_viewer(crate::views::block_viewer::BlockViewerPane::for_plain_text(title, &content));
        InputOutcome::Changed
    }

    pub(super) fn inspect_display_kinds(&mut self, title: &str, kinds: &[MessageKind]) -> InputOutcome {
        let content = kinds.iter().map(|&kind| {
            format!("{}: {}", kind.label(), form_label(self.scrollback.display_defaults().shown(kind)))
        }).collect::<Vec<_>>().join("\n");
        self.show_block_viewer(crate::views::block_viewer::BlockViewerPane::for_plain_text(title, &content));
        InputOutcome::Changed
    }
}
