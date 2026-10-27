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

use notify::RecursiveMode;
use notify_debouncer_mini::{DebounceEventResult, Debouncer};
use tokio::sync::mpsc;

use super::watcher::{
    AccessFilteredWatcher, attach_new_refresh_dirs, new_filtered_debouncer, paths_equal,
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

/// Watches the panel-settings root for writes from any process sharing it.
///
/// Drop stops watching. Events arrive on an unbounded channel; a slow consumer
/// cannot lose the last change, only its delivery latency.
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
    ) -> Option<(Self, mpsc::UnboundedReceiver<PanelSettingsChange>)> {
        let root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let sessions = root.join(SESSIONS_DIR);
        let (tx, rx) = mpsc::unbounded_channel();
        let root_for_events = root.clone();
        let sessions_for_events = sessions.clone();
        let mut debouncer = new_filtered_debouncer(
            debounce.unwrap_or(DEBOUNCE),
            move |res: DebounceEventResult| {
                let Ok(events) = res else { return };
                let mut changes: Vec<PanelSettingsChange> = Vec::new();
                for event in events {
                    if let Some(change) =
                        classify(&event.path, &root_for_events, &sessions_for_events)
                        && !changes.contains(&change)
                    {
                        changes.push(change);
                    }
                }
                for change in changes {
                    let _ = tx.send(change);
                }
            },
        )
        .map_err(|error| tracing::warn!(%error, "failed to create panel settings watcher"))
        .ok()?;

        let mut refresh_dirs = vec![
            (root.clone(), RecursiveMode::NonRecursive),
            (sessions, RecursiveMode::NonRecursive),
        ];
        if let Some(parent) = root
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            refresh_dirs.push((parent.to_path_buf(), RecursiveMode::NonRecursive));
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

    /// Attaches the watches for directories that exist now but did not at
    /// [`Self::start`]. Call after a [`PanelSettingsChange::Root`].
    pub fn refresh_watches(&mut self) -> bool {
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

    fn drain(rx: &mut mpsc::UnboundedReceiver<PanelSettingsChange>) -> Vec<PanelSettingsChange> {
        let mut seen = Vec::new();
        while let Ok(change) = rx.try_recv() {
            seen.push(change);
        }
        seen
    }

    /// Polls like the config-watcher tests: no tokio runtime, one try_recv per tick.
    fn wait_for(
        rx: &mut mpsc::UnboundedReceiver<PanelSettingsChange>,
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

    fn assert_silent(rx: &mut mpsc::UnboundedReceiver<PanelSettingsChange>) {
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
}
