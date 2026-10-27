//! Compact semantic parents using the shared sidebar renderer.

use super::groups::{Aggregate, GroupId, display_value};
use super::{ContextPolicyReport, RowTarget, SectionDisclosure, SectionId, form_label};
use crate::scrollback::block::{DisplayDefaults, MessageKind};
use crate::theme::Theme;
use crate::views::context_bar::fmt_tokens;
use crate::views::row_style::{Disclosure, SidebarRowClass, sidebar_style};
use crate::views::sidebar::SidebarRow;
use ratatui::text::{Line, Span};
use xai_grok_shell::sampling::{ContextCategory, ContextSwitch};

pub(super) fn grouped<'a>(
    section: SectionId,
    disclosure: SectionDisclosure,
    leaves: Vec<SidebarRow<'a>>,
    categories: &[ContextCategory],
    report: Option<&ContextPolicyReport>,
    defaults: &DisplayDefaults,
    theme: &Theme,
) -> Vec<SidebarRow<'a>> {
    super::outline::section_targets(disclosure, section).into_iter().filter_map(|target| {
        match target {
            RowTarget::Group(group) => Some(parent(group, disclosure, report, defaults, theme)),
            RowTarget::Context(category) => categories.iter().position(|&key| key == category)
                .and_then(|index| leaves.get(index)).cloned().map(indent),
            RowTarget::Display(kind) => MessageKind::settings_kinds().iter().position(|&key| key == kind)
                .and_then(|index| leaves.get(index)).cloned().map(indent),
            RowTarget::DockPlacement => None,
        }
    }).collect()
}

fn indent(mut row: SidebarRow<'_>) -> SidebarRow<'_> {
    row.left.spans.insert(0, Span::raw("  "));
    row
}

fn parent<'a>(
    group: GroupId,
    disclosure: SectionDisclosure,
    report: Option<&ContextPolicyReport>,
    defaults: &DisplayDefaults,
    theme: &Theme,
) -> SidebarRow<'a> {
    let (value, class) = match group {
        GroupId::Display(group) => match display_value(group, defaults) {
            Aggregate::Uniform(form) => (form_label(form).to_owned(), SidebarRowClass::from(form)),
            Aggregate::Mixed => ("Mixed".to_owned(), SidebarRowClass::FormOneLine),
            Aggregate::Unavailable => ("Unavailable".to_owned(), SidebarRowClass::Unavailable),
        },
        GroupId::Context(group) => {
            let categories = group.categories();
            let value = report.map_or(Aggregate::Unavailable, |report| {
                Aggregate::of(categories.iter().map(|&category| report.policy.switch(category)))
            });
            let (state, class) = match value {
                Aggregate::Uniform(ContextSwitch::Included) => ("ON", SidebarRowClass::SwitchOn),
                Aggregate::Uniform(ContextSwitch::Excluded) => ("OFF", SidebarRowClass::SwitchOff),
                Aggregate::Mixed => ("Mixed", SidebarRowClass::FormOneLine),
                Aggregate::Unavailable => ("Unavailable", SidebarRowClass::Unavailable),
            };
            let tokens = report.and_then(|report| categories.iter().try_fold(0u64, |total, category| {
                let usage = report.usage.iter().find(|usage| usage.category == *category)?;
                total.checked_add(usage.tokens)
            })).map_or_else(|| "?".to_owned(), fmt_tokens);
            (format!("~{tokens} {state}"), class)
        }
    };
    let style = sidebar_style(class, theme);
    SidebarRow {
        left: Line::from(vec![
            Span::raw("  "),
            Span::styled(Disclosure::of_branch(Some(disclosure.group_expanded(group))).glyph(), theme.muted()),
            Span::raw(" "),
            Span::styled(group.label(), theme.muted()),
        ]),
        right: Some(Line::from(Span::styled(value, style.value))),
        detail: &[],
    }
}
