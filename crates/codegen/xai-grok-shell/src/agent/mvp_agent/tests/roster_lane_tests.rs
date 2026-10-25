//! The dashboard's subagent lane, leader side: `build_roster`'s reconcile of subagent rows
//! against the live child hosts (emission itself is covered by `agent::roster` tests).

use agent_client_protocol as acp;
use xai_grok_tools::implementations::grok_build::task::types::AgentAddress;

use super::{build_minimal_agent_for_tests, make_live_session_handle};
use crate::agent::mvp_agent::{
    ChildHost, ChildReach, ChildResidence, MvpAgent, RunningChild,
};
use crate::agent::roster::{RosterActivity, RosterEntry, RosterOrigin};
use crate::extensions::notification::SubagentDelivery;

const PARENT: &str = "lane-parent";
const CHILD: &str = "lane-child";

fn subagent_row(session_id: &str) -> RosterEntry {
    RosterEntry {
        session_id: session_id.to_owned(),
        title: Some("Child worker".to_owned()),
        cwd: "/repo".to_owned(),
        is_worktree: false,
        session_kind: Some("subagent".to_owned()),
        parent_session_id: None,
        model_id: None,
        reasoning_effort: None,
        yolo: false,
        activity: RosterActivity::Dormant,
        last_turn_summary: None,
        resident: false,
        last_change_unix_ms: 1,
        origin: RosterOrigin::Local,
    }
}

fn register_running_child(agent: &MvpAgent, child: &acp::SessionId) {
    let (handle, _cmd_tx, _cmd_rx) = make_live_session_handle(child, None);
    let (turns, _receipts) = tokio::sync::mpsc::channel(4);
    let (delivery, _delivery_rx) = tokio::sync::watch::channel(SubagentDelivery::OnTurnEnd);
    agent.register_child_session(
        child,
        ChildHost {
            parent_session_id: acp::SessionId::new(PARENT),
            reach: ChildReach::Addressed {
                address: AgentAddress::mint(21),
                residence: ChildResidence::Running(Box::new(RunningChild {
                    handle,
                    turns,
                    parent_prompt_index: Default::default(),
                    delivery,
                })),
            },
        },
    );
}

#[test]
fn running_child_row_becomes_resident_with_its_live_parent_link() {
    let agent = build_minimal_agent_for_tests();
    let child = acp::SessionId::new(CHILD);
    register_running_child(&agent, &child);

    let mut rows = vec![subagent_row(CHILD)];
    agent.reconcile_child_roster_rows(&mut rows);

    let Some(row) = rows.first() else {
        panic!("expected one roster row: {rows:?}");
    };
    assert!(row.resident, "a running child is a live session");
    assert_eq!(row.parent_session_id.as_deref(), Some(PARENT));
    assert_eq!(row.activity, RosterActivity::Idle, "no turn in flight");
}

#[test]
fn finished_child_row_marks_completed_not_resident() {
    let agent = build_minimal_agent_for_tests();
    let child = acp::SessionId::new(CHILD);
    agent.register_child_session(
        &child,
        ChildHost {
            parent_session_id: acp::SessionId::new(PARENT),
            reach: ChildReach::Addressed {
                address: AgentAddress::mint(22),
                residence: ChildResidence::Finished,
            },
        },
    );

    let mut rows = vec![subagent_row(CHILD)];
    agent.reconcile_child_roster_rows(&mut rows);

    let Some(row) = rows.first() else {
        panic!("expected one roster row: {rows:?}");
    };
    assert_eq!(row.activity, RosterActivity::Completed);
    assert!(!row.resident, "a finished child runs no actor");
    assert_eq!(row.parent_session_id.as_deref(), Some(PARENT));
}

#[test]
fn child_row_without_a_host_stays_dormant() {
    let agent = build_minimal_agent_for_tests();

    let mut rows = vec![subagent_row(CHILD)];
    agent.reconcile_child_roster_rows(&mut rows);

    let Some(row) = rows.first() else {
        panic!("expected one roster row: {rows:?}");
    };
    assert_eq!(
        row.activity,
        RosterActivity::Dormant,
        "a reaped or restarted-away child is not running"
    );
    assert!(!row.resident);
    assert_eq!(row.parent_session_id, None, "no live host to name the parent");
}

#[tokio::test]
#[serial_test::serial]
async fn build_roster_includes_live_children_beyond_recent_cap_without_hidden_or_duplicates() {
    use crate::session::info::Info;
    use crate::session::persistence::{Summary, default_model_id};
    let home = tempfile::TempDir::new().unwrap();
    let _env = xai_grok_test_support::EnvGuard::set("GROK_HOME", home.path());
    let cwd = home.path().join("repo").to_string_lossy().into_owned();
    let write = |id: &str, child: bool, hidden: Option<bool>, age: u64| {
        let info = Info { id: acp::SessionId::new(id), cwd: cwd.clone() };
        let mut summary = Summary::new(&info, default_model_id()).unwrap();
        summary.generated_title = Some(id.into());
        if child {
            summary.session_kind = Some("subagent".into());
            summary.parent_session_id = Some(PARENT.into());
        }
        summary.hidden = hidden;
        let dir = crate::session::persistence::session_dir(&info);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("summary.json");
        std::fs::write(&path, serde_json::to_vec(&summary).unwrap()).unwrap();
        std::fs::File::open(path).unwrap().set_modified(
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(age)
        ).unwrap();
    };
    write(CHILD, true, None, 1);
    write("hidden-child", true, Some(true), 1);
    for index in 0..205 {
        write(&format!("recent-{index}"), false, None, 10_000 + index);
    }
    let recent = crate::session::persistence::list_recent_summaries(200).await.unwrap();
    assert_eq!(recent.len(), 200);
    assert!(!recent.iter().any(|summary| summary.info.id.0.as_ref() == CHILD));
    let agent = build_minimal_agent_for_tests();
    let child = acp::SessionId::new(CHILD);
    register_running_child(&agent, &child);
    register_running_child(&agent, &acp::SessionId::new("hidden-child"));
    let rows = agent.build_roster().await;
    assert_eq!(rows.iter().filter(|row| row.session_id == CHILD).count(), 1);
    let row = rows.iter().find(|row| row.session_id == CHILD).unwrap();
    assert!(row.resident);
    assert_eq!(row.parent_session_id.as_deref(), Some(PARENT));
    assert!(!rows.iter().any(|row| row.session_id == "hidden-child"));

    write(CHILD, true, None, 20_000);
    let rows = agent.build_roster().await;
    assert_eq!(rows.iter().filter(|row| row.session_id == CHILD).count(), 1);
    agent.session_registry.clear_host(&child);
    let rows = agent.build_roster().await;
    let row = rows.iter().find(|row| row.session_id == CHILD).unwrap();
    assert!(!row.resident);
    assert_eq!(row.activity, RosterActivity::Dormant);
    assert_eq!(row.parent_session_id.as_deref(), Some(PARENT));
    let restarted = build_minimal_agent_for_tests();
    let rows = restarted.build_roster().await;
    let row = rows.iter().find(|row| row.session_id == CHILD).unwrap();
    assert!(!row.resident);
    assert_eq!(row.parent_session_id.as_deref(), Some(PARENT));
    assert!(!rows.iter().any(|row| row.session_id == "hidden-child"));
}

#[test]
fn non_subagent_rows_are_left_to_the_merge() {
    let agent = build_minimal_agent_for_tests();
    let child = acp::SessionId::new(CHILD);
    register_running_child(&agent, &child);

    let mut rows = vec![RosterEntry {
        session_kind: None,
        ..subagent_row("root-session")
    }];
    agent.reconcile_child_roster_rows(&mut rows);

    let Some(row) = rows.first() else {
        panic!("expected one roster row: {rows:?}");
    };
    assert_eq!(row.activity, RosterActivity::Dormant);
    assert!(!row.resident);
    assert_eq!(row.parent_session_id, None);
}
