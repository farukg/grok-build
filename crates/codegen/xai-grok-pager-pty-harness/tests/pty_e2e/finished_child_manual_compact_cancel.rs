//! PTY: manually compact a finished child, cancel while its auxiliary summary request is held,
//! then verify the child can be queried and reopened from the parent's transcript.
#![cfg(unix)]
#[allow(unused_imports)]
use super::common::*;
use super::send_subagent_message_row::{CHILD_LABEL, ROW_TIMEOUT, SendMessageScenario, WAKE_DONE};

const TAKEOVER_HINT: &str = "q/Esc:back";
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
        .expect("parent and child are idle");

    scenario.focus_scrollback();
    let child_row = CHILD_LABEL;
    scenario.select_row(&child_row);
    scenario
        .harness
        .inject_keys(keys::ENTER)
        .expect("open finished child");
    scenario
        .harness
        .wait_for_text(TAKEOVER_HINT, ROW_TIMEOUT)
        .expect("finished child view opened");
    scenario.wait_for_row("CHILD_DONE");

    // The shell tags compaction summary requests xai-compact-<uuid>; hold that auxiliary specifically.
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

    scenario
        .harness
        .inject_keys(b"/session-info\r")
        .expect("request child session info");
    scenario
        .harness
        .wait_for_text("Session info", Duration::from_secs(20))
        .expect("child session info opens after cancelled compact");
    scenario
        .harness
        .inject_keys(keys::ESC)
        .expect("close session info");
    scenario
        .harness
        .wait_for_text_absent("Session info", Duration::from_secs(10))
        .expect("session info closes");

    scenario
        .harness
        .inject_keys(keys::Q)
        .expect("return to parent");
    scenario
        .harness
        .wait_for_text_absent(TAKEOVER_HINT, ROW_TIMEOUT)
        .expect("return to parent after child session info");
    scenario.select_row(&child_row);
    scenario
        .harness
        .inject_keys(keys::ENTER)
        .expect("reopen same child");
    scenario
        .harness
        .wait_for_text(TAKEOVER_HINT, ROW_TIMEOUT)
        .expect("same child can be reopened after terminal cancellation");
    scenario
        .harness
        .inject_keys(keys::Q)
        .expect("return to parent before cleanup");
    scenario.finish("finished_child_manual_compact_cancel.cast");
}
