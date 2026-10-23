//! Full-content syntax highlight for Read previews, off the draw thread.
//!
//! Mirrors [`super::edit_highlight_worker`]: one `std::thread` and mpsc, coalesced by `entry_id` (latest job wins), polled each tick via `try_recv`.
//! A Read block paints its head and tail rows on the UI thread; this worker styles the whole content once, so expanding a large file reads from the cache.
//!
//! # Cost model
//!
//! - **First paint**: head and tail rows only (see `ReadToolCallBlock::render_content_lines`).
//! - **Upgrade**: one highlight pass over the whole content, off-thread; the result is cached in the block.
//! - **Over cap**: stay on the partial paint; no unbounded work.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use indexmap::IndexMap;
use ratatui::style::Style;

use crate::app::agent_view::AgentView;
use crate::appearance::TerminalColoring;
use crate::scrollback::block::RenderBlock;
use crate::scrollback::blocks::tool::{
    CachedReadRows, EDIT_HL_MAX_BYTES, PARTIAL_HIGHLIGHT_MIN_ROWS, ReadHighlightPhase,
    ReadToolCallBlock, ToolCallBlock, file_text_within_hl_caps,
};
use crate::scrollback::entry::EntryId;

/// Tracing target. Filter with `RUST_LOG=read_hl=debug`.
pub const READ_HL_TRACING_TARGET: &str = "read_hl";

/// A unit of work for the read-highlight worker.
#[derive(Debug, Clone)]
pub struct ReadHlJob {
    pub job_id: u64,
    pub entry_id: EntryId,
    /// Display path used for syntax selection (extension).
    pub path: String,
    pub content: Arc<str>,
    pub coloring: TerminalColoring,
    pub base: Style,
}

/// Outcome of one highlight attempt.
#[derive(Debug, Clone)]
pub enum ReadHlOutcome {
    Ready(CachedReadRows),
    /// Over the caps: stay on the partial paint.
    Failed,
}

/// Result returned over the worker channel.
#[derive(Debug, Clone)]
pub struct ReadHlResult {
    pub job_id: u64,
    pub entry_id: EntryId,
    /// Display path at submit time (stale check).
    pub path: String,
    pub outcome: ReadHlOutcome,
}

/// Per-[`AgentView`] read-HL runtime: channels and in-flight bookkeeping.
pub struct ReadHlRuntime {
    tx: Sender<ReadHlJob>,
    rx: Receiver<ReadHlResult>,
    /// Outstanding `(job_id, entry_id)` for `needs_tick`.
    pending: Vec<(u64, EntryId)>,
    next_job_id: u64,
}

impl ReadHlRuntime {
    fn new() -> Self {
        let (tx, rx) = spawn_worker();
        Self {
            tx,
            rx,
            pending: Vec::new(),
            next_job_id: 1,
        }
    }

    fn alloc_job_id(&mut self) -> u64 {
        let id = self.next_job_id;
        self.next_job_id = self.next_job_id.wrapping_add(1);
        id
    }
}

/// Spawn the single read-highlight worker thread and return its channels.
pub fn spawn_worker() -> (Sender<ReadHlJob>, Receiver<ReadHlResult>) {
    let (job_tx, job_rx) = std::sync::mpsc::channel::<ReadHlJob>();
    let (result_tx, result_rx) = std::sync::mpsc::channel::<ReadHlResult>();

    std::thread::Builder::new()
        .name("read-hl".to_string())
        .spawn(move || {
            while let Ok(first) = job_rx.recv() {
                let pending = drain_coalesced(first, &job_rx);
                for (_, job) in pending {
                    let outcome = run_job(&job);
                    let result = ReadHlResult {
                        job_id: job.job_id,
                        entry_id: job.entry_id,
                        path: job.path,
                        outcome,
                    };
                    if result_tx.send(result).is_err() {
                        return;
                    }
                }
            }
        })
        .expect("spawn read-hl thread");

    (job_tx, result_rx)
}

/// Coalesce `first` plus queued jobs by `entry_id` (latest wins). FIFO across keys.
fn drain_coalesced(first: ReadHlJob, rx: &Receiver<ReadHlJob>) -> IndexMap<EntryId, ReadHlJob> {
    let mut pending: IndexMap<EntryId, ReadHlJob> = IndexMap::new();
    pending.insert(first.entry_id, first);
    while let Ok(job) = rx.try_recv() {
        pending.insert(job.entry_id, job);
    }
    pending
}

/// Style the whole content under the caps.
fn run_job(job: &ReadHlJob) -> ReadHlOutcome {
    if job.content.len() as u64 > EDIT_HL_MAX_BYTES || !file_text_within_hl_caps(&job.content) {
        tracing::debug!(target: READ_HL_TRACING_TARGET, path = %job.path, "read HL skipped (cap)");
        return ReadHlOutcome::Failed;
    }
    // Read the theme beside the highlight walk so the result is labeled with the kind its foregrounds were baked under
    let theme = crate::theme::cache::current_kind();
    let rows = crate::render::content_kind::styled_rows(
        crate::syntax::get_syntect(),
        std::path::Path::new(&job.path),
        &job.content,
        job.coloring,
        job.base,
    );
    ReadHlOutcome::Ready(CachedReadRows {
        rows: Arc::new(rows),
        theme,
        coloring: job.coloring,
    })
}

impl AgentView {
    /// Keep ticking while a read-HL job is outstanding (mpsc has no waker).
    pub fn read_hl_needs_tick(&self) -> bool {
        self.read_hl
            .as_ref()
            .is_some_and(|rt| !rt.pending.is_empty())
    }

    /// Poll worker results and attach the cached rows. Returns true when a redraw is needed.
    pub fn read_hl_tick(&mut self) -> bool {
        self.poll_read_hl_results()
    }

    fn ensure_read_hl_runtime(&mut self) -> &mut ReadHlRuntime {
        if self.read_hl.is_none() {
            self.read_hl = Some(ReadHlRuntime::new());
        }
        self.read_hl.as_mut().expect("just created")
    }

    /// Submit full-content HL for every large Read entry that has neither a cache nor a job in flight.
    /// Called from the tick, so it covers live completions and replayed history alike without a tracker queue.
    /// Reads that need no job settle to `Skipped`, so later ticks do not measure them again.
    pub fn submit_read_highlights(&mut self) {
        let coloring = crate::appearance::terminal_coloring::current();
        let mut wanted: Vec<(EntryId, ReadHlJob)> = Vec::new();
        let mut settled: Vec<EntryId> = Vec::new();
        for (entry_id, entry) in self.scrollback.iter_entries() {
            let RenderBlock::ToolCall(ToolCallBlock::Read(read)) = &entry.block else {
                continue;
            };
            match read_job(read, entry_id, coloring) {
                Some(job) => wanted.push((entry_id, job)),
                None if read.content.is_some()
                    && matches!(read.highlight, ReadHighlightPhase::Unrequested) =>
                {
                    settled.push(entry_id);
                }
                None => {}
            }
        }
        for entry_id in settled {
            if let Some(entry) = self.scrollback.get_by_id_mut(entry_id)
                && let RenderBlock::ToolCall(ToolCallBlock::Read(read)) = &mut entry.block
            {
                read.highlight = ReadHighlightPhase::Skipped;
            }
        }
        for (entry_id, mut job) in wanted {
            let rt = self.ensure_read_hl_runtime();
            if rt.pending.iter().any(|(_, id)| *id == entry_id) {
                continue;
            }
            job.job_id = rt.alloc_job_id();
            let job_id = job.job_id;
            if rt.tx.send(job).is_ok() {
                rt.pending.push((job_id, entry_id));
                if let Some(entry) = self.scrollback.get_by_id_mut(entry_id)
                    && let RenderBlock::ToolCall(ToolCallBlock::Read(read)) = &mut entry.block
                {
                    read.highlight = ReadHighlightPhase::Pending { job_id };
                }
            } else {
                self.abandon_read_hl_worker("job send failed");
                return;
            }
        }
    }

    fn poll_read_hl_results(&mut self) -> bool {
        use std::sync::mpsc::TryRecvError;

        let mut results = Vec::new();
        let mut disconnected = false;
        if let Some(rt) = self.read_hl.as_ref() {
            loop {
                match rt.rx.try_recv() {
                    Ok(r) => results.push(r),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }

        let mut redraw = false;
        for result in results {
            if let Some(rt) = self.read_hl.as_mut() {
                rt.pending.retain(|(id, _)| *id != result.job_id);
            }
            let Some(entry) = self.scrollback.get_by_id_mut(result.entry_id) else {
                continue;
            };
            let RenderBlock::ToolCall(ToolCallBlock::Read(read)) = &mut entry.block else {
                continue;
            };
            if read.path != result.path {
                continue;
            }
            match &read.highlight {
                ReadHighlightPhase::Pending { job_id } if *job_id == result.job_id => {}
                _ => continue,
            }
            match result.outcome {
                ReadHlOutcome::Ready(rows) => {
                    read.highlight = ReadHighlightPhase::Cached(rows);
                    entry.invalidate_cache();
                    redraw = true;
                }
                ReadHlOutcome::Failed => read.highlight = ReadHighlightPhase::Skipped,
            }
        }
        if disconnected {
            self.abandon_read_hl_worker("result channel disconnected");
        }
        redraw
    }

    /// Forget a dead worker: drop its runtime (a later submit respawns a fresh one) and revert in-flight entries to the partial paint.
    fn abandon_read_hl_worker(&mut self, context: &'static str) {
        let Some(rt) = self.read_hl.take() else {
            return;
        };
        tracing::warn!(
            target: READ_HL_TRACING_TARGET,
            context,
            pending = rt.pending.len(),
            "read HL worker gone; reverting pending jobs to the partial paint"
        );
        for (_, entry_id) in rt.pending {
            if let Some(entry) = self.scrollback.get_by_id_mut(entry_id)
                && let RenderBlock::ToolCall(ToolCallBlock::Read(read)) = &mut entry.block
                && matches!(read.highlight, ReadHighlightPhase::Pending { .. })
            {
                read.highlight = ReadHighlightPhase::Unrequested;
            }
        }
    }
}

/// The job a Read block still needs, if any: large content with no cache, no job in flight and no earlier skip.
/// A cache baked under another theme or coloring is requested again.
/// The row count stops at the threshold, so a large file costs one short scan and a small one a full scan of a small string.
fn read_job(
    read: &ReadToolCallBlock,
    entry_id: EntryId,
    coloring: TerminalColoring,
) -> Option<ReadHlJob> {
    let content = read.content.as_deref()?;
    let needed = match &read.highlight {
        ReadHighlightPhase::Unrequested => {
            content.lines().take(PARTIAL_HIGHLIGHT_MIN_ROWS).count() >= PARTIAL_HIGHLIGHT_MIN_ROWS
        }
        ReadHighlightPhase::Pending { .. } | ReadHighlightPhase::Skipped => false,
        ReadHighlightPhase::Cached(cached) => {
            cached.theme != crate::theme::cache::current_kind() || cached.coloring != coloring
        }
    };
    needed.then(|| ReadHlJob {
        job_id: 0,
        entry_id,
        path: read.path.clone(),
        content: Arc::from(content),
        coloring,
        base: crate::theme::Theme::current().primary(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrollback::blocks::tool::ReadToolCallBlock;
    use std::path::PathBuf;

    fn big_read(path: &str) -> ReadToolCallBlock {
        let content = (0..PARTIAL_HIGHLIGHT_MIN_ROWS + 10)
            .map(|i| format!("let value_{i} = {i};"))
            .collect::<Vec<_>>()
            .join("\n");
        ReadToolCallBlock::new(path).with_content(content, PARTIAL_HIGHLIGHT_MIN_ROWS + 10)
    }

    #[test]
    fn small_reads_settle_so_later_ticks_skip_them() {
        let mut agent = crate::app::agent_view::test_agent_view(
            Some("read-hl-small"),
            PathBuf::from("/tmp/read-hl-test"),
        );
        let entry_id = agent
            .scrollback
            .push_block(RenderBlock::ToolCall(ToolCallBlock::Read(
                ReadToolCallBlock::new("a.rs").with_content("fn a() {}\n".into(), 1),
            )));
        agent.submit_read_highlights();
        assert!(!agent.read_hl_needs_tick(), "no job for a small read");
        let entry = agent.scrollback.get_by_id(entry_id).expect("entry");
        let RenderBlock::ToolCall(ToolCallBlock::Read(read)) = &entry.block else {
            panic!("read block missing");
        };
        assert!(matches!(read.highlight, ReadHighlightPhase::Skipped));
    }

    #[test]
    fn small_reads_need_no_job() {
        let read = ReadToolCallBlock::new("a.rs").with_content("fn a() {}\n".into(), 1);
        assert!(read_job(&read, EntryId::new(1), TerminalColoring::Terminal).is_none());
    }

    #[test]
    fn threshold_read_without_final_newline_gets_a_job() {
        let content = vec!["let value = 1;"; PARTIAL_HIGHLIGHT_MIN_ROWS].join("\n");
        let read =
            ReadToolCallBlock::new("big.rs").with_content(content, PARTIAL_HIGHLIGHT_MIN_ROWS);
        let job = read_job(&read, EntryId::new(1), TerminalColoring::Terminal)
            .expect("400 rows require off-thread highlighting");
        let ReadHlOutcome::Ready(cached) = run_job(&job) else {
            panic!("read HL should be ready");
        };
        assert_eq!(cached.rows.len(), PARTIAL_HIGHLIGHT_MIN_ROWS);
    }

    #[test]
    fn large_unrequested_reads_get_a_job_and_pending_ones_do_not() {
        let mut read = big_read("big.rs");
        assert!(read_job(&read, EntryId::new(1), TerminalColoring::Terminal).is_some());
        read.highlight = ReadHighlightPhase::Pending { job_id: 3 };
        assert!(read_job(&read, EntryId::new(1), TerminalColoring::Terminal).is_none());
        read.highlight = ReadHighlightPhase::Skipped;
        assert!(read_job(&read, EntryId::new(1), TerminalColoring::Terminal).is_none());
    }

    #[test]
    fn cache_baked_under_another_coloring_is_requested_again() {
        let mut read = big_read("big.rs");
        read.highlight = ReadHighlightPhase::Cached(CachedReadRows {
            rows: Arc::new(Vec::new()),
            theme: crate::theme::cache::current_kind(),
            coloring: TerminalColoring::Terminal,
        });
        assert!(read_job(&read, EntryId::new(1), TerminalColoring::Terminal).is_none());
        assert!(read_job(&read, EntryId::new(1), TerminalColoring::Both).is_some());
    }

    #[test]
    fn worker_styles_every_row_of_a_large_read() {
        let read = big_read("big.rs");
        let job = read_job(&read, EntryId::new(1), TerminalColoring::Terminal).expect("job");
        let ReadHlOutcome::Ready(rows) = run_job(&job) else {
            panic!("read HL should be ready");
        };
        assert_eq!(rows.rows.len(), PARTIAL_HIGHLIGHT_MIN_ROWS + 10);
    }

    #[test]
    fn submit_marks_pending_and_the_tick_applies_the_cache() {
        let mut agent = crate::app::agent_view::test_agent_view(
            Some("read-hl-submit"),
            PathBuf::from("/tmp/read-hl-test"),
        );
        let entry_id = agent
            .scrollback
            .push_block(RenderBlock::ToolCall(ToolCallBlock::Read(big_read(
                "big.rs",
            ))));

        let (job_tx, job_rx) = std::sync::mpsc::channel();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        agent.read_hl = Some(ReadHlRuntime {
            tx: job_tx,
            rx: result_rx,
            pending: Vec::new(),
            next_job_id: 1,
        });
        agent.submit_read_highlights();
        assert!(agent.read_hl_needs_tick());
        let job = job_rx.try_recv().expect("submitted job");
        let outcome = run_job(&job);
        result_tx
            .send(ReadHlResult {
                job_id: job.job_id,
                entry_id: job.entry_id,
                path: job.path,
                outcome,
            })
            .expect("worker result");
        assert!(agent.read_hl_tick(), "cache application needs a redraw");
        assert!(!agent.read_hl_needs_tick());
        let entry = agent.scrollback.get_by_id(entry_id).expect("entry");
        let RenderBlock::ToolCall(ToolCallBlock::Read(read)) = &entry.block else {
            panic!("read block missing");
        };
        assert!(matches!(read.highlight, ReadHighlightPhase::Cached(_)));
    }
}
