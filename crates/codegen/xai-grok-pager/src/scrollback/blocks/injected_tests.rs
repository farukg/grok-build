//! Tests for the injected-context block (collapsed-by-default kind, Read-style expansion).

use super::*;
use crate::scrollback::types::{BlockContext, DisplayMode};
use unicode_width::UnicodeWidthStr;

fn ctx(mode: DisplayMode, width: u16) -> BlockContext {
    BlockContext {
        mode,
        is_running: false,
        width,
        raw: false,
        max_lines: None,
        appearance: Default::default(),
        is_selected: false,
        cwd: None,
    }
}

fn line_text(output: &crate::scrollback::types::BlockOutput, index: usize) -> String {
    output
        .lines
        .get(index)
        .unwrap_or_else(|| panic!("expected line {index}"))
        .content
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect()
}

#[test]
fn injected_defaults_collapsed_and_folds_on_content() {
    let block = InjectedBlock::new("Task completed", "body text");
    assert_eq!(block.default_display_mode(), DisplayMode::Collapsed);
    assert!(block.is_foldable());
    let empty = InjectedBlock::new("Task completed", "");
    assert!(!empty.is_foldable(), "a label-only entry has nothing to fold");
}

#[test]
fn collapsed_header_shows_label_and_bounded_preview() {
    let block = InjectedBlock::new(
        "Notification drain",
        "first line of the drain body\nsecond line never shows",
    );
    let out = block.output(&ctx(DisplayMode::Collapsed, 80));
    assert_eq!(out.lines.len(), 1);
    let text = line_text(&out, 0);
    assert!(text.starts_with("Notification drain"), "got {text:?}");
    assert!(text.contains("first line of the drain body"), "got {text:?}");
    assert!(!text.contains("second line"), "got {text:?}");
}

#[test]
fn expanded_renders_full_body_below_the_header() {
    let block = InjectedBlock::new("Memory reminder", "line one\nline two\nline three");
    let out = block.output(&ctx(DisplayMode::Expanded, 80));
    assert!(out.lines.len() >= 4, "header, separator, and body rows");
    assert_eq!(line_text(&out, 0), "Memory reminder");
    let joined: Vec<String> = (0..out.lines.len()).map(|i| line_text(&out, i)).collect();
    let all = joined.join("\n");
    assert!(all.contains("line one") && all.contains("line three"), "got {all:?}");
}

#[test]
fn injected_labels_cover_known_reasons_and_fall_back_for_unknown() {
    assert_eq!(injected_label("task_completed"), "Task completed");
    assert_eq!(injected_label("memory_reminder"), "Memory reminder");
    assert_eq!(injected_label("context_view"), "Context view");
    assert_eq!(injected_label("future_thing"), "Injected (future_thing)");
}

#[test]
fn collapsed_preview_fits_the_available_width() {
    let block = InjectedBlock::new(
        "Notification drain",
        "a body line that is far longer than a narrow pane can hold without spilling",
    );
    let out = block.output(&ctx(DisplayMode::Collapsed, 30));
    let text = line_text(&out, 0);
    assert!(
        unicode_width::UnicodeWidthStr::width(text.as_str()) <= 30,
        "row must fit its width: {text:?}"
    );
    assert!(text.contains('\u{2026}'), "the cut must be marked: {text:?}");
}
