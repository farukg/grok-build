//! Child-role stamping (gb-subid-6): a session id a parent view knows as its `child_session_id`
//! must open as `AgentRole::Child`, so its prompts carry `_meta.childOf` and survive the leader
//! losing the child actor (restart, reap).

use super::{AgentRole, stamp_child_role_for_session};
use crate::app::agent::AgentId;
use crate::app::agent_view::test_agent_view;
use crate::app::session_views::SessionViews;

fn views_with_parent_and_child() -> (SessionViews, AgentId, AgentId) {
    let parent = AgentId(0);
    let child = AgentId(1);
    let mut views = SessionViews::new();
    let mut parent_view = test_agent_view(Some("parent-session"), ".".into());
    let mut info = crate::app::subagent::test_support::make_info();
    info.subagent_id = "child-session".into();
    info.child_session_id = "child-session".into();
    parent_view
        .subagent_sessions
        .insert("child-session".to_owned(), info);
    views.insert(parent, parent_view);
    views.insert(child, test_agent_view(Some("unrelated"), ".".into()));
    (views, parent, child)
}

#[test]
fn known_child_session_opens_with_the_child_role() {
    let (mut views, parent, child) = views_with_parent_and_child();
    // The generic open path inserts a fresh Root view for the child's session id.
    let opened = AgentId(2);
    views.insert(opened, test_agent_view(Some("child-session"), ".".into()));

    stamp_child_role_for_session(&mut views, opened, "child-session", None);

    match &views.get(&opened).expect("opened view survives").role {
        AgentRole::Child(link) => {
            assert_eq!(link.parent, Some(parent));
            assert_eq!(link.parent_session_id.0.as_ref(), "parent-session");
            assert_eq!(link.subagent_id, "child-session");
        }
        AgentRole::Root => panic!("a known child id must not keep the Root role"),
    }
    // The stale unrelated view is untouched; the parent stays a root.
    assert!(matches!(views.get(&child).expect("unrelated view").role, AgentRole::Root));
}

#[test]
fn unknown_session_keeps_the_root_role() {
    let (mut views, _parent, _child) = views_with_parent_and_child();
    let opened = AgentId(2);
    views.insert(opened, test_agent_view(Some("never-heard-of"), ".".into()));

    stamp_child_role_for_session(&mut views, opened, "never-heard-of", None);

    assert!(
        matches!(views.get(&opened).expect("opened view survives").role, AgentRole::Root),
        "an id no parent knows stays a root session"
    );
}

#[test]
fn child_role_prompts_target_the_parent_for_childof() {
    let (mut views, _parent, _child) = views_with_parent_and_child();
    let opened = AgentId(2);
    views.insert(opened, test_agent_view(Some("child-session"), ".".into()));
    stamp_child_role_for_session(&mut views, opened, "child-session", None);

    let target = views
        .get(&opened)
        .expect("opened view survives")
        .role
        .prompt_target();
    assert_eq!(
        target,
        crate::app::agent_view::PromptTarget::ChildOf(agent_client_protocol::SessionId::new(
            "parent-session"
        )),
        "sends from the stamped view carry `_meta.childOf`"
    );
}
