use super::*;
use super::tool::{WebFetchToolCallBlock, WebSearchToolCallBlock};
use super::tool::memory_search::MemoryResult;
use crate::scrollback::block::{BlockContent, RenderBlock};
use crate::scrollback::types::{BlockContext, BlockLine, DisplayMode, Selectable, derive_selection_text};
use unicode_width::UnicodeWidthStr;

fn context(mode: DisplayMode, width: u16) -> BlockContext {
    BlockContext {
        mode, width, is_running: false, raw: false, max_lines: None,
        appearance: Default::default(), is_selected: false, cwd: None,
    }
}

fn selected_text<'a>(lines: impl Iterator<Item = &'a BlockLine>) -> String {
    let mut text = String::new();
    for (index, line) in lines.enumerate() {
        if index > 0 {
            text.push_str(line.joiner.as_deref().unwrap_or("\n"));
        }
        text.push_str(&derive_selection_text(line));
    }
    text
}

#[test]
fn tool_body_selection_preserves_source_across_soft_wraps() {
    let source = format!("  {}\n\n    second line", "网址example_".repeat(20));
    let mut search = WebSearchToolCallBlock::new("query");
    search.content = Some(source.clone());
    let mut dispatch = UseToolCallBlock::new("server__action");
    dispatch.output = Some(source.clone());
    let mut memory = MemorySearchToolCallBlock::new("query");
    memory.results.push(MemoryResult {
        score: 1.0, source: "memory".into(), path: "note".into(),
        start_line: 1, end_line: 3, snippet: source.clone(),
    });
    let blocks = [
        ToolCallBlock::WebFetch(WebFetchToolCallBlock::new("https://example.test").with_output(source.clone())),
        ToolCallBlock::WebSearch(search),
        ToolCallBlock::UseTool(dispatch),
        ToolCallBlock::MemorySearch(memory),
    ];
    for width in [18, 31, 80] {
        let ctx = context(DisplayMode::Expanded, width);
        for block in &blocks {
            let output = RenderBlock::ToolCall(block.clone()).output(&ctx);
            let body: Vec<_> = output.lines.iter().filter(|line| {
                line.background_is_panel && matches!(line.selectable, Selectable::Spans(_))
            }).collect();
            assert!(body.len() > 3);
            assert_eq!(selected_text(body.into_iter()), source);
            for line in &output.lines {
                if line.background_is_panel {
                    assert!(line.content.width() <= ctx.content_width());
                }
            }
        }
    }
}

#[test]
fn expanded_lifecycle_text_does_not_depend_on_physical_newlines() {
    let source = format!("BEGIN {} END", "objective界 ".repeat(30));
    let mut workflow = WorkflowBlock::started("run", "name", source.clone());
    workflow.phases.push(WorkflowBlockPhase { title: source.clone(), state: "active".into() });
    let blocks = [
        RenderBlock::Workflow(workflow),
        RenderBlock::BgTask(BgTaskBlock::started(source.clone(), "task").with_description(Some(source.clone()))),
        RenderBlock::Subagent(SubagentBlock::failed(source.clone(), "child", std::time::Duration::ZERO, Some(source.clone()))),
    ];
    let ctx = context(DisplayMode::Expanded, 24);
    for block in blocks {
        assert!(block.is_foldable());
        let output = block.output(&ctx);
        let text = selected_text(output.lines.iter());
        assert!(text.contains(&source), "{text}");
        assert!(output.lines.len() > 3);
        for line in output.lines.iter().skip(1) {
            assert!(line.content.width() <= ctx.content_width());
        }
    }
}

#[test]
fn compact_subagent_budgets_unicode_activity_model_and_failure() {
    let mut running = SubagentBlock::started("description", "child", "explore", None, None,
        Some("模型".repeat(60)), false);
    running.activity_label = Some("进度".repeat(60));
    let failed = SubagentBlock::failed("description", "child", std::time::Duration::ZERO,
        Some("错误".repeat(60)));
    for width in [1, 4, 12, 30, 80] {
        let ctx = context(DisplayMode::Collapsed, width);
        for block in [&running, &failed] {
            let output = block.output(&ctx);
            assert_eq!(output.lines.len(), 1);
            assert!(output.lines[0].content.width() <= ctx.content_width());
        }
    }
}

#[test]
fn injected_preview_uses_all_available_columns() {
    for (width, source) in [(30, "a".repeat(20)), (160, "a".repeat(120))] {
        let block = InjectedBlock::new("Label!", source.clone());
        let output = block.output(&context(DisplayMode::Collapsed, width));
        assert_eq!(derive_selection_text(&output.lines[0]), format!("Label! — {source}"));
    }
    let output = InjectedBlock::new("标签".repeat(30), "body")
        .output(&context(DisplayMode::Collapsed, 12));
    assert!(derive_selection_text(&output.lines[0]).width() <= 12);
}

#[test]
fn tool_headers_use_kind_glyphs_without_losing_identity() {
    let mut web = WebSearchToolCallBlock::new("query");
    web.label = Some("X Search ".into());
    web.is_x_search = true;
    let blocks = [
        (ToolCallBlock::Read(ReadToolCallBlock::new("/skills/deploy/SKILL.md")), crate::glyphs::tool_skill(), "deploy"),
        (ToolCallBlock::Skill(OtherToolCallBlock::new("Skill", "deploy")), crate::glyphs::tool_skill(), "deploy"),
        (ToolCallBlock::WebSearch(web), crate::glyphs::tool_web_search(), "X query"),
        (ToolCallBlock::IntegrationSearch(IntegrationSearchToolCallBlock::new("query")), crate::glyphs::tool_integration_search(), "query"),
        (ToolCallBlock::Edit(EditToolCallBlock::new("file.rs", vec![])), crate::glyphs::tool_edit_file(), "file.rs"),
        (ToolCallBlock::Edit(EditToolCallBlock::new("file.rs", vec![]).with_prefix("Creating ")), crate::glyphs::tool_create_file(), "file.rs"),
    ];
    for (block, glyph, identity) in blocks {
        let output = block.output(&context(DisplayMode::Collapsed, 80));
        let text = derive_selection_text(&output.lines[0]);
        let painted: String = output.lines[0].content.spans.iter().map(|span| span.content.as_ref()).collect();
        assert!(painted.starts_with(glyph), "{painted}");
        assert!(painted.contains(identity), "{painted}");
        assert!(!text.is_empty());
    }
}

#[test]
fn expanded_tool_errors_wrap_without_changing_copy_source() {
    let error = format!("BEGIN {} END", "网络failure_".repeat(30));
    let mut dispatch = UseToolCallBlock::new("server__action");
    dispatch.error = Some(error.clone());
    let blocks = [
        ToolCallBlock::WebFetch(WebFetchToolCallBlock::new("url").with_error(error.clone())),
        ToolCallBlock::WebSearch(WebSearchToolCallBlock::new("query").with_error(error.clone())),
        ToolCallBlock::UseTool(dispatch),
    ];
    let ctx = context(DisplayMode::Expanded, 24);
    for block in blocks {
        let output = RenderBlock::ToolCall(block).output(&ctx);
        let text = selected_text(output.lines.iter());
        assert!(text.contains(&error), "{text}");
        assert!(output.lines.len() > 3);
        assert!(output.lines.iter().skip(1).all(|line| line.content.width() <= ctx.content_width()));
    }
}

#[test]
fn truncated_shared_boxes_keep_three_rows_without_mutating_output() {
    let source = "one\ntwo\nthree\nfour\nfive";
    let fetch = WebFetchToolCallBlock::new("url").with_output(source);
    let mut search = WebSearchToolCallBlock::new("query");
    search.content = Some(source.into());
    let mut dispatch = UseToolCallBlock::new("server__action");
    dispatch.output = Some(source.into());
    for block in [ToolCallBlock::WebFetch(fetch.clone()), ToolCallBlock::WebSearch(search.clone()),
        ToolCallBlock::UseTool(dispatch.clone())] {
        let output = RenderBlock::ToolCall(block).output(&context(DisplayMode::Truncated, 80));
        let body: Vec<_> = output.lines.iter().filter(|line| {
            line.background_is_panel && matches!(&line.selectable, Selectable::Spans(_))
        }).collect();
        assert_eq!(body.len(), 3);
        assert_eq!(selected_text(body.into_iter()), "one\ntwo\nthree");
        assert!(output.lines.iter().any(|line| line.content.spans.iter().any(|span| span.content.contains('…'))));
    }
    assert_eq!(fetch.copy_text(), source);
    assert_eq!(search.copy_text(), source);
    assert_eq!(dispatch.output.as_deref(), Some(source));
}

#[test]
fn raw_unicode_bodies_preserve_indentation_tabs_and_final_newlines() {
    let source = format!("  \t|{}|  \r\n│ {}\t \n\n    \n", "界é_e\u{301}".repeat(400), "quoted ".repeat(20));
    let fetch = WebFetchToolCallBlock::new("url").with_output(source.clone());
    let mut search = WebSearchToolCallBlock::new("query");
    search.content = Some(source.clone());
    let mut dispatch = UseToolCallBlock::new("server__action");
    dispatch.output = Some(source.clone());
    let blocks = [
        ToolCallBlock::WebFetch(fetch.clone()),
        ToolCallBlock::WebSearch(search.clone()),
        ToolCallBlock::UseTool(dispatch.clone()),
    ];
    for width in [12, 24, 80] {
        let ctx = context(DisplayMode::Expanded, width);
        for block in &blocks {
            let output = RenderBlock::ToolCall(block.clone()).output(&ctx);
            let body: Vec<_> = output.lines.iter().filter(|line| {
                line.background_is_panel && matches!(&line.selectable, Selectable::Spans(_))
            }).collect();
            assert!(body.len() > 3);
            assert!(body.iter().all(|line| line.content.width() <= ctx.content_width()));
            assert_eq!(selected_text(body.into_iter()), source);
        }
    }
    assert_eq!(fetch.copy_text(), source);
    assert_eq!(search.copy_text(), source);
    assert_eq!(dispatch.output.as_deref(), Some(source.as_str()));
}

#[test]
fn truncated_raw_table_body_keeps_the_row_cap() {
    let source = format!("|{}|\nlast\n", "界cell".repeat(40));
    let block = RenderBlock::ToolCall(ToolCallBlock::WebFetch(
        WebFetchToolCallBlock::new("url").with_output(source.clone()),
    ));
    let ctx = context(DisplayMode::Truncated, 24);
    let output = block.output(&ctx);
    let body: Vec<_> = output.lines.iter().filter(|line| {
        line.background_is_panel && matches!(&line.selectable, Selectable::Spans(_))
    }).collect();
    assert_eq!(body.len(), 3);
    assert!(body.iter().all(|line| line.content.width() <= ctx.content_width()));
    assert!(source.starts_with(&selected_text(body.into_iter())));
    let ellipses: Vec<_> = output.lines.iter().filter(|line| {
        line.content.spans.iter().any(|span| span.content.contains('…'))
    }).collect();
    assert_eq!(ellipses.len(), 1);
    assert!(matches!(ellipses[0].selectable, Selectable::None));
}

#[test]
fn raw_table_and_quote_markers_are_content_not_layout() {
    for source in [format!("|{}|", "cell".repeat(40)), format!("│ {}", "quoted ".repeat(40))] {
        let block = RenderBlock::ToolCall(ToolCallBlock::WebFetch(
            WebFetchToolCallBlock::new("url").with_output(source.clone()),
        ));
        let output = block.output(&context(DisplayMode::Expanded, 24));
        let body = output.lines.iter().filter(|line| {
            line.background_is_panel && matches!(&line.selectable, Selectable::Spans(_))
        });
        assert_eq!(selected_text(body), source);
    }
}
