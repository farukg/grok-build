//! PTY: compact a finished child, cancel its held summary, then query and reopen it.
#![cfg(unix)]
#[allow(unused_imports)]
use super::common::*;
use super::send_subagent_message_row::{
    CHILD_LABEL, PARENT_NAVIGATION, ROW_TIMEOUT, SendMessageScenario, WAKE_DONE,
};

const AUX_HOLD_TIMEOUT: Duration = Duration::from_secs(30);
const COMPACT_TERMINAL_TIMEOUT: Duration = Duration::from_secs(45);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "PTY e2e; run the owning pty_e2e_shell_tools Cargo test with --ignored"]
async fn finished_child_manual_compact_cancel_returns_to_reopenable_child() {
    let mut scenario = SendMessageScenario::start().await;
    std::fs::write(&scenario.hold_flag, b"done").expect("release child command");
    scenario.wait_for_row(WAKE_DONE);
    scenario
        .harness
        .wait_for_turn_idle(ROW_TIMEOUT)
        .expect("parent is idle after the child completion wake");

    scenario.focus_scrollback();
    let child_row = format!("Message sent to {CHILD_LABEL}");
    scenario.select_row(&child_row);
    scenario
        .harness
        .inject_keys(keys::ENTER)
        .expect("open finished child");
    scenario
        .harness
        .wait_for_text(PARENT_NAVIGATION, ROW_TIMEOUT)
        .expect("finished child header exposes parent navigation");
    scenario.wait_for_row("CHILD_DONE");
    scenario
        .harness
        .wait_for_turn_idle(ROW_TIMEOUT)
        .expect("child turn is idle before administrative compaction");

    scenario
        .content
        .server()
        .set_auxiliary_hold_matching("xai-compact-");
    scenario
        .harness
        .inject_keys(b"/compact\r")
        .expect("submit manual compact");
    tokio::time::timeout(
        AUX_HOLD_TIMEOUT,
        scenario.content.server().wait_until_matching_parked(1),
    )
    .await
    .expect("compact summary reached its controlled auxiliary hold");

    scenario
        .harness
        .inject_keys(keys::CTRL_C)
        .expect("cancel compact");
    scenario
        .harness
        .wait_for_text("Compaction cancelled.", COMPACT_TERMINAL_TIMEOUT)
        .expect("compact cancellation acknowledged while auxiliary response is held");
    scenario.content.server().release_auxiliary_replies();
    scenario
        .harness
        .wait_until_stable(
            "child compact reached terminal cancellation UI",
            COMPACT_TERMINAL_TIMEOUT,
            Duration::from_secs(2),
            |h| h.contains_text("Compaction cancelled.") && !h.contains_text("Cancelling"),
        )
        .expect("cancelling state cleared on acknowledged terminal result");

    scenario.verify_child_session_info();
    scenario.return_to_parent(WAKE_DONE);
    scenario.select_row(&child_row);
    scenario
        .harness
        .inject_keys(keys::ENTER)
        .expect("reopen same child");
    scenario
        .harness
        .wait_for_text(PARENT_NAVIGATION, ROW_TIMEOUT)
        .expect("reopened child header exposes parent navigation");
    scenario.wait_for_row("CHILD_DONE");
    scenario.verify_child_session_info();
    scenario.return_to_parent(WAKE_DONE);
    scenario.finish("finished_child_manual_compact_cancel.cast");
}
