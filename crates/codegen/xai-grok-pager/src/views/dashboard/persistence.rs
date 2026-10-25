use super::child_layout::ChildLayoutUpdate;
use super::state::PersistedDashboard;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, mpsc};
use tokio::sync::oneshot;

pub(crate) enum DashboardWrite {
    Full(PersistedDashboard),
    Child(ChildLayoutUpdate),
}

impl DashboardWrite {
    fn write_to_path(self, path: &Path) -> io::Result<()> {
        match self {
            Self::Full(saved) => super::state::write_persisted_to_path(path, &saved),
            Self::Child(update) => update.write_to_path(path),
        }
    }
}

struct PendingWrite {
    path: PathBuf,
    write: DashboardWrite,
    completed: oneshot::Sender<io::Result<()>>,
}

#[derive(Clone)]
struct DashboardWriter {
    sender: mpsc::Sender<PendingWrite>,
}

impl DashboardWriter {
    fn start(
        mut write: impl FnMut(&Path, DashboardWrite) -> io::Result<()> + Send + 'static,
    ) -> io::Result<Self> {
        let (sender, receiver) = mpsc::channel::<PendingWrite>();
        std::thread::Builder::new()
            .name("dashboard-config-writer".into())
            .spawn(move || {
                for pending in receiver {
                    let result = write(&pending.path, pending.write);
                    let _ = pending.completed.send(result);
                }
            })?;
        Ok(Self { sender })
    }

    fn enqueue(
        &self,
        path: PathBuf,
        write: DashboardWrite,
    ) -> io::Result<oneshot::Receiver<io::Result<()>>> {
        let (completed, receipt) = oneshot::channel();
        self.sender
            .send(PendingWrite {
                path,
                write,
                completed,
            })
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "dashboard writer stopped"))?;
        Ok(receipt)
    }
}

static WRITER: OnceLock<Result<DashboardWriter, String>> = OnceLock::new();

pub(crate) fn enqueue(write: DashboardWrite) -> io::Result<oneshot::Receiver<io::Result<()>>> {
    #[cfg(test)]
    if let Some((writer, path)) = TEST_WRITER.with(|slot| slot.borrow().clone()) {
        return writer.enqueue(path, write);
    }
    let path = super::state::config_path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no grok home"))?;
    let writer = WRITER.get_or_init(|| {
        DashboardWriter::start(|path, write| write.write_to_path(path))
            .map_err(|error| error.to_string())
    });
    match writer {
        Ok(writer) => writer.enqueue(path, write),
        Err(error) => Err(io::Error::other(error.clone())),
    }
}

#[cfg(test)]
thread_local! {
    static TEST_WRITER: std::cell::RefCell<Option<(DashboardWriter, PathBuf)>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_test_writer<R>(
    path: PathBuf,
    mut after_write: impl FnMut(&Path) + Send + 'static,
    run: impl FnOnce() -> R,
) -> io::Result<R> {
    struct Restore(Option<(DashboardWriter, PathBuf)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_WRITER.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let writer = DashboardWriter::start(move |path, write| {
        write.write_to_path(path)?;
        after_write(path);
        Ok(())
    })?;
    let _restore = Restore(TEST_WRITER.with(|slot| slot.replace(Some((writer, path)))));
    Ok(run())
}

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
