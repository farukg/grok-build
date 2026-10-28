use super::*;
use std::time::Duration;

use crate::app::actions::PermissionLabel;
use crate::appearance::{AppearanceConfig, ExecuteHeaderStyle};
use crate::render::Renderable;
use crate::scrollback::block::BlockContent;
use crate::scrollback::blocks::tool::{
    SentMessageInput, SentMessageTarget, WebFetchToolCallBlock, WebSearchToolCallBlock,
};
use crate::scrollback::blocks::*;
use crate::scrollback::entry::ScrollbackEntry;
use crate::scrollback::types::{
    DisplayForm, DisplayMode, derive_selection_text, line_plain_text, selectable_cols,
};
use crate::scrollback::wrappers::EntryRenderer;
use crate::theme::cache::pin_theme;
use crate::views::plan_approval_view::PlanReviewOutcome;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use unicode_width::UnicodeWidthStr;

fn context(mode: DisplayMode, width: u16) -> BlockContext {
    BlockContext {
        mode,
        width,
        is_running: false,
        raw: false,
        max_lines: None,
        appearance: AppearanceConfig::default(),
        is_selected: false,
        cwd: None,
    }
}

fn tools() -> Vec<(ToolCallBlock, &'static str)> {
    use crate::glyphs;
    vec![
        (
            ToolCallBlock::Execute(ExecuteToolCallBlock::new("echo source")),
            glyphs::tool_execute(),
        ),
        (
            ToolCallBlock::Read(ReadToolCallBlock::new("source.rs")),
            glyphs::tool_read_file(),
        ),
        (
            ToolCallBlock::Read(ReadToolCallBlock::new("/skills/deploy/SKILL.md")),
            glyphs::tool_skill(),
        ),
        (
            ToolCallBlock::Edit(EditToolCallBlock::new("source.rs", vec![])),
            glyphs::tool_edit_file(),
        ),
        (
            ToolCallBlock::Edit(EditToolCallBlock::new("new.rs", vec![]).with_prefix("Creating ")),
            glyphs::tool_create_file(),
        ),
        (
            ToolCallBlock::ListDir(ListDirToolCallBlock::new("src")),
            glyphs::tool_list_dir(),
        ),
        (
            ToolCallBlock::Search(SearchToolCallBlock::new("query")),
            glyphs::tool_search(),
        ),
        (
            ToolCallBlock::WebFetch(WebFetchToolCallBlock::new("https://example.test")),
            glyphs::tool_web_fetch(),
        ),
        (
            ToolCallBlock::WebSearch(WebSearchToolCallBlock::new("query")),
            glyphs::tool_web_search(),
        ),
        (
            ToolCallBlock::IntegrationSearch(IntegrationSearchToolCallBlock::new("query")),
            glyphs::tool_integration_search(),
        ),
        (
            ToolCallBlock::UseTool(UseToolCallBlock::new("server__action")),
            glyphs::history_integration(),
        ),
        (
            ToolCallBlock::MemorySearch(MemorySearchToolCallBlock::new("query")),
            glyphs::tool_memory_search(),
        ),
        (
            ToolCallBlock::SentMessage(SentMessageToolCallBlock::new(
                SentMessagePresentation::Sent,
                None,
            )),
            glyphs::history_message(),
        ),
        (
            ToolCallBlock::Skill(OtherToolCallBlock::new("Skill", "deploy")),
            glyphs::tool_skill(),
        ),
        (
            ToolCallBlock::Other(OtherToolCallBlock::new("unknown_tool", "source")),
            glyphs::history_tool(),
        ),
    ]
}

fn blocks() -> Vec<(RenderBlock, &'static str)> {
    use crate::glyphs;
    let mut blocks: Vec<_> = tools()
        .into_iter()
        .map(|(tool, glyph)| (RenderBlock::ToolCall(tool), glyph))
        .collect();
    blocks.extend([
        (RenderBlock::user_prompt("source"), glyphs::history_user()),
        (
            RenderBlock::agent_message("source"),
            glyphs::history_assistant(),
        ),
        (RenderBlock::thinking("source"), glyphs::history_thinking()),
        (RenderBlock::system("source"), glyphs::history_system()),
        (
            RenderBlock::SessionEvent(SessionEventBlock::new(SessionEvent::TurnCompleted {
                elapsed: None,
            })),
            glyphs::history_system(),
        ),
        (
            RenderBlock::BgTask(BgTaskBlock::started("source", "task")),
            glyphs::history_task(),
        ),
        (
            RenderBlock::Subagent(SubagentBlock::started(
                "source", "child", "explore", None, None, None, false,
            )),
            glyphs::history_subagent(),
        ),
        (
            RenderBlock::Workflow(WorkflowBlock::started("run", "workflow", "source")),
            glyphs::history_workflow(),
        ),
        (
            RenderBlock::Btw(BtwBlock::new("question", "source")),
            glyphs::history_btw(),
        ),
        (
            RenderBlock::ContextInfo(ContextInfoBlock::new(Default::default(), "model")),
            glyphs::history_summary(),
        ),
        (
            RenderBlock::memory_capture(1, 2, vec![]),
            glyphs::tool_memory_search(),
        ),
        (
            RenderBlock::Injected(InjectedBlock::new("Legacy", "source")),
            glyphs::history_injected(),
        ),
    ]);
    for (reason, glyph) in [
        ("future_reason", glyphs::history_injected()),
        ("primary", glyphs::history_instructions()),
        ("system_reminder", glyphs::history_reminder()),
        ("notification_drain", glyphs::history_notification()),
        ("memory_reminder", glyphs::tool_memory_search()),
        ("context_view", glyphs::history_summary()),
        ("session_prefix", glyphs::history_environment()),
        ("agent_message", glyphs::history_message()),
    ] {
        blocks.push((
            RenderBlock::Injected(InjectedBlock::from_reason(reason, "source")),
            glyph,
        ));
    }
    blocks
}

#[test]
fn every_message_has_one_integrated_prefix_and_copy_excludes_chrome() {
    let _theme = pin_theme();
    for (block, glyph) in blocks() {
        for mode in [
            DisplayMode::Collapsed,
            DisplayMode::Truncated,
            DisplayMode::Expanded,
        ] {
            let ctx = context(mode, 100);
            let output = block.output(&ctx);
            let first = output.lines.first().expect("message row");
            let painted = line_plain_text(&first.content);
            assert!(
                painted.starts_with(&format!("{glyph} ")),
                "{block:?}: {painted}"
            );
            assert_eq!(first.chrome_prefix_width, glyph.width() + 1);
            assert_eq!(painted.matches(glyph).count(), 1, "{block:?}: {painted}");
            assert!(
                !painted.contains(crate::glyphs::diamond_filled()),
                "{painted}"
            );
            assert!(
                !derive_selection_text(first).starts_with(glyph),
                "copy includes chrome: {painted}"
            );
        }
    }
    let stub = RenderBlock::stub("test fixture", Color::Blue);
    assert_eq!(
        line_plain_text(&stub.output(&context(DisplayMode::Expanded, 80)).lines[0].content),
        "test fixture"
    );
}

#[test]
fn off_custom_prompt_and_narrow_width_contracts() {
    let _theme = pin_theme();
    for (block, glyph) in blocks() {
        for width in [0, 1, 2, 7, 23, 100] {
            let mut ctx = context(DisplayMode::Collapsed, width);
            for bullet in [
                ToolBullet::Category,
                ToolBullet::None,
                ToolBullet::Dot,
                ToolBullet::Diamond,
            ] {
                ctx.appearance.scrollback.blocks.tool.bullet = bullet;
                let output = block.output(&ctx);
                assert_eq!(output.lines.len(), 1);
                let first = &output.lines[0];
                assert!(
                    first.content.width() <= usize::from(width),
                    "{block:?}, {width}: {:?}",
                    first.content
                );
                if width > 0 && !matches!(block, RenderBlock::UserPrompt(_)) {
                    match bullet {
                        ToolBullet::None => assert_eq!(first.chrome_prefix_width, 0),
                        ToolBullet::Dot => {
                            assert!(line_plain_text(&first.content).starts_with('·'))
                        }
                        ToolBullet::Category => {
                            assert!(line_plain_text(&first.content).starts_with(glyph))
                        }
                        ToolBullet::Diamond => assert!(
                            line_plain_text(&first.content)
                                .starts_with(crate::glyphs::diamond_filled())
                        ),
                        ToolBullet::SmallCircle
                        | ToolBullet::Circle
                        | ToolBullet::SmallTriangle
                        | ToolBullet::Triangle => unreachable!(),
                    }
                }
            }
        }
    }
    let mut ctx = context(DisplayMode::Expanded, 80);
    ctx.appearance.scrollback.blocks.prompt.show_prefix = false;
    let user = RenderBlock::user_prompt("source");
    assert_eq!(
        line_plain_text(&user.output(&ctx).lines[0].content),
        "source"
    );
    ctx.appearance.scrollback.blocks.prompt.show_prefix = true;
    ctx.appearance.prompt.compact = true;
    assert_eq!(
        line_plain_text(&user.output(&ctx).lines[0].content),
        "source"
    );
    assert!(
        RenderBlock::bash_prompt("echo source")
            .output(&context(DisplayMode::Collapsed, 80))
            .lines[0]
            .content
            .spans[0]
            .content
            .starts_with('$')
    );
}

fn paint(entry: &ScrollbackEntry, tick: u64, theme: &Theme) -> Buffer {
    let area = Rect::new(0, 0, 100, 8);
    let mut buffer = Buffer::empty(area);
    EntryRenderer::new(entry, theme)
        .with_tick(tick)
        .with_selected(true)
        .render(area, &mut buffer);
    buffer
}

#[test]
fn empty_live_sources_still_have_one_non_copyable_symbol() {
    let _theme = pin_theme();
    let mut ctx = context(DisplayMode::Expanded, 80);
    ctx.is_running = true;
    for (block, glyph) in [
        (
            RenderBlock::agent_message_streaming(),
            crate::glyphs::history_assistant(),
        ),
        (
            RenderBlock::Thinking(ThinkingBlock::streaming()),
            crate::glyphs::history_thinking(),
        ),
    ] {
        let output = block.output(&ctx);
        let first = &output.lines[0];
        assert!(line_plain_text(&first.content).starts_with(glyph));
        assert_eq!(first.chrome_prefix_width, 2);
        assert!(!derive_selection_text(first).contains(glyph));
    }
}

#[test]
fn running_glyphs_pulse_without_width_or_output_cache_changes() {
    let _theme = pin_theme();
    let theme = Theme::current();
    let mut running: Vec<_> = tools()
        .into_iter()
        .filter(|(tool, _)| !matches!(tool, ToolCallBlock::SentMessage(_)))
        .map(|(tool, glyph)| (RenderBlock::ToolCall(tool), glyph))
        .collect();
    running.extend([
        (
            RenderBlock::ToolCall(ToolCallBlock::SentMessage(SentMessageToolCallBlock::new(
                SentMessagePresentation::Sending,
                None,
            ))),
            crate::glyphs::history_message(),
        ),
        (
            RenderBlock::Btw(BtwBlock::new("question", "source")),
            crate::glyphs::history_btw(),
        ),
        (
            RenderBlock::Thinking(ThinkingBlock::new("source")),
            crate::glyphs::history_thinking(),
        ),
        (
            RenderBlock::agent_message("source"),
            crate::glyphs::history_assistant(),
        ),
        (
            RenderBlock::BgTask(BgTaskBlock::started("source", "task")),
            crate::glyphs::history_task(),
        ),
        (
            RenderBlock::Subagent(SubagentBlock::started(
                "source", "child", "explore", None, None, None, false,
            )),
            crate::glyphs::history_subagent(),
        ),
        (
            RenderBlock::Workflow(WorkflowBlock::started("run", "workflow", "source")),
            crate::glyphs::history_workflow(),
        ),
    ]);
    for (block, glyph) in running {
        let entry = ScrollbackEntry::running(block).with_display_mode(DisplayMode::Collapsed);
        let first = paint(&entry, 0, &theme);
        let cache = entry.cached_output_ref().lines.as_ptr();
        let second = paint(&entry, 10, &theme);
        assert_eq!(
            entry.cached_output_ref().lines.as_ptr(),
            cache,
            "tick must not rebuild output"
        );
        let y = u16::from(entry.block.has_vpad_for(&AppearanceConfig::default()));
        let a = first.cell((3, y)).unwrap();
        let b = second.cell((3, y)).unwrap();
        assert_eq!(a.symbol(), glyph);
        assert_eq!(b.symbol(), glyph);
        assert_ne!(a.fg, b.fg, "running {glyph} must pulse");
        assert_eq!(entry.cached_output_ref().lines[0].chrome_prefix_width, 2);
    }
}

#[test]
fn task_failure_completion_cancel_pause_and_queue_have_real_rendered_colors() {
    let _theme = pin_theme();
    let theme = Theme::current();
    let mut cancelled = WorkflowBlock::started("run", "workflow", "source");
    cancelled.status = WorkflowBlockStatus::Cancelled {
        elapsed: Duration::ZERO,
    };
    let mut paused = cancelled.clone();
    paused.status = WorkflowBlockStatus::Paused {
        elapsed: Duration::ZERO,
    };
    let queued = SentMessageToolCallBlock::new(
        SentMessagePresentation::Sent,
        Some(SentMessageInput {
            target: SentMessageTarget::Parent,
            delivery: Some(SentMessageDelivery::Queue),
            text: "source".into(),
        }),
    );
    for (block, color, word) in [
        (
            RenderBlock::BgTask(BgTaskBlock::completed("source", "task", Duration::ZERO)),
            theme.accent_success,
            "completed",
        ),
        (
            RenderBlock::BgTask(BgTaskBlock::failed(
                "source",
                "task",
                Duration::ZERO,
                Some(9),
                None,
            )),
            theme.accent_error,
            "failed",
        ),
        (
            RenderBlock::Workflow(cancelled),
            theme.gray_dim,
            "cancelled",
        ),
        (RenderBlock::Workflow(paused), theme.warning, "paused"),
        (
            RenderBlock::ToolCall(ToolCallBlock::SentMessage(queued)),
            theme.accent_success,
            "queued",
        ),
    ] {
        let entry = ScrollbackEntry::new(block).with_display_mode(DisplayMode::Collapsed);
        for tick in [0, 10] {
            let output = paint(&entry, tick, &theme);
            assert_eq!(output.cell((3, 0)).unwrap().fg, color);
            assert!(line_plain_text(&entry.cached_output_ref().lines[0].content).contains(word));
        }
    }
}

#[test]
fn terminal_subagent_and_workflow_variants_keep_static_status_colors() {
    let _theme = pin_theme();
    let theme = Theme::current();
    let mut cases = vec![
        (
            RenderBlock::Subagent(SubagentBlock::completed("source", "child", Duration::ZERO)),
            theme.accent_success,
        ),
        (
            RenderBlock::Subagent(SubagentBlock::failed(
                "source",
                "child",
                Duration::ZERO,
                Some("failure".into()),
            )),
            theme.accent_error,
        ),
    ];
    let mut cancelled =
        SubagentBlock::started("source", "child", "explore", None, None, None, false);
    cancelled.kind = SubagentBlockKind::Cancelled {
        elapsed: Duration::ZERO,
    };
    cases.push((RenderBlock::Subagent(cancelled), theme.gray_dim));
    for (status, color) in [
        (WorkflowBlockStatus::Running, theme.gray),
        (
            WorkflowBlockStatus::Done {
                elapsed: Duration::ZERO,
            },
            theme.accent_success,
        ),
        (
            WorkflowBlockStatus::Failed {
                elapsed: Duration::ZERO,
            },
            theme.accent_error,
        ),
        (
            WorkflowBlockStatus::Cancelled {
                elapsed: Duration::ZERO,
            },
            theme.gray_dim,
        ),
        (
            WorkflowBlockStatus::Paused {
                elapsed: Duration::ZERO,
            },
            theme.warning,
        ),
    ] {
        let mut block = WorkflowBlock::started("run", "workflow", "source");
        block.status = status;
        cases.push((RenderBlock::Workflow(block), color));
    }
    for (block, color) in cases {
        let entry = ScrollbackEntry::new(block).with_display_mode(DisplayMode::Collapsed);
        assert_eq!(paint(&entry, 0, &theme).cell((3, 0)).unwrap().fg, color);
        assert_eq!(paint(&entry, 10, &theme).cell((3, 0)).unwrap().fg, color);
    }
}

#[test]
fn live_thought_keeps_source_progression_and_finishes_neutrally() {
    let _theme = pin_theme();
    let mut thought = ThinkingBlock::streaming();
    let mut ctx = context(DisplayMode::Expanded, 80);
    ctx.is_running = true;
    thought.push_chunk("first ");
    thought.push_chunk("**second**\n\nthird");
    assert_eq!(thought.text(), "first **second**\n\nthird");
    let running = RenderBlock::Thinking(thought.clone()).output(&ctx);
    assert!(line_plain_text(&running.lines[0].content).contains("Thinking"));
    assert!(
        running
            .lines
            .iter()
            .any(|line| derive_selection_text(line).contains("second"))
    );
    thought.finish();
    thought.set_elapsed_time_ms(Some(1200));
    ctx.is_running = false;
    ctx.mode = DisplayMode::Collapsed;
    let finished = RenderBlock::Thinking(thought);
    let output = finished.output(&ctx);
    assert!(line_plain_text(&output.lines[0].content).contains("Thought for"));
    assert!(!line_plain_text(&output.lines[0].content).contains("Thinking"));
    assert_eq!(finished.bullet(&ctx), None);
}

#[test]
fn typed_injections_are_static_grouped_and_start_at_the_existing_minimum() {
    let _theme = pin_theme();
    let mut ctx = context(DisplayMode::Collapsed, 80);
    ctx.is_running = true;
    for reason in [
        "human",
        "primary",
        "compaction_meta",
        "system_reminder",
        "length_continue",
        "project_instructions",
        "auto_continue",
        "auto_recovery",
        "interjection",
        "agent_message",
        "parent_human_message",
        "task_completed",
        "subagent_completed",
        "notification_drain",
        "goal_summary",
        "goal_classifier_nudge",
        "scheduler_fired",
        "stop_hook_feedback",
        "working_directory_switch",
        "session_prefix",
        "direct_bash",
        "goal_setup",
        "skill_catalog",
        "mcp_catalog",
        "unknown",
        "memory_reminder",
        "context_view",
        "future_reason",
    ] {
        let block = RenderBlock::Injected(InjectedBlock::from_reason(reason, "body\nsecond"));
        assert_eq!(
            block.bullet(&ctx),
            None,
            "static notice is not a task: {reason}"
        );
        assert_eq!(block.default_display_mode(), DisplayMode::Collapsed);
        assert!(block.is_groupable());
        assert_eq!(
            MessageKind::from(&block).starting_form(),
            DisplayForm::Collapsed
        );
        let output = block.output(&ctx);
        assert_eq!(output.lines.len(), 1);
        assert!(!line_plain_text(&output.lines[0].content).contains("second"));
        let entry = ScrollbackEntry::running(block).with_display_mode(DisplayMode::Collapsed);
        let theme = Theme::current();
        assert_eq!(
            paint(&entry, 0, &theme).cell((3, 0)).unwrap().fg,
            paint(&entry, 10, &theme).cell((3, 0)).unwrap().fg
        );
    }
}

#[test]
fn all_session_event_variants_keep_one_prefix_without_start_marker_animation() {
    let _theme = pin_theme();
    let events = vec![
        SessionEvent::TurnCompleted { elapsed: None },
        SessionEvent::TurnCancelled {
            elapsed: None,
            cause: CancelledBy::User,
        },
        SessionEvent::TurnBlockedByHook { elapsed: None },
        SessionEvent::TurnHalted {
            elapsed: Duration::ZERO,
        },
        SessionEvent::TurnFailed {
            error: "failure".into(),
            elapsed: None,
        },
        SessionEvent::CompactionStarted {
            percentage: 85,
            reason: "threshold".into(),
        },
        SessionEvent::CompactionCompleted {
            tokens_before: Some(20),
            tokens_after: 10,
            elapsed_ms: None,
        },
        SessionEvent::CompactionFailed {
            error: "failure".into(),
        },
        SessionEvent::CompactionCancelled,
        SessionEvent::RetryFailed {
            error: "failure".into(),
            error_type: None,
        },
        SessionEvent::RequestFailed {
            status: Some(500),
            headline: "failure".into(),
            detail: "detail".into(),
        },
        SessionEvent::ReAuthRequired,
        SessionEvent::ContextTooLarge,
        SessionEvent::DiskFull,
        SessionEvent::CompactStarted,
        SessionEvent::CompactCompleted {
            elapsed: Duration::ZERO,
        },
        SessionEvent::MemoryCommandStarted {
            command: MemoryCommandKind::Flush,
        },
        SessionEvent::MemoryCommandCompleted {
            summary: "result".into(),
            succeeded: false,
            elapsed: Duration::ZERO,
        },
        SessionEvent::HookAnnotation {
            message: "note".into(),
        },
        SessionEvent::HookOutcome {
            message: "outcome".into(),
        },
        SessionEvent::ModelUnavailable {
            previous_model_id: "old".into(),
            new_model_id: "new".into(),
            reason: "removed".into(),
        },
        SessionEvent::MemorySaved {
            path: "memory.md".into(),
            trigger: "flush".into(),
        },
        SessionEvent::GoalCompleted {
            elapsed: Duration::ZERO,
        },
        SessionEvent::Recap {
            summary: "recap".into(),
            auto: false,
        },
        SessionEvent::PlanModeEnteredByAgent {
            permission: PermissionLabel::Ask,
        },
        SessionEvent::PlanReviewClosed {
            outcome: PlanReviewOutcome::Approved,
            permission: PermissionLabel::Ask,
        },
    ];
    for event in events {
        let block = RenderBlock::SessionEvent(SessionEventBlock::new(event));
        let ctx = context(DisplayMode::Collapsed, 100);
        let first = &block.output(&ctx).lines[0];
        assert_eq!(first.chrome_prefix_width, 2);
        assert!(!line_plain_text(&first.content).starts_with(crate::glyphs::diamond_filled()));
        assert!(!block.bullet(&ctx).is_some_and(|style| style.animated));
    }
}

#[test]
fn monochrome_lifecycle_text_is_chrome_and_not_copied_source() {
    let _theme = pin_theme();
    color_support::set_level_for_test(color_support::ColorLevel::None);
    let ctx = context(DisplayMode::Collapsed, 80);
    let block = RenderBlock::BgTask(BgTaskBlock::failed(
        "source",
        "task",
        Duration::ZERO,
        Some(1),
        None,
    ));
    let output = block.output(&ctx);
    let first = &output.lines[0];
    assert!(line_plain_text(&first.content).contains("[failed]"));
    assert!(!derive_selection_text(first).contains("[failed]"));
    let reminder = RenderBlock::Injected(InjectedBlock::from_reason("system_reminder", "source"));
    assert!(!line_plain_text(&reminder.output(&ctx).lines[0].content).contains('['));
    color_support::set_level_for_test(color_support::ColorLevel::TrueColor);
}

#[test]
fn every_tool_failure_is_static_red_even_on_a_running_entry() {
    let _theme = pin_theme();
    let theme = Theme::current();
    for (mut tool, glyph) in tools() {
        let error = Some("failure".to_owned());
        match &mut tool {
            ToolCallBlock::Execute(block) => block.set_error(error),
            ToolCallBlock::Read(block) => block.set_error(error),
            ToolCallBlock::Edit(block) => block.set_error(error),
            ToolCallBlock::ListDir(block) => block.set_error(error),
            ToolCallBlock::Search(block) => block.set_error(error),
            ToolCallBlock::WebFetch(block) => block.set_error(error),
            ToolCallBlock::WebSearch(block) => block.set_error(error),
            ToolCallBlock::IntegrationSearch(block) => block.set_error(error),
            ToolCallBlock::UseTool(block) => block.set_error(error),
            ToolCallBlock::MemorySearch(block) => block.set_error(error),
            ToolCallBlock::SentMessage(block) => {
                block.presentation = SentMessagePresentation::Rejected {
                    reason: "failure".into(),
                }
            }
            ToolCallBlock::Skill(block) | ToolCallBlock::Other(block) => block.set_error(error),
        }
        let entry = ScrollbackEntry::running(RenderBlock::ToolCall(tool))
            .with_display_mode(DisplayMode::Collapsed);
        for tick in [0, 10] {
            let buffer = paint(&entry, tick, &theme);
            let cell = buffer.cell((3, 0)).unwrap();
            assert_eq!(cell.symbol(), glyph);
            assert_eq!(cell.fg, theme.accent_error);
        }
    }
}

#[test]
fn label_commands_have_one_scrollback_symbol_and_retain_viewer_identity() {
    let _theme = pin_theme();
    let block = RenderBlock::execute("echo source");
    for header in [ExecuteHeaderStyle::Shell, ExecuteHeaderStyle::Label] {
        let mut ctx = context(DisplayMode::Expanded, 80);
        ctx.appearance.scrollback.blocks.execute.header_style = header;
        let output = block.output(&ctx);
        assert_eq!(
            line_plain_text(&output.lines[0].content)
                .matches(crate::glyphs::tool_execute())
                .count(),
            1
        );
        let preamble = block.preamble(&ctx).unwrap();
        let title = preamble.lines[0].to_string();
        assert!(title.contains("echo source"));
        if header == ExecuteHeaderStyle::Label {
            assert!(title.starts_with(crate::glyphs::tool_execute()));
        }
    }
}

#[test]
fn markdown_links_follow_the_symbol_gutter_on_first_and_continuation_rows() {
    let _theme = pin_theme();
    let block = RenderBlock::agent_message("[abcde](https://example.test)");
    let ctx = context(DisplayMode::Expanded, 5);
    let output = block.output(&ctx);
    let mut overlay = crate::render::osc8::LinkOverlay::new();
    block.with_hyperlinks(|links| {
        crate::scrollback::render::map_hyperlinks_to_overlay(
            links,
            &output,
            0,
            0,
            20,
            0,
            0,
            &[],
            None,
            &mut overlay,
        )
    });
    let positions: Vec<_> = overlay
        .links()
        .iter()
        .map(|link| (link.screen_row, link.col_start, link.col_end))
        .collect();
    assert_eq!(positions, [(0, 2, 5), (1, 2, 4)]);
    assert_eq!(
        block.copy_visible_text_in_state(&ctx).as_deref(),
        Some("abcde")
    );
}

#[test]
fn body_gutter_keeps_table_edges_aligned_and_plain_wrapped_paths_intact() {
    let _theme = pin_theme();
    let table = RenderBlock::agent_message("| A | B |\n| --- | --- |\n| one | two |\n");
    let output = table.output(&context(DisplayMode::Expanded, 80));
    let grid: Vec<_> = output
        .lines
        .iter()
        .filter(|line| {
            let text = derive_selection_text(line);
            text.starts_with(['┌', '│', '├', '└'])
        })
        .collect();
    assert!(grid.len() >= 3);
    for line in grid {
        let painted = line_plain_text(&line.content);
        let border = painted.find(['┌', '│', '├', '└']).unwrap();
        assert_eq!(
            painted[..border].width(),
            2,
            "table edge must stay in one column: {painted}"
        );
    }

    let path = format!("/tmp/{}.txt", "source".repeat(12));
    let block = RenderBlock::agent_message(&path);
    let ctx = context(DisplayMode::Expanded, 18);
    let output = block.output(&ctx);
    let mut overlay = crate::render::osc8::LinkOverlay::new();
    crate::render::osc8::scan_lines_for_url_overlays(
        output.lines.iter().enumerate().map(|(row, line)| {
            (
                u16::try_from(row).unwrap(),
                &line.content,
                line.joiner.as_deref(),
                line.chrome_prefix_width,
            )
        }),
        0,
        &[],
        &mut overlay,
    );
    assert!(overlay.links().len() > 1);
    let expected =
        crate::render::osc8::LinkTarget::File(std::sync::Arc::from(std::path::Path::new(&path)));
    for link in overlay.links() {
        assert_eq!(link.target, expected);
        assert_eq!(link.col_start, 2);
    }
    assert_eq!(
        block.copy_visible_text_in_state(&ctx).as_deref(),
        Some(path.as_str())
    );
}

#[test]
fn unicode_path_link_selection_and_styled_markdown_copy_exclude_chrome() {
    let _theme = pin_theme();
    let ctx = context(DisplayMode::Collapsed, 80);
    let block = RenderBlock::read("/tmp/源.rs", None);
    let output = block.output(&ctx);
    let first = &output.lines[0];
    assert_eq!(derive_selection_text(first), "/tmp/源.rs");
    assert_eq!(
        selectable_cols(&first.content, &first.selectable),
        Some(2..12)
    );
    assert_eq!(
        first.link_target,
        Some(crate::render::osc8::LinkTarget::File(std::sync::Arc::from(
            std::path::Path::new("/tmp/源.rs")
        ),))
    );
    let assistant = RenderBlock::agent_message("**bold** and [link](https://example.test)");
    let expanded = context(DisplayMode::Expanded, 80);
    assert_eq!(
        assistant.copy_visible_text_in_state(&expanded).as_deref(),
        Some("bold and link")
    );
    let edit = RenderBlock::edit("   源.rs   ", None);
    assert_eq!(
        edit.copy_visible_text_in_state(&context(DisplayMode::Expanded, 8))
            .as_deref(),
        Some("   源.rs   ")
    );
}

#[test]
fn acp_completed_tools_are_successful_but_orphan_turn_end_is_neutral() {
    use crate::acp::{meta::NotificationMeta, tracker::AcpUpdateTracker};
    use crate::scrollback::ScrollbackState;
    use agent_client_protocol as acp;

    let _theme = pin_theme();
    let theme = Theme::current();
    for is_replay in [false, true] {
        for eager_completion in [false, true] {
            let mut tracker = AcpUpdateTracker::new();
            let mut scrollback = ScrollbackState::new();
            let meta = NotificationMeta {
                is_replay,
                ..Default::default()
            };
            for (id, status) in [
                ("orphan", acp::ToolCallStatus::Pending),
                (
                    "completed",
                    if eager_completion {
                        acp::ToolCallStatus::Completed
                    } else {
                        acp::ToolCallStatus::Pending
                    },
                ),
            ] {
                assert!(
                    tracker.handle_update(
                        acp::SessionUpdate::ToolCall(
                            acp::ToolCall::new(id, "/tmp/source.rs")
                                .kind(acp::ToolKind::Read)
                                .status(status),
                        ),
                        &meta,
                        &mut scrollback
                    )
                );
            }
            if !eager_completion {
                assert!(
                    tracker.handle_update(
                        acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                            "completed",
                            acp::ToolCallUpdateFields::new()
                                .status(Some(acp::ToolCallStatus::Completed))
                        ),),
                        &meta,
                        &mut scrollback
                    )
                );
            }
            tracker.finish_turn(&mut scrollback);
            assert_eq!(scrollback.len(), 2);
            for (index, color) in [(0, theme.gray), (1, theme.accent_success)] {
                let entry = scrollback.get(index).unwrap();
                assert!(!entry.is_running);
                let mut entry = entry.clone();
                entry.set_display_mode(DisplayMode::Collapsed);
                for tick in [0, 10] {
                    assert_eq!(paint(&entry, tick, &theme).cell((3, 0)).unwrap().fg, color);
                }
            }
        }
    }
}

#[test]
fn acp_failed_terminal_without_error_text_keeps_failure_color() {
    use crate::acp::{meta::NotificationMeta, tracker::AcpUpdateTracker};
    use crate::scrollback::ScrollbackState;
    use agent_client_protocol as acp;

    let _theme = pin_theme();
    let theme = Theme::current();
    let mut tracker = AcpUpdateTracker::new();
    let mut scrollback = ScrollbackState::new();
    let meta = NotificationMeta::default();
    tracker.handle_update(
        acp::SessionUpdate::ToolCall(
            acp::ToolCall::new("failed", "opaque_tool").kind(acp::ToolKind::Other),
        ),
        &meta,
        &mut scrollback,
    );
    tracker.handle_update(
        acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
            "failed",
            acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::Failed)),
        )),
        &meta,
        &mut scrollback,
    );
    tracker.finish_turn(&mut scrollback);
    let mut entry = scrollback.get(0).unwrap().clone();
    entry.set_display_mode(DisplayMode::Collapsed);
    assert_eq!(
        paint(&entry, 0, &theme).cell((3, 0)).unwrap().fg,
        theme.accent_error
    );
    assert_eq!(
        paint(&entry, 10, &theme).cell((3, 0)).unwrap().fg,
        theme.accent_error
    );
}

#[test]
fn confirmed_tool_outcomes_override_running_and_invalidate_cached_paint() {
    use crate::scrollback::{ScrollbackState, types::ToolCompletion};

    let _theme = pin_theme();
    let theme = Theme::current();
    for (completion, color) in [
        (ToolCompletion::Completed, theme.accent_success),
        (ToolCompletion::Failed, theme.accent_error),
        (ToolCompletion::Cancelled, theme.gray_dim),
    ] {
        let mut scrollback = ScrollbackState::new();
        let id = scrollback.push_block(RenderBlock::read("/tmp/source.rs", None));
        scrollback.set_entry_running(id, true);
        let _ = paint(scrollback.get_by_id(id).unwrap(), 0, &theme);
        assert!(scrollback.set_tool_completion(id, completion));
        let entry = scrollback.get_by_id(id).unwrap();
        assert!(
            entry.is_running,
            "a reported outcome must dominate stale running state"
        );
        let mut ctx = entry.context(80, &AppearanceConfig::default(), None);
        ctx.mode = DisplayMode::Collapsed;
        let output = entry.output_uncached(&ctx);
        assert_eq!(output.lines[0].content.spans[0].style.fg, Some(color));
        assert_eq!(
            entry.copy_visible_text_in_state(&ctx).as_deref(),
            Some("/tmp/source.rs")
        );
        for tick in [0, 10] {
            assert_eq!(paint(entry, tick, &theme).cell((3, 0)).unwrap().fg, color);
        }
        scrollback.finish_running(id);
        let entry = scrollback.get_by_id(id).unwrap();
        assert_eq!(paint(entry, 20, &theme).cell((3, 0)).unwrap().fg, color);
    }
}
