//! PTY: Enter and Ctrl+F on a message row open the exact child session; unresolved targets stay in the parent.
// Unix only: shares the `/bin/sleep`-held child of `send_subagent_message_row`.
#![cfg(unix)]
#[allow(unused_imports)]
use super::common::*;
use super::send_subagent_message_row::{
    CHILD_LABEL, PARENT_DONE, PARENT_NAVIGATION, ROW_TIMEOUT, SendMessageScenario,
};

const CTRL_F: &[u8] = b"\x06";

fn assert_child_open(scenario: &mut SendMessageScenario, chord: &str) {
    scenario
        .harness
        .wait_for_text(PARENT_NAVIGATION, ROW_TIMEOUT)
        .unwrap_or_else(|error| panic!("{chord} did not open the child session: {error}"));
    assert!(
        !scenario.harness.contains_text(PARENT_DONE),
        "{chord}: the child session must replace the parent transcript\nscreen:\n{}",
        scenario.harness.screen_contents()
    );
    scenario.verify_child_session_info();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "PTY e2e; run the owning pty_e2e_* Cargo test with --ignored (see Cargo.toml)"]
async fn send_subagent_message_row_opens_child() {
    let mut scenario = SendMessageScenario::start().await;
    let steer_row = format!("Message sent to {CHILD_LABEL}");
    scenario.focus_scrollback();
    scenario.select_row(&steer_row);

    scenario
        .harness
        .inject_keys(keys::ENTER)
        .expect("press Enter");
    assert_child_open(&mut scenario, "Enter");
    write_screen_dump_if_requested(&scenario.harness, "send_row_enter_opens_child");
    scenario.return_to_parent(PARENT_DONE);
    scenario.select_row(&steer_row);

    scenario.harness.inject_keys(CTRL_F).expect("press Ctrl+F");
    assert_child_open(&mut scenario, "Ctrl+F");
    scenario.return_to_parent(PARENT_DONE);

    // The rejected row's target never resolved through a spawn, so Enter has no child to open.
    scenario.select_row("Message rejected \u{b7} subagent");
    scenario
        .harness
        .inject_keys(keys::ENTER)
        .expect("press Enter on the rejected row");
    scenario
        .harness
        .wait_until_stable(
            "parent transcript stays after Enter on the rejected row",
            Duration::from_secs(6),
            Duration::from_secs(3),
            |h| {
                !h.contains_text(PARENT_NAVIGATION)
                    && h.contains_text("\u{203a} Message rejected")
                    && h.contains_text(PARENT_DONE)
            },
        )
        .expect("no child session for an unresolved target");
    write_screen_dump_if_requested(&scenario.harness, "send_row_rejected_enter_stays");

    scenario.finish("send_subagent_message_row_opens_child.cast");
}
