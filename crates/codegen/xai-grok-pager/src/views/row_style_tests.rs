use std::time::Duration;

use strum::IntoEnumIterator as _;
use unicode_width::UnicodeWidthStr as _;

use super::*;
use crate::scrollback::blocks::{BtwBlock, SessionEvent, SubagentBlock, WorkflowBlock};
use crate::theme::cache::pin_theme;

/// The active (pinned) theme plus every built-in palette with distinct color slots; the terminal-native palette is excluded because its slots are all `Reset`.
fn themes() -> Vec<(&'static str, Theme)> {
    vec![
        ("current", Theme::current()),
        ("groknight", Theme::groknight()),
        ("grokday", Theme::grokday()),
        ("tokyonight", Theme::tokyonight()),
        ("rosepine_moon", Theme::rosepine_moon()),
        ("oscura_midnight", Theme::oscura_midnight()),
        ("color_dna", Theme::color_dna()),
    ]
}

fn plain() -> RowState {
    RowState::default()
}

fn label(class: OutlineClass, state: RowState, theme: &Theme) -> Style {
    outline_style(class, state, theme).label
}

#[test]
fn outline_classes_are_pairwise_distinguishable() {
    let _guard = pin_theme();
    for (name, theme) in themes() {
        let classes: Vec<_> = OutlineClass::iter().collect();
        for (i, a) in classes.iter().enumerate() {
            for b in classes.iter().skip(i + 1) {
                assert_ne!(
                    label(*a, plain(), &theme),
                    label(*b, plain(), &theme),
                    "{name}: {a:?} and {b:?} render identically"
                );
            }
        }
    }
}

#[test]
fn running_differs_from_idle_for_every_class() {
    let _guard = pin_theme();
    let running = RowState {
        running: true,
        ..plain()
    };
    for (name, theme) in themes() {
        for class in OutlineClass::iter() {
            assert_ne!(
                label(class, running, &theme),
                label(class, plain(), &theme),
                "{name}: running {class:?} looks idle"
            );
            assert_ne!(
                outline_style(class, running, &theme).meta,
                outline_style(class, plain(), &theme).meta,
                "{name}: running {class:?} meta looks idle"
            );
        }
    }
}

#[test]
fn removed_from_context_differs_and_is_struck_through() {
    let _guard = pin_theme();
    let removed = RowState {
        presence: ContextPresence::Removed,
        ..plain()
    };
    for (name, theme) in themes() {
        for class in OutlineClass::iter() {
            let kept = outline_style(class, plain(), &theme);
            let dropped = outline_style(class, removed, &theme);
            assert_ne!(dropped.label, kept.label, "{name}: {class:?} label");
            assert_ne!(dropped.meta, kept.meta, "{name}: {class:?} meta");
            assert!(
                dropped.label.add_modifier.contains(Modifier::CROSSED_OUT),
                "{name}: {class:?} label is not struck through"
            );
            assert!(
                !dropped.glyph.add_modifier.contains(Modifier::CROSSED_OUT),
                "{name}: {class:?} glyph must stay readable"
            );
            assert!(
                dropped.label.add_modifier.contains(Modifier::DIM),
                "{name}: {class:?} label is not dimmed"
            );
        }
    }
}

#[test]
fn context_presence_markers_are_distinct() {
    let _guard = pin_theme();
    let presences = [
        ContextPresence::InContext,
        ContextPresence::DisplayOnly,
        ContextPresence::Removed,
    ];
    for (name, theme) in themes() {
        let marker = |presence| {
            let state = RowState {
                presence,
                ..plain()
            };
            outline_style(OutlineClass::ToolCall, state, &theme).marker
        };
        for (i, a) in presences.iter().enumerate() {
            for b in presences.iter().skip(i + 1) {
                assert_ne!(
                    marker(*a),
                    marker(*b),
                    "{name}: {a:?} vs {b:?} marker style"
                );
                assert_ne!(
                    a.marker_glyph(),
                    b.marker_glyph(),
                    "{name}: {a:?} vs {b:?} marker glyph"
                );
            }
        }
    }
}

#[test]
fn switch_on_differs_from_switch_off() {
    let _guard = pin_theme();
    for (name, theme) in themes() {
        let on = sidebar_style(SidebarRowClass::SwitchOn, &theme);
        let off = sidebar_style(SidebarRowClass::SwitchOff, &theme);
        assert_ne!(on.glyph, off.glyph, "{name}: glyph");
        assert_ne!(on.value, off.value, "{name}: value");
        assert_ne!(
            SidebarRowClass::SwitchOn.glyph(),
            SidebarRowClass::SwitchOff.glyph()
        );
    }
}

#[test]
fn sidebar_classes_are_pairwise_distinguishable() {
    let _guard = pin_theme();
    for (name, theme) in themes() {
        let classes: Vec<_> = SidebarRowClass::iter().collect();
        for (i, a) in classes.iter().enumerate() {
            for b in classes.iter().skip(i + 1) {
                assert_ne!(
                    sidebar_style(*a, &theme),
                    sidebar_style(*b, &theme),
                    "{name}: {a:?} and {b:?} render identically"
                );
            }
        }
    }
}

#[test]
fn unavailable_row_is_dimmer_than_an_enabled_switch() {
    let _guard = pin_theme();
    for (name, theme) in themes() {
        let unavailable = sidebar_style(SidebarRowClass::Unavailable, &theme);
        assert!(
            unavailable.label.add_modifier.contains(Modifier::DIM),
            "{name}: unavailable label not dimmed"
        );
        for enabled in [SidebarRowClass::SwitchOn, SidebarRowClass::SwitchOff] {
            let style = sidebar_style(enabled, &theme);
            assert_ne!(unavailable.label, style.label, "{name}: {enabled:?} label");
            assert!(
                !style.label.add_modifier.contains(Modifier::DIM),
                "{name}: {enabled:?} label must not be dimmed"
            );
        }
    }
}

#[test]
fn display_form_maps_to_its_own_sidebar_class() {
    assert_eq!(
        SidebarRowClass::from(DisplayForm::Expanded),
        SidebarRowClass::FormExpanded
    );
    assert_eq!(
        SidebarRowClass::from(DisplayForm::Collapsed),
        SidebarRowClass::FormOneLine
    );
}

#[test]
fn branch_state_encoding_selects_distinct_one_column_glyphs() {
    let expanded = Disclosure::of_branch(Some(true)).glyph();
    let collapsed = Disclosure::of_branch(Some(false)).glyph();
    let leaf = Disclosure::of_branch(None).glyph();
    assert_ne!(expanded, collapsed);
    assert_ne!(expanded, leaf);
    assert_ne!(collapsed, leaf);
    for glyph in [expanded, collapsed, leaf] {
        assert_eq!(glyph.width(), 1, "{glyph:?} must fit one column");
    }
}

#[test]
fn sidebar_glyphs_are_distinct_one_column_and_reuse_disclosure_glyphs() {
    let shown: Vec<_> = SidebarRowClass::iter()
        .filter_map(SidebarRowClass::glyph)
        .collect();
    assert_eq!(shown.len(), SidebarRowClass::iter().count() - 1);
    for (i, a) in shown.iter().enumerate() {
        assert_eq!(a.width(), 1, "{a:?} must fit one column");
        for b in shown.iter().skip(i + 1) {
            assert_ne!(a, b);
        }
    }
    assert_eq!(SidebarRowClass::SectionTitle.glyph(), None);
    assert_eq!(
        SidebarRowClass::FormExpanded.glyph(),
        Some(Disclosure::of_branch(Some(true)).glyph())
    );
    assert_eq!(
        SidebarRowClass::FormOneLine.glyph(),
        Some(Disclosure::of_branch(Some(false)).glyph())
    );
}

#[test]
fn blocks_map_to_their_outline_class() {
    let failed_workflow = {
        let mut workflow = WorkflowBlock::started("run", "flow", "goal");
        workflow.status = WorkflowBlockStatus::Failed {
            elapsed: Duration::from_secs(2),
        };
        RenderBlock::Workflow(workflow)
    };
    let cases = [
        (RenderBlock::user_prompt("hi"), OutlineClass::UserPrompt),
        (
            RenderBlock::agent_message("hello"),
            OutlineClass::AgentMessage,
        ),
        (
            RenderBlock::Btw(BtwBlock::new("q", "a")),
            OutlineClass::AgentMessage,
        ),
        (RenderBlock::thinking("hmm"), OutlineClass::Thinking),
        (RenderBlock::system("note"), OutlineClass::SystemNote),
        (
            RenderBlock::session_event(SessionEvent::DiskFull),
            OutlineClass::SystemNote,
        ),
        (
            RenderBlock::tool_call("custom_tool", "summary", true),
            OutlineClass::ToolCall,
        ),
        (
            RenderBlock::tool_call("custom_tool", "summary", false),
            OutlineClass::Failed,
        ),
        (
            RenderBlock::bg_task("sleep 1", "t1"),
            OutlineClass::BackgroundTask,
        ),
        (
            RenderBlock::bg_task_completed("sleep 1", "t1", Duration::from_secs(1)),
            OutlineClass::BackgroundTask,
        ),
        (
            RenderBlock::bg_task_failed("sleep 1", "t1", Duration::from_secs(1), Some(1), None),
            OutlineClass::Failed,
        ),
        (
            RenderBlock::Subagent(SubagentBlock::completed("d", "sid", Duration::from_secs(1))),
            OutlineClass::Delegated,
        ),
        (
            RenderBlock::Subagent(SubagentBlock::failed(
                "d",
                "sid",
                Duration::from_secs(1),
                None,
            )),
            OutlineClass::Failed,
        ),
        (
            RenderBlock::Workflow(WorkflowBlock::started("run", "flow", "goal")),
            OutlineClass::Delegated,
        ),
        (failed_workflow, OutlineClass::Failed),
    ];
    for (block, expected) in cases {
        assert_eq!(OutlineClass::of_block(&block), expected, "{block:?}");
    }
}
