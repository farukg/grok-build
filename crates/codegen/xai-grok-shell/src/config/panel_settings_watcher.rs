//! Watches the pager's panel-settings root (`<grok home>/panel-settings`) so one CLI
//! window applies what another window saved there: `global.toml` and
//! `sessions/<session id>.toml`.
//!
//! One non-recursive watch per directory (the root, its `sessions/` subdir, and the
//! root's parent, so a root created after start still gets its watches attached by
//! [`PanelSettingsWatcher::refresh_watches`]); no per-session watches, no scans.
//! Events are debounced and access-filtered like the other config watchers here.
//! Only typed changes leave this module; every consumer re-reads the files and
//! compares against what it holds, so a window's own writes are absorbed by
//! equality instead of by self-write suppression.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::sync::{Arc, Mutex};

use notify::RecursiveMode;
use notify_debouncer_mini::{DebounceEventResult, Debouncer};
use tokio::sync::mpsc;

use super::watcher::{
    AccessFilteredWatcher, WatchDiagnosis, attach_new_refresh_dirs, new_filtered_debouncer, paths_equal,
};

/// Coalesces the create+rename pair of an atomic settings write into one change.
const DEBOUNCE: Duration = Duration::from_millis(300);
const GLOBAL_FILE: &str = "global.toml";
const SESSIONS_DIR: &str = "sessions";
const SESSION_EXTENSION: &str = "toml";

/// What changed under the watched panel-settings root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelSettingsChange {
    /// `global.toml` changed (written, replaced or removed).
    Global,
    /// `sessions/<session id>.toml` changed; only views holding `session` respond.
    Session { session: String },
    /// The root's shape changed (created or removed). The consumer re-reads every
    /// scope it holds and calls [`PanelSettingsWatcher::refresh_watches`], so a first
    /// write made before the directory was watchable is still applied.
    Root,
}

/// A path under the root, as one typed change. Temp files, backups and files of
/// other directories (including the watched parent) are `None`.
fn classify(path: &Path, root: &Path, sessions: &Path) -> Option<PanelSettingsChange> {
    if paths_equal(path, root) || paths_equal(path, sessions) {
        return Some(PanelSettingsChange::Root);
    }
    let parent = path.parent()?;
    let name = path.file_name().and_then(|name| name.to_str())?;
    if name == GLOBAL_FILE && paths_equal(parent, root) {
        return Some(PanelSettingsChange::Global);
    }
    if path.extension().is_some_and(|extension| extension == SESSION_EXTENSION)
        && paths_equal(parent, sessions)
        && let Some(session) = path.file_stem().and_then(|stem| stem.to_str())
        && !session.is_empty()
    {
        return Some(PanelSettingsChange::Session {
            session: session.to_owned(),
        });
    }
    None
}

const MAX_PENDING: usize = 64;

#[derive(Default)]
enum PendingChanges {
    #[default]
    Quiet,
    Changes(Vec<PanelSettingsChange>),
    Root,
}

impl PendingChanges {
    fn merge(&mut self, change: PanelSettingsChange) {
        if change == PanelSettingsChange::Root {
            *self = Self::Root;
            return;
        }
        match self {
            Self::Root => {}
            Self::Quiet => *self = Self::Changes(vec![change]),
            Self::Changes(changes) => {
                if !changes.contains(&change) {
                    if changes.len() == MAX_PENDING {
                        *self = Self::Root;
                    } else {
                        changes.push(change);
                    }
                }
            }
        }
    }

    fn pop(&mut self) -> Option<PanelSettingsChange> {
        match self {
            Self::Quiet => None,
            Self::Root => { *self = Self::Quiet; Some(PanelSettingsChange::Root) }
            Self::Changes(changes) => {
                let change = changes.pop();
                if changes.is_empty() { *self = Self::Quiet; }
                change
            }
        }
    }
}

struct ChangeSender {
    pending: Arc<Mutex<PendingChanges>>,
    wake: mpsc::Sender<()>,
}

impl ChangeSender {
    fn send(&self, change: PanelSettingsChange) {
        let mut pending = self.pending.lock().unwrap_or_else(|error| error.into_inner());
        pending.merge(change);
        let _ = self.wake.try_send(());
    }

    fn events(&self, result: DebounceEventResult, root: &Path, sessions: &Path) {
        match result {
            Ok(events) => {
                for event in events {
                    if let Some(change) = classify(&event.path, root, sessions) {
                        self.send(change);
                    }
                }
            }
            Err(error) => {
                let diagnosis = WatchDiagnosis::from(&error);
                tracing::warn!(%diagnosis, "panel settings watcher resynchronizing");
                self.send(PanelSettingsChange::Root);
            }
        }
    }
}

/// Bounded pending scopes with one wake, shared with the native callback.
pub struct PanelSettingsReceiver {
    pending: Arc<Mutex<PendingChanges>>,
    wake: mpsc::Receiver<()>,
}

impl PanelSettingsReceiver {
    fn take(&mut self) -> Option<PanelSettingsChange> {
        let mut pending = self.pending.lock().unwrap_or_else(|error| error.into_inner());
        pending.pop()
    }

    pub async fn recv(&mut self) -> Option<PanelSettingsChange> {
        loop {
            if let Some(change) = self.take() { return Some(change); }
            self.wake.recv().await?;
        }
    }

    #[cfg(test)]
    fn try_recv(&mut self) -> Result<PanelSettingsChange, mpsc::error::TryRecvError> {
        if let Some(change) = self.take() { return Ok(change); }
        self.wake.try_recv()?;
        self.take().ok_or(mpsc::error::TryRecvError::Empty)
    }
}

fn change_channel() -> (ChangeSender, PanelSettingsReceiver) {
    let pending = Arc::new(Mutex::new(PendingChanges::Quiet));
    let (wake, rx) = mpsc::channel(1);
    (ChangeSender { pending: pending.clone(), wake: wake.clone() },
     PanelSettingsReceiver { pending, wake: rx })
}

/// Watches the panel-settings root for writes from any process sharing it.
///
/// Drop stops watching. Slow consumers coalesce pending scopes, overflowing to Root.
pub struct PanelSettingsWatcher {
    debouncer: Debouncer<AccessFilteredWatcher>,
    refresh_dirs: Vec<(PathBuf, RecursiveMode)>,
    refreshed_dirs: HashSet<PathBuf>,
}

impl PanelSettingsWatcher {
    /// Watches `root` and `root/sessions` (both non-recursive), plus `root`'s parent
    /// so a root that does not exist yet is still caught by [`Self::refresh_watches`].
    /// `debounce` overrides the coalescing window (tests).
    /// `None` when no watch could be installed at all.
    pub fn start(
        root: &Path,
        debounce: Option<Duration>,
    ) -> Option<(Self, PanelSettingsReceiver)> {
        let root = std::path::absolute(root).ok()?;
        let sessions = root.join(SESSIONS_DIR);
        let (tx, rx) = change_channel();
        let root_for_events = root.clone();
        let sessions_for_events = sessions.clone();
        let mut debouncer = new_filtered_debouncer(
            debounce.unwrap_or(DEBOUNCE),
            move |res: DebounceEventResult| {
                tx.events(res, &root_for_events, &sessions_for_events);
            },
        )
        .map_err(|error| tracing::warn!(diagnosis = %WatchDiagnosis::from(&error), "failed to create panel settings watcher"))
        .ok()?;

        let mut refresh_dirs = vec![
            (root.clone(), RecursiveMode::NonRecursive),
            (sessions, RecursiveMode::NonRecursive),
        ];
        let stable_parent = root.parent().filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf);
        if let Some(parent) = &stable_parent {
            refresh_dirs.push((parent.clone(), RecursiveMode::NonRecursive));
        }
        let mut refreshed_dirs = HashSet::new();
        attach_new_refresh_dirs(
            &mut debouncer,
            &refresh_dirs,
            &mut refreshed_dirs,
            "failed to watch panel settings directory",
        );
        if refreshed_dirs.is_empty() {
            return None;
        }
        tracing::debug!(root = %root.display(), "panel settings watcher started");
        Some((
            Self {
                debouncer,
                refresh_dirs,
                refreshed_dirs,
            },
            rx,
        ))
    }

    /// Reinstalls root/session watches after replacement or event loss, retaining the parent.
    /// Call after a [`PanelSettingsChange::Root`].
    pub fn refresh_watches(&mut self) -> bool {
        self.refreshed_dirs.retain(|target| {
            if self.refresh_dirs.get(2).map(|(parent, _)| parent) == Some(target) { return true; }
            AccessFilteredWatcher::invalidate_directory(self.debouncer.watcher(), target);
            false
        });
        attach_new_refresh_dirs(
            &mut self.debouncer,
            &self.refresh_dirs,
            &mut self.refreshed_dirs,
            "failed to watch panel settings directory",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn drain(rx: &mut PanelSettingsReceiver) -> Vec<PanelSettingsChange> {
        let mut seen = Vec::new();
        while let Ok(change) = rx.try_recv() {
            seen.push(change);
        }
        seen
    }

    /// Polls like the config-watcher tests: no tokio runtime, one try_recv per tick.
    fn wait_for(
        rx: &mut PanelSettingsReceiver,
        want: PanelSettingsChange,
    ) -> Option<PanelSettingsChange> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            for change in drain(rx) {
                assert_eq!(change, want, "only the wanted change may arrive");
                return Some(change);
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn assert_silent(rx: &mut PanelSettingsReceiver) {
        let deadline = std::time::Instant::now() + Duration::from_millis(700);
        loop {
            let seen = drain(rx);
            assert!(
                seen.is_empty(),
                "unexpected changes: {seen:?}"
            );
            if std::time::Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The store's atomic write shape: a temp file beside the target, then a rename.
    fn write_atomically(path: &Path, contents: &str) {
        let mut temp = path.with_extension("toml.tmpnew");
        fs::write(&temp, contents).expect("write temp");
        fs::rename(&temp, path).expect("rename into place");
    }

    const DEBOUNCE_TEST: Duration = Duration::from_millis(60);

    #[test]
    fn global_toml_write_emits_global() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("panel-settings");
        fs::create_dir_all(root.join("sessions")).expect("create root");
        let (_watcher, mut rx) = PanelSettingsWatcher::start(&root, Some(DEBOUNCE_TEST))
            .expect("watcher starts on an existing root");

        write_atomically(&root.join("global.toml"), "version = 1\n");
        wait_for(&mut rx, PanelSettingsChange::Global).expect("global change observed");
    }

    #[test]
    fn session_toml_write_emits_that_session_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("panel-settings");
        fs::create_dir_all(root.join("sessions")).expect("create sessions");
        let (_watcher, mut rx) = PanelSettingsWatcher::start(&root, Some(DEBOUNCE_TEST))
            .expect("watcher starts");

        write_atomically(&root.join("sessions").join("sess-42.toml"), "version = 1\n");
        wait_for(
            &mut rx,
            PanelSettingsChange::Session {
                session: "sess-42".to_owned(),
            },
        )
        .expect("session change observed");
    }

    #[test]
    fn temp_and_backup_files_emit_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("panel-settings");
        fs::create_dir_all(root.join("sessions")).expect("create root");
        let (_watcher, mut rx) = PanelSettingsWatcher::start(&root, Some(DEBOUNCE_TEST))
            .expect("watcher starts");

        fs::write(root.join(".grok-tmp-1"), "").expect("write temp");
        fs::write(root.join("global.toml.bad"), "version = 1\n").expect("write backup");
        fs::write(root.join("sessions").join("notes.md"), "").expect("write other kind");
        fs::write(dir.path().join("unrelated.toml"), "").expect("write outside root");
        assert_silent(&mut rx);
    }

    #[test]
    fn two_writes_in_one_window_come_as_one_change() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("panel-settings");
        fs::create_dir_all(&root).expect("create root");
        let (_watcher, mut rx) = PanelSettingsWatcher::start(&root, Some(DEBOUNCE_TEST))
            .expect("watcher starts");

        write_atomically(&root.join("global.toml"), "version = 1\n");
        write_atomically(&root.join("global.toml"), "version = 2\n");
        wait_for(&mut rx, PanelSettingsChange::Global).expect("one change arrives");
        assert_silent(&mut rx);
    }

    #[test]
    fn a_root_created_after_start_is_reported_and_then_watched() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("panel-settings");
        let (mut watcher, mut rx) = PanelSettingsWatcher::start(&root, Some(DEBOUNCE_TEST))
            .expect("parent watch installs even without the root");

        fs::create_dir_all(&root).expect("create root late");
        write_atomically(&root.join("global.toml"), "version = 1\n");
        wait_for(&mut rx, PanelSettingsChange::Root).expect("root creation observed");
        assert!(
            watcher.refresh_watches(),
            "the now-existing root gets its watch attached"
        );

        write_atomically(&root.join("global.toml"), "version = 2\n");
        wait_for(&mut rx, PanelSettingsChange::Global).expect("late root is watched");
    }

    #[test]
    fn events_stay_inside_their_own_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = dir.path().join("panel-settings");
        let second = dir.path().join("other-settings");
        fs::create_dir_all(first.join("sessions")).expect("create first root");
        fs::create_dir_all(second.join("sessions")).expect("create second root");
        let (_first_watcher, mut first_rx) =
            PanelSettingsWatcher::start(&first, Some(DEBOUNCE_TEST)).expect("first starts");
        let (_second_watcher, mut second_rx) =
            PanelSettingsWatcher::start(&second, Some(DEBOUNCE_TEST)).expect("second starts");

        write_atomically(&first.join("global.toml"), "version = 1\n");
        wait_for(&mut first_rx, PanelSettingsChange::Global).expect("own root observes");
        assert_silent(&mut second_rx);
    }
    #[test]
    fn slow_consumer_deduplicates_across_callbacks_and_overflow_resyncs_every_scope() {
        let (tx, mut rx) = change_channel();
        for _ in 0..1000 { tx.send(PanelSettingsChange::Global); }
        assert_eq!(rx.wake.len(), 1);
        assert_eq!(drain(&mut rx), vec![PanelSettingsChange::Global]);
        for index in 0..=MAX_PENDING {
            tx.send(PanelSettingsChange::Session { session: format!("session-{index}") });
        }
        tx.send(PanelSettingsChange::Global);
        tx.send(PanelSettingsChange::Session { session: "latest".to_owned() });
        assert_eq!(rx.wake.len(), 1);
        assert_eq!(drain(&mut rx), vec![PanelSettingsChange::Root]);
        tx.send(PanelSettingsChange::Global);
        assert_eq!(drain(&mut rx), vec![PanelSettingsChange::Global]);
    }

    #[test]
    fn backend_errors_request_one_root_resync_with_native_watch_repair() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("panel-settings");
        fs::create_dir_all(root.join("sessions")).unwrap();
        let (mut watcher, mut native_rx) = PanelSettingsWatcher::start(&root, Some(DEBOUNCE_TEST)).unwrap();
        let (tx, mut rx) = change_channel();
        tx.send(PanelSettingsChange::Global);
        for _ in 0..100 {
            tx.events(Err(notify::Error::generic("backend failure")), &root, &root.join("sessions"));
        }
        assert_eq!(drain(&mut rx), vec![PanelSettingsChange::Root]);
        assert!(watcher.refresh_watches());
        write_atomically(&root.join("global.toml"), "version = 1\n");
        wait_for(&mut native_rx, PanelSettingsChange::Global).expect("error resync reinstalls a working native watch");
        drop(tx);
        assert!(matches!(rx.try_recv(), Err(mpsc::error::TryRecvError::Disconnected)));
    }

    #[test]
    fn rapid_root_and_sessions_replacement_reinstalls_watches_for_later_edits() {
        for replace_root in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("panel-settings");
            let sessions = root.join("sessions");
            fs::create_dir_all(&sessions).unwrap();
            let (mut watcher, mut rx) = PanelSettingsWatcher::start(&root, Some(DEBOUNCE_TEST)).unwrap();
            let replaced = if replace_root { &root } else { &sessions };
            fs::remove_dir_all(replaced).unwrap();
            fs::create_dir_all(&sessions).unwrap();
            wait_for(&mut rx, PanelSettingsChange::Root).expect("replacement observed by stable parent");
            assert!(watcher.refresh_watches());
            assert_silent(&mut rx);
            write_atomically(&sessions.join("after.toml"), "version = 1\n");
            wait_for(&mut rx, PanelSettingsChange::Session { session: "after".to_owned() }).expect("new sessions inode watched");
            write_atomically(&root.join("global.toml"), "version = 1\n");
            wait_for(&mut rx, PanelSettingsChange::Global).expect("new root inode watched");
        }
    }

    #[cfg(unix)]
    #[test]
    fn replaced_root_symlink_keeps_the_configured_parent_watch() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        fs::create_dir_all(first.join("sessions")).unwrap();
        fs::create_dir_all(second.join("sessions")).unwrap();
        let root = dir.path().join("panel-settings");
        std::os::unix::fs::symlink(&first, &root).unwrap();
        let (mut watcher, mut rx) = PanelSettingsWatcher::start(&root, Some(DEBOUNCE_TEST)).unwrap();
        fs::remove_file(&root).unwrap();
        std::os::unix::fs::symlink(&second, &root).unwrap();
        wait_for(&mut rx, PanelSettingsChange::Root).expect("configured symlink replacement observed");
        assert!(watcher.refresh_watches());
        assert_silent(&mut rx);
        write_atomically(&second.join("global.toml"), "version = 1\n");
        wait_for(&mut rx, PanelSettingsChange::Global).expect("replacement target watched");
    }

}
