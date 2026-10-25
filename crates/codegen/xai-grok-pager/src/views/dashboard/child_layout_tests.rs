use super::*;
use crate::views::dashboard::state::{Grouping, load_persisted_from_path};

fn key(id: &str) -> PersistedRowId {
    PersistedRowId::TopLevel { session_id: id.into() }
}

#[test]
fn scoped_write_preserves_unrelated_config_and_root_layout_on_restart() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let original = "# preserved\nmodel = 'unchanged'\n[dashboard]\nenabled = false\ngrouping = 'directory'\npinned = ['top:root-a', 'top:child-a', 'top:root-b']\nreorder = ['top:root-b', 'top:child-a', 'top:root-a', 'top:child-b']\ncustom = 'retained'\n[dashboard.onboarding]\nkeep = true\n[ui]\naccent = 'blue'\n";
    std::fs::write(&path, original).unwrap();
    let update = ChildLayoutUpdate {
        scope: [key("child-a"), key("child-b")].into(),
        pinned: [key("child-b")].into(),
        reorder: vec![key("child-b"), key("child-a")],
    };
    update.write_to_path(&path).unwrap();
    let restarted = load_persisted_from_path(&path).unwrap();
    assert!(!restarted.enabled);
    assert_eq!(restarted.grouping, Grouping::Directory);
    assert_eq!(restarted.pinned, [key("root-a"), key("root-b"), key("child-b")].into());
    assert_eq!(restarted.reorder, vec![key("root-b"), key("child-b"), key("root-a"), key("child-a")]);
    let before: toml_edit::DocumentMut = original.parse().unwrap();
    let after: toml_edit::DocumentMut = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    for name in ["model", "ui"] {
        assert_eq!(before[name].to_string(), after[name].to_string());
    }
    for name in ["enabled", "grouping", "custom", "onboarding"] {
        assert_eq!(before["dashboard"][name].to_string(), after["dashboard"][name].to_string());
    }
    update.write_to_path(&path).unwrap();
    assert_eq!(load_persisted_from_path(&path).unwrap().reorder, restarted.reorder);
}

#[test]
fn malformed_layout_is_not_overwritten_by_child_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let original = "[dashboard]\npinned = 'not an array'\nreorder = []\n";
    std::fs::write(&path, original).unwrap();
    let update = ChildLayoutUpdate { scope: [key("child")].into(), pinned: [key("child")].into(), reorder: Vec::new() };
    assert!(update.write_to_path(&path).is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
}
