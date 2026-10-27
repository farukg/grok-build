//! Coloured lines of the F7 tree: section titles with their disclosure glyph, and leaf labels one level below.

use super::outline::SectionId;
use crate::scrollback::block::{DisplayDefaults, MessageKind};
use crate::scrollback::types::DisplayForm;
use crate::theme::Theme;
use crate::views::row_style::{Disclosure, SidebarRowClass, SidebarStyle, sidebar_style};
use ratatui::text::{Line, Span};
use xai_grok_shell::extensions::context_policy::ContextPolicyReport;
use xai_grok_shell::sampling::ContextCategory;

/// `▾ Title  summary`; the summary stays visible while the section is folded.
pub(super) fn title_line(
    section: SectionId,
    expanded: bool,
    summary: Option<String>,
    theme: &Theme,
) -> Line<'static> {
    let style = sidebar_style(SidebarRowClass::SectionTitle, theme);
    let glyph = Disclosure::of_branch(Some(expanded)).glyph();
    let mut spans = vec![
        Span::styled(glyph.to_string(), style.glyph),
        Span::raw(" "),
        Span::styled(section.title(), style.label),
    ];
    if let Some(summary) = summary {
        spans.push(Span::styled(format!("  {summary}"), style.value));
    }
    Line::from(spans)
}

/// `  ● label`, one level below its section title.
pub(super) fn leaf_label(
    class: SidebarRowClass,
    style: SidebarStyle,
    label: &'static str,
) -> Line<'static> {
    let glyph = class.glyph().unwrap_or(" ");
    Line::from(vec![
        Span::raw("  "),
        Span::styled(glyph.to_string(), style.glyph),
        Span::raw(" "),
        Span::styled(label, style.label),
    ])
}

/// `N/M ON` once the shell has answered.
pub(super) fn context_summary(
    report: Option<&ContextPolicyReport>,
    categories: &[ContextCategory],
) -> Option<String> {
    let report = report?;
    let on = categories
        .iter()
        .filter(|&&category| report.policy.includes(category))
        .count();
    let scope = match report.selection.as_ref().map(|selection| selection.scope) {
        Some(xai_grok_shell::sampling::ContextScope::Global) => "Global · c",
        Some(xai_grok_shell::sampling::ContextScope::Session) => "Session · c",
        None => "scope unavailable",
    };
    Some(format!("{scope} · {on}/{} ON", categories.len()))
}

/// `N/M full`: how many message kinds show their full form.
pub(super) fn display_summary(defaults: &DisplayDefaults) -> String {
    let kinds: Vec<_> = MessageKind::settings_kinds().iter().copied().filter(|&kind| kind != MessageKind::Injected).collect();
    let full = kinds.iter().filter(|&&kind| defaults.shown(kind) == DisplayForm::Expanded).count();
    format!("{full}/{} full", kinds.len())
}
