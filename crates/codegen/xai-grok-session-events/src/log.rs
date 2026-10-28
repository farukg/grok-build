use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use chrono::Utc;

use crate::decode::EventEntry;
use crate::types::Event;

const EVENTS_FILE: &str = "events.jsonl";

/// Writes events to `events.jsonl`.
/// Clones share one file, and the writer is `Send + Sync` so background tasks can hold one.
#[derive(Clone)]
pub struct EventWriter {
    inner: Arc<EventWriterInner>,
}

pub enum ToolLifecycleEvent<'a> {
    Cancelled {
        tool_call_id: &'a str,
        owner: &'a xai_tool_protocol::tool_execution::ToolCallOwnership,
    },
    Execution {
        tool_call_id: &'a str,
        owner: &'a xai_tool_protocol::tool_execution::ToolCallOwnership,
        timing: &'a xai_tool_protocol::tool_execution::ToolExecutionTiming,
    },
}

type ToolLifecycleCallback = dyn Fn(ToolLifecycleEvent<'_>) + Send + Sync;

struct EventWriterInner {
    file: Mutex<Option<File>>,
    error_logged: AtomicBool,
    tool_lifecycle: OnceLock<Arc<ToolLifecycleCallback>>,
}

impl EventWriter {
    pub fn open(session_dir: &Path) -> Self {
        let path = session_dir.join(EVENTS_FILE);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| {
                tracing::warn!(path = %path.display(), error = %e, "failed to open {EVENTS_FILE}");
                e
            })
            .ok();
        Self {
            inner: Arc::new(EventWriterInner {
                file: Mutex::new(file),
                tool_lifecycle: OnceLock::new(),
                error_logged: AtomicBool::new(false),
            }),
        }
    }

    pub fn noop() -> Self {
        Self {
            inner: Arc::new(EventWriterInner {
                file: Mutex::new(None),
                tool_lifecycle: OnceLock::new(),
                error_logged: AtomicBool::new(true), // True from the start, so this writer never warns
            }),
        }
    }

    pub fn on_tool_lifecycle(&self, callback: impl FnOnce() -> Arc<ToolLifecycleCallback>) {
        self.inner.tool_lifecycle.get_or_init(callback);
    }

    pub fn cancel_tool(
        &self,
        tool_call_id: &str,
        owner: &xai_tool_protocol::tool_execution::ToolCallOwnership,
    ) {
        if let Some(callback) = self.inner.tool_lifecycle.get() {
            callback(ToolLifecycleEvent::Cancelled {
                tool_call_id,
                owner,
            });
        }
    }

    pub fn emit(&self, event: Event) {
        if let Event::ToolExecution {
            tool_call_id,
            owner,
            timing,
        } = &event
            && let Some(callback) = self.inner.tool_lifecycle.get()
        {
            callback(ToolLifecycleEvent::Execution {
                tool_call_id,
                owner,
                timing,
            });
        }
        let entry = EventEntry {
            ts: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            event,
        };
        let Ok(mut line) = serde_json::to_vec(&entry) else {
            return;
        };
        line.push(b'\n');

        let Ok(mut guard) = self.inner.file.lock() else {
            return;
        };
        if let Some(ref mut f) = *guard
            && let Err(e) = f.write_all(&line)
            && !self.inner.error_logged.swap(true, Ordering::Relaxed)
        {
            tracing::warn!(error = %e, "{EVENTS_FILE} write failed");
        }
    }
}

impl std::fmt::Debug for EventWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventWriter").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        EVENT_SCHEMA_VERSION, Event, SessionRelationship, ToolOutcome, TurnOutcomeLabel,
    };

    fn _assert_event_writer_is_send_sync_clone()
    where
        EventWriter: Send + Sync + Clone,
    {
    }

    #[test]
    fn test_emit_writes_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let writer = EventWriter::open(dir.path());

        writer.emit(Event::TurnStarted {
            session_id: "test-session".into(),
            turn_number: 1,
            model_id: "grok-3".into(),
            yolo_mode: false,
            conversation_message_count: 0,
            session_relationship: SessionRelationship::Primary,
            schema_version: EVENT_SCHEMA_VERSION.into(),
            redirect_kind: None,
        });
        writer.emit(Event::FirstToken);
        writer.emit(Event::ToolCompleted {
            tool_name: "bash".into(),
            duration_ms: 1500,
            outcome: ToolOutcome::Success,
            tool_call_id: "call_xyz".into(),
            source: crate::types::ToolCompletedSource::Shell,
            rewriting_hook: None,
        });
        writer.emit(Event::TurnEnded {
            outcome: TurnOutcomeLabel::Completed,
            cancellation_category: None,
            cancellation_context: None,
        });

        let text = std::fs::read_to_string(dir.path().join("events.jsonl")).unwrap();
        let lines: Vec<&str> = text.trim().split('\n').collect();
        assert_eq!(lines.len(), 4);

        let [l0, l1, l2, l3] = lines.as_slice() else {
            panic!("expected four event lines: {lines:?}");
        };
        let first: serde_json::Value = serde_json::from_str(l0).unwrap();
        assert_eq!(first.get("type"), Some(&serde_json::json!("turn_started")));
        assert_eq!(
            first.get("session_id"),
            Some(&serde_json::json!("test-session"))
        );
        assert!(first.get("ts").and_then(|v| v.as_str()).is_some());

        let second: serde_json::Value = serde_json::from_str(l1).unwrap();
        assert_eq!(second.get("type"), Some(&serde_json::json!("first_token")));

        let third: serde_json::Value = serde_json::from_str(l2).unwrap();
        assert_eq!(
            third.get("type"),
            Some(&serde_json::json!("tool_completed"))
        );
        assert_eq!(third.get("tool_name"), Some(&serde_json::json!("bash")));
        assert_eq!(third.get("duration_ms"), Some(&serde_json::json!(1500)));
        assert_eq!(
            third.get("tool_call_id"),
            Some(&serde_json::json!("call_xyz"))
        );
        assert!(
            third.get("source").is_none(),
            "shell ToolCompleted must omit source"
        );

        let fourth: serde_json::Value = serde_json::from_str(l3).unwrap();
        assert_eq!(fourth.get("type"), Some(&serde_json::json!("turn_ended")));
        assert_eq!(fourth.get("outcome"), Some(&serde_json::json!("completed")));
        assert!(fourth.get("cancellation_category").is_none());

        let entries: Vec<EventEntry> = lines
            .iter()
            .map(|line| crate::decode::decode_event_entry(line).unwrap())
            .collect();
        for entry in &entries {
            chrono::DateTime::parse_from_rfc3339(&entry.ts).unwrap();
        }
        assert!(matches!(
            &entries[0].event,
            Event::TurnStarted { session_id, turn_number: 1, schema_version, .. }
                if session_id == "test-session" && schema_version == EVENT_SCHEMA_VERSION
        ));
        assert!(matches!(&entries[1].event, Event::FirstToken));
        assert!(matches!(
            &entries[2].event,
            Event::ToolCompleted {
                tool_name,
                duration_ms: 1500,
                outcome: ToolOutcome::Success,
                tool_call_id,
                source: crate::types::ToolCompletedSource::Shell,
                rewriting_hook: None,
            } if tool_name == "bash" && tool_call_id == "call_xyz"
        ));
        assert!(matches!(
            &entries[3].event,
            Event::TurnEnded {
                outcome: TurnOutcomeLabel::Completed,
                cancellation_category: None,
                cancellation_context: None,
            }
        ));
    }

    #[test]
    fn cloned_writer_shares_file() {
        let dir = tempfile::tempdir().unwrap();
        let w1 = EventWriter::open(dir.path());
        let w2 = w1.clone();

        w1.emit(Event::FirstToken);
        w2.emit(Event::FirstToken);

        let text = std::fs::read_to_string(dir.path().join("events.jsonl")).unwrap();
        let lines: Vec<&str> = text.trim().split('\n').collect();
        assert_eq!(lines.len(), 2, "both writes should go to the same file");
    }
}

#[cfg(test)]
#[path = "tool_execution_tests.rs"]
mod tool_execution_tests;
