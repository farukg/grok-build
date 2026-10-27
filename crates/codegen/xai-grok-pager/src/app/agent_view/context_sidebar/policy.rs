use super::*;
use crate::app::actions::ContextPolicyTransportError;
use crate::views::modal::{ModalConfirmation, ModalOption};
use crate::views::modal_window::{ModalSizing, ModalWindowConfig, ModalWindowState, render_modal_window};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use xai_grok_shell::extensions::context_policy::{ContextPolicyFailure, ContextPolicyResponse};
use xai_grok_shell::sampling::{ContextPolicyEdit, ContextPolicyMutation, ContextScope};

#[derive(Debug, Clone, Copy)]
enum Choice {
    Global,
    Session,
    Cancel,
}

#[derive(Debug)]
pub(crate) enum ExpiredConfirmation {
    Confirmed(ContextPolicyReport),
    Unavailable,
}

impl ExpiredConfirmation {
    pub(super) fn shown(&self) -> Option<&ContextPolicyReport> {
        match self { Self::Confirmed(report) => Some(report), Self::Unavailable => None }
    }

    fn dismiss(self) -> PolicyState {
        match self { Self::Confirmed(report) => PolicyState::Ready(report), Self::Unavailable => PolicyState::Unavailable }
    }

    pub(super) fn render(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let mut window = ModalWindowState::default();
        let config = ModalWindowConfig {
            title: "Context confirmation expired", tabs: None, shortcuts: &[], fold_info: None,
            sizing: ModalSizing { width_pct: 1.0, min_width: 20, v_margin: 0, h_pad: 1, v_pad: 0, footer_lines: 1, ..ModalSizing::medium() },
        };
        if let Some(content) = render_modal_window(buf, area, &mut window, &config, theme) {
            Paragraph::new("Context changed while this warning was open. Nothing from this confirmation will be applied.")
                .wrap(Wrap { trim: false }).render(content.content, buf);
            Paragraph::new("Enter / Esc: dismiss").render(content.footer, buf);
        }
    }
}

#[derive(Debug)]
pub(crate) struct Confirmation {
    session_id: agent_client_protocol::SessionId,
    edit: ContextPolicyEdit,
    selected: Choice,
}

fn choices() -> ModalConfirmation<Choice> {
    ModalConfirmation { options: vec![
        ModalOption { key: 'g', result: Choice::Global },
        ModalOption { key: 's', result: Choice::Session },
        ModalOption { key: 'c', result: Choice::Cancel },
    ] }
}

fn failure_message(failure: &ContextPolicyFailure) -> &'static str {
    match failure {
        ContextPolicyFailure::Stale => "Context changed elsewhere. Review the confirmed values and try again.",
        ContextPolicyFailure::OriginChanged => "The Context origin session changed; nothing was applied.",
        ContextPolicyFailure::StorageIo { .. } | ContextPolicyFailure::InvalidToml
        | ContextPolicyFailure::EncodeToml | ContextPolicyFailure::InvalidLegacyJson
        | ContextPolicyFailure::UnsupportedVersion { .. } => "Context storage failed. Only confirmed effective values are shown.",
        ContextPolicyFailure::RevisionExhausted => "Context change was rejected; nothing was applied.",
        ContextPolicyFailure::MailboxClosed | ContextPolicyFailure::SessionUnavailable | ContextPolicyFailure::SessionReplyLost | ContextPolicyFailure::NotLoaded => "The Context actor is unavailable. No successful application was confirmed.",
        ContextPolicyFailure::StorageWorkerLost => "Context persistence result is unknown. Only confirmed effective values are shown; no rollback was assumed.",
        ContextPolicyFailure::DurabilityUnconfirmed { .. } => "Context was published, but crash durability is unconfirmed. Only confirmed effective values are shown.",
        ContextPolicyFailure::ReplyLost | ContextPolicyFailure::Apply { .. } => "Context application is not fully confirmed. A lost reply may already have applied; no rollback was assumed.",
    }
}

impl Confirmation {
    pub(super) fn still_current(&self, report: &ContextPolicyReport) -> bool {
        report.selection.as_ref().is_some_and(|selection| selection.version == self.edit.expected)
    }

    pub(super) fn render(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let mut window = ModalWindowState::default();
        let config = ModalWindowConfig {
            title: "Confirm Context", tabs: None, shortcuts: &[], fold_info: None,
            sizing: ModalSizing { width_pct: 1.0, min_width: 20, v_margin: 0, h_pad: 1, v_pad: 0, footer_lines: 3, ..ModalSizing::medium() },
        };
        let Some(content) = render_modal_window(buf, area, &mut window, &config, theme) else { return; };
        let change = match &self.edit.mutation {
            ContextPolicyMutation::SetCategories { categories, value, .. } => {
                let value = match value { ContextSwitch::Included => "include", ContextSwitch::Excluded => "exclude" };
                match categories.single() {
                    Some(category) => format!("{value} {}", label(category)),
                    None => format!("{value} {} Context categories", categories.count()),
                }
            }
            ContextPolicyMutation::SelectScope { scope } => match scope {
                ContextScope::Global => "restore the saved Global values".to_owned(),
                ContextScope::Session => "restore this session's saved choices".to_owned(),
            },
            ContextPolicyMutation::ReplaceSession { .. } => "replace this session's choices".to_owned(),
        };
        let option = |choice: Choice, text: &str| {
            let selected = matches!((self.selected, choice), (Choice::Global, Choice::Global) | (Choice::Session, Choice::Session) | (Choice::Cancel, Choice::Cancel));
            format!("{} {text}", if selected { "›" } else { " " })
        };
        let (impact, global_label, session_label) = match self.edit.mutation {
            ContextPolicyMutation::SelectScope { .. } => (
                "Only this session changes which saved values it uses. Global values and dormant session choices are not overwritten.",
                "g  Use saved Global values", "s  Restore Session choices",
            ),
            ContextPolicyMutation::SetCategories { .. } | ContextPolicyMutation::ReplaceSession { .. } => (
                "Global edits affect following sessions and future sessions. Only this session preserves every other stored choice.",
                "g  Apply globally", "s  Only this session",
            ),
        };
        let detail = match &self.edit.mutation {
            ContextPolicyMutation::SetCategories { categories, value, .. } => match value {
                ContextSwitch::Included => "Previously excluded content becomes available to subsequent requests.",
                ContextSwitch::Excluded => match categories.single() {
                    Some(category) => consequence(category),
                    None => "Selected instructions, tools, memory or earlier messages will be omitted from subsequent requests.",
                },
            },
            ContextPolicyMutation::SelectScope { .. } => "The restored choices may change which instructions, tools and history the model receives.",
            ContextPolicyMutation::ReplaceSession { .. } => "Stored session choices will be replaced.",
        };
        let text = format!("You are about to {change}.\n\n{detail}\n\n{impact}\n\nExisting requests and running tasks keep their snapshots.\n\n↑/↓ choose · Enter confirms · Esc cancels");
        Paragraph::new(text).wrap(Wrap { trim: false }).render(content.content, buf);
        Paragraph::new(vec![
            Line::from(option(Choice::Global, global_label)),
            Line::from(option(Choice::Session, session_label)),
            Line::from(option(Choice::Cancel, "c  Cancel")),
        ]).render(content.footer, buf);
    }
}

impl AgentView {
    pub(crate) fn context_confirmation_pending(&self) -> bool {
        matches!(&self.context_sidebar, ContextSidebar::Open(open) if matches!(open.policy, PolicyState::Confirming { .. } | PolicyState::ExpiredConfirmation(_)))
    }

    pub(crate) fn context_policy_request_started(&mut self) -> Option<u64> {
        if matches!(self.context_sidebar, ContextSidebar::Closed) { return None; }
        let Some(next) = self.context_policy_generation.checked_add(1) else {
            if let ContextSidebar::Open(open) = &mut self.context_sidebar { open.policy = PolicyState::Unavailable; }
            self.show_toast("Context request identity exhausted; reload this session view.");
            return None;
        };
        self.context_policy_generation = next;
        Some(next)
    }

    pub(crate) fn context_policy_response_for(
        &mut self,
        session_id: &agent_client_protocol::SessionId,
        generation: u64,
        result: Result<ContextPolicyResponse, ContextPolicyTransportError>,
    ) {
        if self.session.session_id.as_ref() != Some(session_id)
            || self.context_policy_generation != generation
            || matches!(self.context_sidebar, ContextSidebar::Closed)
        { return; }
        self.apply_context_policy_response(result);
    }

    pub(crate) fn context_policy_notified(&mut self, response: ContextPolicyResponse) {
        let ContextSidebar::Open(open) = &self.context_sidebar else { return; };
        let incoming = match &response {
            ContextPolicyResponse::Confirmed { report } => Some(report),
            ContextPolicyResponse::Rejected { current, .. } => current.as_ref(),
        };
        if let (Some(previous), Some(incoming)) = (open.policy.shown(), incoming)
            && let (Some(old), Some(new)) = (&previous.selection, &incoming.selection)
            && new.version.revision < old.version.revision
        { return; }
        let _ = self.context_policy_request_started();
        self.apply_context_policy_response(Ok(response));
    }

    fn apply_context_policy_response(&mut self, result: Result<ContextPolicyResponse, ContextPolicyTransportError>) {
        match result {
            Ok(ContextPolicyResponse::Confirmed { report }) => self.context_policy_answered(Ok(report)),
            Ok(ContextPolicyResponse::Rejected { failure, current }) => {
                match current {
                    Some(report) => self.context_policy_answered(Ok(report)),
                    None => {
                        if let ContextSidebar::Open(open) = &mut self.context_sidebar {
                            open.policy = match open.policy {
                                PolicyState::Confirming { .. } | PolicyState::ExpiredConfirmation(_) => PolicyState::ExpiredConfirmation(ExpiredConfirmation::Unavailable),
                                PolicyState::Loading | PolicyState::Ready(_) | PolicyState::Applying { .. } | PolicyState::Unavailable => PolicyState::Unavailable,
                            };
                        }
                    }
                }
                self.show_toast(failure_message(&failure));
            }
            Err(error) => {
                self.context_policy_answered(Err(error));
                self.show_toast("Context result is unknown. Reopen F7 to read the actual state; no rollback was assumed.");
            }
        }
    }

    pub(super) fn request_context_change(&mut self, mutation: ContextPolicyMutation) -> InputOutcome {
        let Some(session_id) = self.session.session_id.clone() else { return InputOutcome::Unchanged; };
        let ContextSidebar::Open(open) = &mut self.context_sidebar else { return InputOutcome::Unchanged; };
        let shown = match open.policy.changeable().cloned() {
            Ok(shown) => shown,
            Err(reason) => { self.show_toast(reason); return InputOutcome::Changed; }
        };
        let Some(selection) = &shown.selection else {
            self.show_toast("Context scope is not confirmed by the Shell; reopen F7.");
            return InputOutcome::Changed;
        };
        let warn = match &mutation {
            ContextPolicyMutation::SelectScope { .. } => true,
            ContextPolicyMutation::SetCategories { target, .. } => *target == ContextScope::Global,
            ContextPolicyMutation::ReplaceSession { .. } => false,
        };
        let edit = ContextPolicyEdit { expected: selection.version.clone(), mutation };
        if warn {
            open.policy = PolicyState::Confirming { shown, confirmation: Confirmation { session_id, edit, selected: Choice::Cancel } };
            InputOutcome::Changed
        } else {
            open.policy = PolicyState::Applying { shown };
            InputOutcome::Action(Action::SetContextPolicy(edit))
        }
    }

    pub(super) fn toggle_context_scope(&mut self) -> InputOutcome {
        let scope = match &self.context_sidebar {
            ContextSidebar::Open(open) => open.policy.shown().and_then(|report| report.selection.as_ref()).map(|selection| selection.scope),
            ContextSidebar::Closed => None,
        };
        match scope {
            Some(ContextScope::Global) => self.request_context_change(ContextPolicyMutation::SelectScope { scope: ContextScope::Session }),
            Some(ContextScope::Session) => self.request_context_change(ContextPolicyMutation::SelectScope { scope: ContextScope::Global }),
            None => { self.show_toast("Context scope has not been confirmed by the Shell."); InputOutcome::Changed }
        }
    }

    pub(in crate::app::agent_view) fn route_context_confirmation(&mut self, event: &Event) -> Option<InputOutcome> {
        if !self.context_confirmation_pending() { return None; }
        if !self.no_input_overlay_pending() {
            if let ContextSidebar::Open(open) = &mut self.context_sidebar {
                open.policy = match std::mem::replace(&mut open.policy, PolicyState::Unavailable) {
                    PolicyState::Confirming { shown, .. } => PolicyState::Ready(shown),
                    PolicyState::ExpiredConfirmation(expired) => expired.dismiss(),
                    state @ (PolicyState::Loading | PolicyState::Ready(_) | PolicyState::Applying { .. } | PolicyState::Unavailable) => state,
                };
            }
            return None;
        }
        let ContextSidebar::Open(open) = &mut self.context_sidebar else { return None; };
        if matches!(open.policy, PolicyState::ExpiredConfirmation(_)) {
            if matches!(event, Event::Key(key) if key.kind != KeyEventKind::Release)
                || matches!(event, Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left))
            {
                if let PolicyState::ExpiredConfirmation(expired) = std::mem::replace(&mut open.policy, PolicyState::Unavailable) {
                    open.policy = expired.dismiss();
                }
            }
            return Some(InputOutcome::Changed);
        }
        let PolicyState::Confirming { confirmation, .. } = &mut open.policy else { return None; };
        if self.session.session_id.as_ref() != Some(&confirmation.session_id) {
            open.policy = PolicyState::Unavailable;
            return Some(InputOutcome::Changed);
        }
        let choice = match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Esc => Some(Choice::Cancel),
                KeyCode::Enter => Some(confirmation.selected),
                KeyCode::Char(ch) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => choices().resolve(ch).copied(),
                KeyCode::Up | KeyCode::Left => {
                    confirmation.selected = match confirmation.selected { Choice::Cancel => Choice::Session, Choice::Session => Choice::Global, Choice::Global => Choice::Cancel };
                    None
                }
                KeyCode::Down | KeyCode::Right | KeyCode::Tab => {
                    confirmation.selected = match confirmation.selected { Choice::Cancel => Choice::Global, Choice::Global => Choice::Session, Choice::Session => Choice::Cancel };
                    None
                }
                _ => None,
            },
            _ => None,
        };
        let Some(choice) = choice else { return Some(InputOutcome::Changed); };
        let PolicyState::Confirming { shown, confirmation } = std::mem::replace(&mut open.policy, PolicyState::Unavailable) else { return Some(InputOutcome::Changed); };
        if matches!(choice, Choice::Cancel) {
            open.policy = PolicyState::Ready(shown);
            return Some(InputOutcome::Changed);
        }
        let target = match choice { Choice::Global => ContextScope::Global, Choice::Session => ContextScope::Session, Choice::Cancel => { return Some(InputOutcome::Changed); } };
        let mut edit = confirmation.edit;
        match &mut edit.mutation {
            ContextPolicyMutation::SetCategories { target: scope, .. } | ContextPolicyMutation::SelectScope { scope } => *scope = target,
            ContextPolicyMutation::ReplaceSession { .. } => {},
        }
        open.policy = PolicyState::Applying { shown };
        Some(InputOutcome::Action(Action::SetContextPolicy(edit)))
    }

    pub(super) fn inspect_context_category(&mut self, category: ContextCategory) -> InputOutcome {
        let state = match &self.context_sidebar {
            ContextSidebar::Open(open) => match open.policy.shown() {
                Some(report) => {
                    let counter = report.usage.iter().find(|usage| usage.category == category)
                        .map_or_else(|| "No current token estimate was supplied.".to_owned(), |usage| format!("Estimated tokens: {}.", usage.tokens));
                    format!("{}\n\n{}\n\n{}\n\nThe current user message is always sent. Changes affect subsequent request snapshots only.",
                        if report.policy.includes(category) { "Included" } else { "Excluded" }, consequence(category), counter)
                }
                None => "Context policy and token estimate are unavailable.".to_owned(),
            },
            ContextSidebar::Closed => return InputOutcome::Unchanged,
        };
        self.show_block_viewer(crate::views::block_viewer::BlockViewerPane::for_plain_text(label(category), &state));
        InputOutcome::Changed
    }
}
