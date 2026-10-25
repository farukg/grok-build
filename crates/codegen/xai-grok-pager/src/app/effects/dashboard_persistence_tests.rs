use super::*;
use crate::views::dashboard::{PersistedDashboard, PersistedRowId};
use crate::views::dashboard::child_layout::ChildLayoutUpdate;
use std::collections::BTreeSet;
use std::time::Duration;

fn key(id: &str) -> PersistedRowId {
    PersistedRowId::TopLevel { session_id: id.into() }
}

#[tokio::test(flavor = "current_thread")]
async fn dashboard_effects_enqueue_full_and_scoped_writes_before_receipt_tasks_are_polled() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config.toml");
    let (written, observed) = std::sync::mpsc::channel();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let (progress, _progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut tasks = JoinSet::new();
    crate::views::dashboard::persistence::with_test_writer(config.clone(), move |path| {
        let doc: toml_edit::DocumentMut = std::fs::read_to_string(path).unwrap().parse().unwrap();
        written.send(doc["dashboard"]["reorder"].as_array().unwrap().iter()
            .map(|value| value.as_str().unwrap().to_owned()).collect::<Vec<_>>()).unwrap();
    }, || {
        let mut full = PersistedDashboard::defaults();
        full.reorder = vec![key("root"), key("child-a"), key("child-b")];
        let scoped = ChildLayoutUpdate {
            scope: BTreeSet::from([key("child-a"), key("child-b")]),
            pinned: BTreeSet::from([key("child-b")]),
            reorder: vec![key("child-b")],
        };
        for effect in [Effect::PersistDashboard(full), Effect::PersistDashboardChildLayout(scoped)] {
            execute(effect, &mut tasks, &tx, Path::new("."), &SessionFlags::default(), &progress);
        }
        // The current-thread runtime cannot poll spawned receipt tasks before the first await.
        assert_eq!(observed.recv_timeout(Duration::from_secs(5)).unwrap(),
            vec!["top:root", "top:child-a", "top:child-b"]);
        assert_eq!(observed.recv_timeout(Duration::from_secs(5)).unwrap(),
            vec!["top:root", "top:child-b"]);
    }).unwrap();
    while let Some(result) = tasks.join_next().await {
        assert!(matches!(result.unwrap(), TaskResult::CancelComplete));
    }
    let restarted = crate::views::dashboard::state::load_persisted_from_path(&config).unwrap();
    assert_eq!(restarted.pinned, BTreeSet::from([key("child-b")]));
    assert_eq!(restarted.reorder, vec![key("root"), key("child-b")]);
}
