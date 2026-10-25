use super::*;
use crate::session::persistence::default_model_id;
use crate::session::storage::SessionListVisibility;

#[tokio::test]
async fn roster_visibility_admits_disk_children_without_changing_history() {
    let temp = tempfile::tempdir().unwrap();
    let adapter = JsonlStorageAdapter::with_root(temp.path().to_path_buf());
    for (id, kind, hidden) in [
        ("root", None, None),
        ("child", Some("subagent"), None),
        ("explicit-hidden-child", Some("subagent"), Some(true)),
        ("headless", Some("headless"), None),
    ] {
        let info = Info { id: acp::SessionId::new(id), cwd: "/repo".into() };
        let mut summary = Summary::new(&info, default_model_id()).unwrap();
        summary.generated_title = Some(id.into());
        summary.session_kind = kind.map(str::to_owned);
        summary.hidden = hidden;
        if kind == Some("subagent") {
            summary.parent_session_id = Some("parent".into());
        }
        let dir = adapter.session_dir(&info);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("summary.json"), serde_json::to_vec(&summary).unwrap()).unwrap();
    }
    for summaries in [
        adapter.list_sessions_with_visibility(None, SessionListVisibility::Roster).await.unwrap(),
        adapter.list_sessions_recent_with_visibility(20, SessionListVisibility::Roster).await.unwrap(),
    ] {
        let ids: std::collections::BTreeSet<_> = summaries.iter().map(|summary| summary.info.id.0.as_ref()).collect();
        assert_eq!(ids, ["root", "child"].into());
        assert_eq!(summaries.iter().find(|summary| summary.info.id.0.as_ref() == "child").unwrap().parent_session_id.as_deref(), Some("parent"));
    }
    let history = StorageAdapter::list_sessions(&adapter, None).await.unwrap();
    assert!(history.iter().any(|summary| summary.info.id.0.as_ref() == "root"));
    assert!(!history.iter().any(|summary| summary.is_subagent_kind()));
    assert!(history.iter().any(|summary| summary.info.id.0.as_ref() == "headless"));
    let recent = adapter.list_sessions_recent(20).await.unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].info.id.0.as_ref(), "root");
}
