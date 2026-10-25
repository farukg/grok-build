use super::*;
use super::super::state::{Grouping, PersistedRowId};
use std::collections::BTreeSet;
use std::time::Duration;

struct WriterOverride;

impl Drop for WriterOverride {
    fn drop(&mut self) {
        TEST_WRITER.with(|slot| *slot.borrow_mut() = None);
    }
}

fn key(id: &str) -> PersistedRowId {
    PersistedRowId::TopLevel { session_id: id.into() }
}

fn full(root: &str) -> DashboardWrite {
    let mut saved = PersistedDashboard::defaults();
    saved.enabled = false;
    saved.grouping = Grouping::Directory;
    saved.pinned = BTreeSet::from([key(root)]);
    saved.reorder = vec![key(root), key("child-a"), key("child-b")];
    DashboardWrite::Full(saved)
}

fn child(id: &str) -> DashboardWrite {
    DashboardWrite::Child(ChildLayoutUpdate {
        scope: BTreeSet::from([key("child-a"), key("child-b")]),
        pinned: BTreeSet::from([key(id)]),
        reorder: vec![key(id)],
    })
}

#[test]
fn ordered_interleaved_full_and_scoped_writes_serialize_entire_transactions() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    std::fs::write(&path, "[preferences]\nkeep = 'untouched'\n").unwrap();
    let (started, entered) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let (observed, states) = mpsc::channel();
    let mut first = true;
    let writer = DashboardWriter::start(move |path, write| {
        if first {
            first = false;
            started.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        write.write_to_path(path)?;
        let doc: toml_edit::DocumentMut = std::fs::read_to_string(path)?.parse().unwrap();
        let strings = |field: &str| {
            doc["dashboard"][field].as_array().unwrap().iter()
                .map(|value| value.as_str().unwrap().to_owned()).collect::<Vec<_>>()
        };
        observed.send((strings("pinned"), strings("reorder"))).unwrap();
        assert_eq!(doc["preferences"]["keep"].as_str(), Some("untouched"));
        assert_eq!(doc["dashboard"]["enabled"].as_bool(), Some(false));
        assert_eq!(doc["dashboard"]["grouping"].as_str(), Some("directory"));
        Ok(())
    }).unwrap();
    TEST_WRITER.with(|slot| *slot.borrow_mut() = Some((writer, path.clone())));
    let _override = WriterOverride;
    let first = enqueue(full("root-one")).unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    let second = enqueue(child("child-b")).unwrap();
    let third = enqueue(full("root-two")).unwrap();
    let fourth = enqueue(child("child-a")).unwrap();
    assert!(matches!(states.try_recv(), Err(mpsc::TryRecvError::Empty)));
    release.send(()).unwrap();
    for receipt in [first, second, third, fourth] {
        receipt.blocking_recv().unwrap().unwrap();
    }
    let expected = [
        (vec!["top:root-one"], vec!["top:root-one", "top:child-a", "top:child-b"]),
        (vec!["top:root-one", "top:child-b"], vec!["top:root-one", "top:child-b"]),
        (vec!["top:root-two"], vec!["top:root-two", "top:child-a", "top:child-b"]),
        (vec!["top:root-two", "top:child-a"], vec!["top:root-two", "top:child-a"]),
    ];
    for (pins, order) in expected {
        let actual = states.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(actual.0, pins);
        assert_eq!(actual.1, order);
    }
    let restarted = super::super::state::load_persisted_from_path(&path).unwrap();
    assert_eq!(restarted.pinned, BTreeSet::from([key("root-two"), key("child-a")]));
    assert_eq!(restarted.reorder, vec![key("root-two"), key("child-a")]);
}

#[test]
fn failed_full_transaction_does_not_stop_later_scoped_and_full_writes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let unreadable = b"[preferences]\nkeep = '\xff'\n";
    std::fs::write(&path, unreadable).unwrap();
    with_test_writer(path.clone(), |_| {}, || {
        let error = enqueue(full("failed-root")).unwrap().blocking_recv().unwrap().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read(&path).unwrap(), unreadable);
        std::fs::write(&path, "[dashboard]\npinned = ['top:root']\nreorder = ['top:root', 'top:child-a']\n[preferences]\nkeep = 'repaired'\n").unwrap();
        let scoped = enqueue(child("child-b")).unwrap();
        let full = enqueue(full("final-root")).unwrap();
        scoped.blocking_recv().unwrap().unwrap();
        full.blocking_recv().unwrap().unwrap();
    }).unwrap();
    let restarted = super::super::state::load_persisted_from_path(&path).unwrap();
    assert_eq!(restarted.pinned, BTreeSet::from([key("final-root")]));
    assert_eq!(restarted.reorder, vec![key("final-root"), key("child-a"), key("child-b")]);
    let doc: toml_edit::DocumentMut = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    assert_eq!(doc["preferences"]["keep"].as_str(), Some("repaired"));
}

#[test]
fn dropped_receipt_does_not_cancel_or_reorder_a_scoped_write() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let writer = DashboardWriter::start(|path, write| write.write_to_path(path)).unwrap();
    TEST_WRITER.with(|slot| *slot.borrow_mut() = Some((writer, path.clone())));
    let _override = WriterOverride;
    drop(enqueue(full("root")).unwrap());
    drop(enqueue(child("child-b")).unwrap());
    enqueue(child("child-a")).unwrap().blocking_recv().unwrap().unwrap();
    let restarted = super::super::state::load_persisted_from_path(&path).unwrap();
    assert_eq!(restarted.pinned, BTreeSet::from([key("root"), key("child-a")]));
    assert_eq!(restarted.reorder, vec![key("root"), key("child-a")]);
}
