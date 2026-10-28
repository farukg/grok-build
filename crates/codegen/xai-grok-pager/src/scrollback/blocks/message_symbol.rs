use crate::appearance::ToolBullet;
use crate::scrollback::block::{MessageKind, RenderBlock};
use crate::scrollback::types::{AccentStyle, BlockContext, ToolCompletion};
use crate::theme::{Theme, color_support};

use super::{BgTaskKind, SessionEvent, SubagentBlockKind, WorkflowBlockStatus};
use super::tool::{SentMessageDelivery, SentMessagePresentation, ToolCallBlock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MessageStatus {
    Information,
    Running,
    Done,
    Finished,
    Failed,
    Cancelled,
    Paused,
    Queued,
    Unavailable,
    Unconfirmed,
    Inactive,
}

impl MessageStatus {
    fn label(self) -> Option<&'static str> {
        match self {
            Self::Information => None,
            Self::Running => Some("running"),
            Self::Done | Self::Finished => Some("done"),
            Self::Failed => Some("failed"),
            Self::Cancelled => Some("cancelled"),
            Self::Paused => Some("paused"),
            Self::Queued => Some("queued"),
            Self::Unavailable => Some("unavailable"),
            Self::Unconfirmed => Some("unconfirmed"),
            Self::Inactive => Some("inactive"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum SymbolLayout {
    Header,
    Body,
}

pub(crate) struct MessageSymbol {
    glyph: &'static str,
    kind: MessageKind,
    status: MessageStatus,
    pub(crate) layout: SymbolLayout,
}

impl MessageSymbol {
    fn configured_glyph(&self, ctx: &BlockContext) -> Option<&'static str> {
        match self.kind {
            MessageKind::UserPrompt => Some(self.glyph),
            MessageKind::AgentMessage | MessageKind::Execute | MessageKind::Read
            | MessageKind::Edit | MessageKind::ListDir | MessageKind::Search
            | MessageKind::WebFetch | MessageKind::WebSearch | MessageKind::IntegrationSearch
            | MessageKind::UseTool | MessageKind::MemorySearch | MessageKind::SentMessage
            | MessageKind::Skill | MessageKind::OtherTool | MessageKind::Thinking | MessageKind::System
            | MessageKind::SessionEvent | MessageKind::BgTask | MessageKind::Subagent
            | MessageKind::Workflow | MessageKind::Btw | MessageKind::ContextInfo
            | MessageKind::MemoryCapture | MessageKind::Injected | MessageKind::InjectedOther
            | MessageKind::InjectedInstructions | MessageKind::InjectedContinuation
            | MessageKind::InjectedNotification | MessageKind::InjectedMemory
            | MessageKind::InjectedSummary | MessageKind::InjectedEnvironment
            | MessageKind::InjectedConversation | MessageKind::Stub => match ctx.appearance.scrollback.blocks.tool.bullet {
                ToolBullet::Category => Some(self.glyph),
                ToolBullet::None | ToolBullet::Dot | ToolBullet::SmallCircle | ToolBullet::Circle
                | ToolBullet::SmallTriangle | ToolBullet::Triangle | ToolBullet::Diamond => {
                    ctx.appearance.scrollback.blocks.tool.bullet.char()
                }
            },
        }
    }

    pub(crate) fn is_visible(&self, ctx: &BlockContext) -> bool {
        self.configured_glyph(ctx).is_some_and(|glyph| {
            unicode_width::UnicodeWidthStr::width(glyph) <= usize::from(ctx.width)
        })
    }

    pub(crate) fn prefix(&self, ctx: &BlockContext) -> String {
        let Some(glyph) = self.configured_glyph(ctx) else {
            return String::new();
        };
        let prefix = match (color_support::get(), self.status.label()) {
            (color_support::ColorLevel::None, Some(label)) => format!("{glyph} [{label}] "),
            (color_support::ColorLevel::None, None)
            | (color_support::ColorLevel::Basic | color_support::ColorLevel::Ansi256
                | color_support::ColorLevel::TrueColor, Some(_))
            | (color_support::ColorLevel::Basic | color_support::ColorLevel::Ansi256
                | color_support::ColorLevel::TrueColor, None) => format!("{glyph} "),
        };
        use unicode_width::UnicodeWidthStr;
        if prefix.width() <= usize::from(ctx.width) {
            prefix
        } else if glyph.width() <= usize::from(ctx.width) {
            glyph.to_owned()
        } else {
            String::new()
        }
    }

    pub(crate) fn style(&self, ctx: &BlockContext) -> Option<AccentStyle> {
        let theme = Theme::current();
        match self.status {
            MessageStatus::Information if self.kind == MessageKind::UserPrompt => {
                let color = match theme.accent_user {
                    ratatui::style::Color::Reset if crate::theme::cache::terminal_native_locked() => ratatui::style::Color::Cyan,
                    color => color,
                };
                Some(AccentStyle::static_color(color))
            }
            MessageStatus::Information | MessageStatus::Inactive | MessageStatus::Finished => None,
            MessageStatus::Running => Some(match self.kind {
                MessageKind::Thinking => {
                    let config = &ctx.appearance.scrollback.blocks.thinking;
                    if config.animate { AccentStyle::animated(config.accent) } else { AccentStyle::static_color(config.accent) }
                }
                MessageKind::Execute => AccentStyle::animated(ctx.appearance.scrollback.blocks.execute.running_accent),
                MessageKind::SessionEvent => AccentStyle::animated(theme.gray),
                MessageKind::UserPrompt | MessageKind::AgentMessage | MessageKind::Read
                | MessageKind::Edit | MessageKind::ListDir | MessageKind::Search
                | MessageKind::WebFetch | MessageKind::WebSearch | MessageKind::IntegrationSearch
                | MessageKind::UseTool | MessageKind::MemorySearch | MessageKind::SentMessage
                | MessageKind::Skill | MessageKind::OtherTool | MessageKind::System
                | MessageKind::BgTask | MessageKind::Subagent
                | MessageKind::Workflow | MessageKind::Btw | MessageKind::ContextInfo
                | MessageKind::MemoryCapture | MessageKind::Injected | MessageKind::InjectedOther
                | MessageKind::InjectedInstructions | MessageKind::InjectedContinuation
                | MessageKind::InjectedNotification | MessageKind::InjectedMemory
                | MessageKind::InjectedSummary | MessageKind::InjectedEnvironment
                | MessageKind::InjectedConversation | MessageKind::Stub => AccentStyle::animated_running(ctx, &theme),
            }),
            MessageStatus::Done | MessageStatus::Queued => Some(AccentStyle::static_color(theme.accent_success)),
            MessageStatus::Failed => Some(AccentStyle::static_color(theme.accent_error)),
            MessageStatus::Cancelled => Some(AccentStyle::static_color(theme.gray_dim)),
            MessageStatus::Paused | MessageStatus::Unavailable | MessageStatus::Unconfirmed => {
                Some(AccentStyle::static_color(theme.warning))
            }
        }
    }
}

impl MessageKind {
    pub(crate) fn symbol_glyph(self) -> &'static str {
        use crate::glyphs;
        match self {
            Self::UserPrompt => glyphs::history_user(),
            Self::AgentMessage => glyphs::history_assistant(),
            Self::Execute => glyphs::tool_execute(),
            Self::Read => glyphs::tool_read_file(),
            Self::Edit => glyphs::tool_edit_file(),
            Self::ListDir => glyphs::tool_list_dir(),
            Self::Search => glyphs::tool_search(),
            Self::WebFetch => glyphs::tool_web_fetch(),
            Self::WebSearch => glyphs::tool_web_search(),
            Self::IntegrationSearch => glyphs::tool_integration_search(),
            Self::UseTool => glyphs::history_integration(),
            Self::MemorySearch | Self::MemoryCapture | Self::InjectedMemory => glyphs::tool_memory_search(),
            Self::SentMessage => glyphs::history_message(),
            Self::Skill => glyphs::tool_skill(),
            Self::OtherTool => glyphs::history_tool(),
            Self::Thinking => glyphs::history_thinking(),
            Self::System | Self::SessionEvent => glyphs::history_system(),
            Self::BgTask => glyphs::history_task(),
            Self::Subagent => glyphs::history_subagent(),
            Self::Workflow => glyphs::history_workflow(),
            Self::Btw => glyphs::history_btw(),
            Self::ContextInfo | Self::InjectedSummary => glyphs::history_summary(),
            Self::Injected | Self::InjectedOther => glyphs::history_injected(),
            Self::InjectedInstructions => glyphs::history_instructions(),
            Self::InjectedContinuation => glyphs::history_reminder(),
            Self::InjectedNotification => glyphs::history_notification(),
            Self::InjectedEnvironment => glyphs::history_environment(),
            Self::InjectedConversation => glyphs::history_message(),
            Self::Stub => "",
        }
    }
}

fn started_status(ctx: &BlockContext) -> MessageStatus {
    if ctx.is_running { MessageStatus::Running } else { MessageStatus::Inactive }
}

impl RenderBlock {
    pub(crate) fn message_symbol(&self, ctx: &BlockContext) -> Option<MessageSymbol> {
        self.message_symbol_with_completion(ctx, ToolCompletion::Unconfirmed)
    }

    pub(crate) fn message_symbol_with_completion(
        &self,
        ctx: &BlockContext,
        completion: ToolCompletion,
    ) -> Option<MessageSymbol> {
        let kind = MessageKind::from(self);
        let mut glyph = kind.symbol_glyph();
        let mut layout = SymbolLayout::Header;
        let status = match self {
            Self::Stub(_) => return None,
            Self::UserPrompt(user) => {
                layout = SymbolLayout::Body;
                if !ctx.appearance.scrollback.blocks.prompt.show_prefix || ctx.appearance.prompt.compact {
                    return None;
                }
                if user.is_bash { glyph = "$"; }
                if user.is_cron { glyph = crate::glyphs::history_reminder(); }
                MessageStatus::Information
            }
            Self::System(_) => {
                layout = SymbolLayout::Body;
                MessageStatus::Information
            }
            Self::ContextInfo(_) | Self::MemoryCapture(_) | Self::Injected(_) => MessageStatus::Information,
            Self::AgentMessage(_) => {
                layout = SymbolLayout::Body;
                if ctx.is_running { MessageStatus::Running } else { MessageStatus::Finished }
            }
            Self::Thinking(_) => {
                if ctx.mode != crate::scrollback::types::DisplayMode::Collapsed
                    && !ctx.appearance.scrollback.blocks.thinking.header
                {
                    layout = SymbolLayout::Body;
                }
                if ctx.is_running { MessageStatus::Running } else { MessageStatus::Finished }
            }
            Self::Btw(_) => {
                if ctx.is_running { MessageStatus::Running } else { MessageStatus::Finished }
            }
            Self::ToolCall(tool) => {
                match tool {
                    ToolCallBlock::Read(read) if read.is_skill_read() => glyph = crate::glyphs::tool_skill(),
                    ToolCallBlock::Edit(edit) if edit.prefix.starts_with("Creating ") => glyph = crate::glyphs::tool_create_file(),
                    ToolCallBlock::Execute(_) | ToolCallBlock::Read(_) | ToolCallBlock::Edit(_)
                    | ToolCallBlock::ListDir(_) | ToolCallBlock::Search(_) | ToolCallBlock::WebFetch(_)
                    | ToolCallBlock::WebSearch(_) | ToolCallBlock::IntegrationSearch(_)
                    | ToolCallBlock::UseTool(_) | ToolCallBlock::MemorySearch(_)
                    | ToolCallBlock::SentMessage(_) | ToolCallBlock::Skill(_) | ToolCallBlock::Other(_) => {}
                }
                match tool {
                    ToolCallBlock::SentMessage(sent) => match sent.presentation {
                        SentMessagePresentation::Sending => started_status(ctx),
                        SentMessagePresentation::Sent => match sent.input.as_ref().and_then(|input| input.delivery) {
                            Some(SentMessageDelivery::Queue) => MessageStatus::Queued,
                            None | Some(SentMessageDelivery::Steer | SentMessageDelivery::Interject) => MessageStatus::Done,
                        },
                        SentMessagePresentation::Rejected { .. } => MessageStatus::Failed,
                        SentMessagePresentation::Unconfirmed { .. } => MessageStatus::Unconfirmed,
                    },
                    ToolCallBlock::Execute(_) | ToolCallBlock::Read(_) | ToolCallBlock::Edit(_)
                    | ToolCallBlock::ListDir(_) | ToolCallBlock::Search(_) | ToolCallBlock::WebFetch(_)
                    | ToolCallBlock::WebSearch(_) | ToolCallBlock::IntegrationSearch(_)
                    | ToolCallBlock::UseTool(_) | ToolCallBlock::MemorySearch(_)
                    | ToolCallBlock::Skill(_) | ToolCallBlock::Other(_) => {
                        match completion {
                            ToolCompletion::Completed => MessageStatus::Done,
                            ToolCompletion::Failed => MessageStatus::Failed,
                            ToolCompletion::Cancelled => MessageStatus::Cancelled,
                            ToolCompletion::Unconfirmed => {
                                if !tool.is_success() { MessageStatus::Failed } else { started_status(ctx) }
                            }
                        }
                    }
                }
            }
            Self::BgTask(task) => match task.kind {
                BgTaskKind::Started => started_status(ctx),
                BgTaskKind::Completed { .. } => MessageStatus::Done,
                BgTaskKind::Failed { .. } => MessageStatus::Failed,
            },
            Self::Subagent(agent) => match agent.kind {
                SubagentBlockKind::Started => started_status(ctx),
                SubagentBlockKind::Completed { .. } => MessageStatus::Done,
                SubagentBlockKind::Failed { .. } => MessageStatus::Failed,
                SubagentBlockKind::Cancelled { .. } => MessageStatus::Cancelled,
            },
            Self::Workflow(workflow) => match workflow.status {
                WorkflowBlockStatus::Running => started_status(ctx),
                WorkflowBlockStatus::Done { .. } => MessageStatus::Done,
                WorkflowBlockStatus::Failed { .. } => MessageStatus::Failed,
                WorkflowBlockStatus::Cancelled { .. } => MessageStatus::Cancelled,
                WorkflowBlockStatus::Paused { .. } => MessageStatus::Paused,
            },
            Self::SessionEvent(event) => {
                layout = SymbolLayout::Body;
                let (event_glyph, status) = match event.event {
                    SessionEvent::CompactionStarted { .. } | SessionEvent::CompactStarted => {
                        (crate::glyphs::history_summary(), MessageStatus::Information)
                    }
                    SessionEvent::CompactionCompleted { .. } | SessionEvent::CompactCompleted { .. } => {
                        (crate::glyphs::history_summary(), MessageStatus::Done)
                    }
                    SessionEvent::CompactionFailed { .. } => (crate::glyphs::history_summary(), MessageStatus::Failed),
                    SessionEvent::CompactionCancelled => (crate::glyphs::history_summary(), MessageStatus::Cancelled),
                    SessionEvent::TurnCompleted { .. } | SessionEvent::GoalCompleted { .. } => (glyph, MessageStatus::Done),
                    SessionEvent::TurnCancelled { .. } => (glyph, MessageStatus::Cancelled),
                    SessionEvent::TurnFailed { .. } | SessionEvent::RetryFailed { .. }
                    | SessionEvent::RequestFailed { .. } => (glyph, MessageStatus::Failed),
                    SessionEvent::ReAuthRequired | SessionEvent::ContextTooLarge | SessionEvent::DiskFull
                    | SessionEvent::ModelUnavailable { .. } => (glyph, MessageStatus::Unavailable),
                    SessionEvent::MemoryCommandCompleted { succeeded, .. } => {
                        (crate::glyphs::tool_memory_search(), if succeeded { MessageStatus::Done } else { MessageStatus::Failed })
                    }
                    SessionEvent::Recap { .. } => {
                        layout = SymbolLayout::Header;
                        (crate::glyphs::history_summary(), if ctx.is_running { MessageStatus::Running } else { MessageStatus::Information })
                    }
                    SessionEvent::TurnBlockedByHook { .. } | SessionEvent::TurnHalted { .. } => (glyph, MessageStatus::Paused),
                    SessionEvent::MemoryCommandStarted { .. } | SessionEvent::MemorySaved { .. } => {
                        (crate::glyphs::tool_memory_search(), MessageStatus::Information)
                    }
                    SessionEvent::HookAnnotation { .. } | SessionEvent::HookOutcome { .. }
                    | SessionEvent::PlanModeEnteredByAgent { .. } | SessionEvent::PlanReviewClosed { .. } => (glyph, MessageStatus::Information),
                };
                glyph = event_glyph;
                status
            }
        };
        Some(MessageSymbol { glyph, kind, status, layout })
    }

    pub(crate) fn symbol_context(&self, ctx: &BlockContext, prefix: &str) -> BlockContext {
        let mut content = ctx.clone();
        let width = unicode_width::UnicodeWidthStr::width(prefix);
        content.width = ctx.width.saturating_sub(u16::try_from(width).unwrap_or(ctx.width));
        content.appearance.scrollback.blocks.tool.bullet = ToolBullet::None;
        if matches!(self, Self::UserPrompt(_)) {
            content.appearance.scrollback.blocks.prompt.show_prefix = false;
        }
        content
    }
}

#[cfg(test)]
#[path = "message_symbol_tests.rs"]
mod tests;
