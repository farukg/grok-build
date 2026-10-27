use super::*;
use crate::app::agent_test_fixtures::running_bg_task;
use crate::app::agent_view::test_agent_view;
use xai_grok_test_support::acp_fixtures;

#[test]
fn cumulative_wire_stdout_revisions_distinguish_equal_length_replacements_and_final_empty() {
    let mut view = test_agent_view(Some("inline-wire"), std::path::PathBuf::from("/tmp"));
    view.session.bg_tasks.insert("task".into(), running_bg_task("task", false));
    view.session.bg_tool_call_to_task.insert("call".into(), "task".into());
    let mut revision = 0;
    for stdout in ["AAAA", "BBBB"] {
        let update = acp_fixtures::tool_call_update("call", acp::ToolCallUpdateFields::new()
            .raw_output(Some(serde_json::json!({"type":"Bash", "output_for_prompt":stdout}))));
        assert!(route_bg_task_stdout(&update, &mut view));
        assert_eq!(view.session.bg_tasks.get("task").unwrap().stdout, stdout);
        let next = view.inline_sources.task_revision("task");
        assert_ne!(next, revision);
        revision = next;
    }
    let update = acp_fixtures::tool_call_update("call", acp::ToolCallUpdateFields::new()
        .raw_output(Some(serde_json::json!({"type":"Bash", "output_for_prompt":""}))));
    assert!(route_bg_task_stdout(&update, &mut view));
    assert_eq!(view.session.bg_tasks.get("task").unwrap().stdout, "BBBB");
    assert_eq!(view.inline_sources.task_revision("task"), revision);
    view.append_bg_task_stdout("task", "monitor event");
    assert_eq!(view.session.bg_tasks.get("task").unwrap().stdout, "BBBB\nmonitor event");
    assert_ne!(view.inline_sources.task_revision("task"), revision);
}
