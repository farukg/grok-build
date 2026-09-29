//! F7: the sidebar shows the shell's policy, asks for changes and never shows an unanswered one.
use super::{AgentView, AppRenderParams, BannerSlotParams, test_fixtures};
use crate::actions::ActionRegistry;
use crate::app::actions::Action;
use crate::app::app_view::InputOutcome;
use crate::scrollback::render::ScratchBuffer;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use xai_grok_shell::extensions::context_policy::ContextPolicyReport;
use xai_grok_shell::sampling::{CategoryTokens, ContextCategory, ContextPolicy, ContextSwitch};

fn draw(agent: &mut AgentView) -> Buffer {
    let area = Rect::new(0, 0, 160, 40);
    let mut buf = Buffer::empty(area);
    agent.draw(
        area,
        &mut buf,
        &ActionRegistry::defaults(),
        &mut ScratchBuffer::new(),
        None,
        false,
        BannerSlotParams::none(),
        false,
        &mut Vec::new(),
        AppRenderParams::default(),
    );
    buf
}

fn screen(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .filter_map(|x| buf.cell((x, y)).map(|cell| cell.symbol().to_string()))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn press(agent: &mut AgentView, code: KeyCode) -> InputOutcome {
    agent.handle_input(
        &Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
        &ActionRegistry::defaults(),
    )
}

fn sidebar_row(buf: &Buffer, label: &str) -> String {
    screen(buf)
        .lines()
        .find(|line| line.contains(label))
        .unwrap_or_default()
        .to_string()
}

#[test]
fn f7_shows_the_shells_policy_and_waits_for_its_answer_to_a_change() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = test_fixtures::make_agent();

    assert!(matches!(
        press(&mut agent, KeyCode::F(7)),
        InputOutcome::Action(Action::FetchContextPolicy)
    ));
    assert!(screen(&draw(&mut agent)).contains("Loading"));

    let mut policy = ContextPolicy::default();
    policy.set(ContextCategory::Reasoning, ContextSwitch::Excluded);
    agent.context_policy_answered(Ok(ContextPolicyReport {
        policy,
        usage: vec![CategoryTokens { category: ContextCategory::Reasoning, tokens: 12_345 }],
    }));
    let buf = draw(&mut agent);
    assert!(sidebar_row(&buf, "Reasoning").contains("OFF"));
    assert!(sidebar_row(&buf, "Reasoning").contains("~12K"));
    assert!(sidebar_row(&buf, "Your messages").contains("ON"));

    let InputOutcome::Action(Action::SetContextPolicy(requested)) =
        press(&mut agent, KeyCode::Char(' '))
    else {
        panic!("space asks the shell for the change");
    };
    let first = xai_grok_shell::sampling::switchable_categories()[0];
    assert!(!requested.includes(first), "the cursor starts on the first row, which was ON");
    assert!(!requested.includes(ContextCategory::Reasoning), "the rest of the policy is unchanged");
    assert!(
        screen(&draw(&mut agent)).contains("Applying"),
        "the row keeps the shell's last answer until it answers again"
    );

    agent.context_policy_answered(Ok(ContextPolicyReport { policy: requested, usage: Vec::new() }));
    assert!(!screen(&draw(&mut agent)).contains("Applying"));
    assert!(matches!(press(&mut agent, KeyCode::Esc), InputOutcome::Changed));
    assert!(!screen(&draw(&mut agent)).contains("Context sent to the model"));
}
