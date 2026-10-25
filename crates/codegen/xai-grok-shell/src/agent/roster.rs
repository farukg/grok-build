//! Roster types for the multi-client FleetView dashboard.
//!
//! The roster is a list of dashboard-sized summaries of every session the leader hosts (resident actors).
//! It also carries recently-touched on-disk (`Dormant`) sessions.
//! Clients read it two ways:
//!
//!   - request/response `x.ai/sessions/list` returns `{ "sessions": [RosterEntry, …] }`
//!   - broadcast notification `x.ai/sessions/changed` carries `{ "upserted": [RosterEntry, …], "removed": ["sess-abc", …] }`
//!
//! The wire shape is intentionally small and current-state only; no event fold or materialized snapshot is required (the snapshot is deferred).

use serde::{Deserialize, Serialize};
use xai_grok_sampling_types::ReasoningEffort;

use crate::session::persistence::Summary;

/// Coarse activity of a session as rendered in the dashboard's status column. Mirrors the design's `SessionActivity` at dashboard granularity.
/// A full background-work breakdown (bg tasks / monitors / scheduler / subagents) lands with a richer `SessionActivity`. The dashboard only needs this coarse signal to pick a status glyph.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RosterActivity {
    /// A turn (user-originated or autonomous) is running.
    Working,
    /// Resident, no turn in flight.
    Idle,
    /// A permission / question / plan-approval is pending.
    NeedsInput,
    /// On disk, not resident.
    Dormant,
    /// Finished and resumable.
    Completed,
    /// Actor panicked / load failed.
    Dead,
}

/// Where the session lives.
/// Only `Local` is produced today; `Remote` is reserved for cross-machine roster aggregation.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RosterOrigin {
    Local,
    Remote { host: String },
}

/// One dashboard row.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RosterEntry {
    pub session_id: String,
    /// Generated/display title, if known. Clients fall back to cwd / id.
    #[serde(default)]
    pub title: Option<String>,
    pub cwd: String,
    pub is_worktree: bool,
    /// Non-`None` when the row is not a plain loadable session: `subagent` for subagent child
    /// sessions listed for the dashboard's subagent lane, or a leader-synthesized external kind
    /// (`cursor-worker`, …) for rows a local agent does not own. Plain local rows stay `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_kind: Option<String>,
    /// For `subagent` rows: the parent session's id, from the live child host or the child's
    /// persisted summary. Lets clients stamp `_meta.childOf` and the leader recover the child
    /// after losing its in-memory registry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    /// Per-session reasoning effort for `model_id`.
    /// Carried alongside the model so clients can render the session's effort in the roster without a separate `model_state` fetch.
    /// `None` means "use the model/global default" (or the session predates per-session effort persistence).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    pub yolo: bool,
    pub activity: RosterActivity,
    /// Ultra-short summary of the session's most recent turn, for the row's secondary line.
    /// From `summary.json`; absent until the first turn ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn_summary: Option<String>,
    /// `true` while a resident actor hosts the session (vs. read from disk).
    pub resident: bool,
    /// Best-effort last-change timestamp (unix millis). Used for sort order.
    pub last_change_unix_ms: i64,
    pub origin: RosterOrigin,
}

/// Response payload for `x.ai/sessions/list`.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct RosterListResponse {
    pub sessions: Vec<RosterEntry>,
}

/// Params payload for the `x.ai/sessions/changed` broadcast notification.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct RosterChanged {
    #[serde(default)]
    pub upserted: Vec<RosterEntry>,
    #[serde(default)]
    pub removed: Vec<String>,
}

/// JSON-RPC method names for the roster API.
pub const SESSIONS_LIST_METHOD: &str = "x.ai/sessions/list";
pub const SESSIONS_CHANGED_METHOD: &str = "x.ai/sessions/changed";

/// Merge live `resident` rows with on-disk `summaries` into the sorted roster. Pure, so it is unit-testable without disk or a live actor.
/// Resident rows own the live state but carry no title or last-active time, so each adopts those from its summary. A `Working` row keeps its "now" timestamp instead.
/// Summaries with no resident row become `Dormant`; keying by id dedups them. Headless and explicitly-hidden summaries are excluded, including resident rows whose persisted summary is headless.
/// Subagent children are kept as `session_kind: "subagent"` rows with their persisted parent link — the dashboard's subagent lane lists them while its default lane filters them out.
pub(crate) fn merge_roster(
    mut entries: Vec<RosterEntry>,
    summaries: Vec<Summary>,
) -> Vec<RosterEntry> {
    let excluded_ids: std::collections::HashSet<String> = summaries
        .iter()
        .filter(|summary| excluded_from_roster(summary))
        .map(|summary| summary.info.id.0.to_string())
        .collect();
    entries.retain(|entry| !excluded_ids.contains(&entry.session_id));
    let mut by_id: std::collections::HashMap<String, Summary> = summaries
        .into_iter()
        .filter(|summary| !excluded_ids.contains(summary.info.id.0.as_ref()))
        .map(|s| (s.info.id.0.to_string(), s))
        .collect();

    // Backfill resident rows; remove the summary so it isn't re-emitted below.
    for entry in &mut entries {
        let Some(summary) = by_id.remove(&entry.session_id) else {
            continue;
        };
        if let Some(title) = summary.display_title_opt() {
            entry.title = Some(title);
        }
        // Copied through even when `None`: disk is authoritative
        // A rewind-cleared summary.json also clears a resident row whose cache still holds the old value
        entry.last_turn_summary = summary.last_turn_summary.clone();
        if entry.activity != RosterActivity::Working {
            entry.last_change_unix_ms = summary.last_change_unix_ms();
        }
        if summary.is_subagent_kind() {
            entry.session_kind = Some(crate::session::persistence::SUBAGENT_SESSION_KIND.to_string());
            entry.parent_session_id = summary.parent_session_id.clone();
        }
    }

    // Remaining summaries have no resident row: emit them as dormant.
    entries.extend(by_id.into_values().map(|summary| {
        let subagent = summary.is_subagent_kind();
        RosterEntry {
            session_id: summary.info.id.0.to_string(),
            title: summary.display_title_opt(),
            cwd: summary.info.cwd.clone(),
            is_worktree: summary.session_kind.as_deref() == Some("worktree")
                || summary.source_workspace_dir.is_some(),
            session_kind: subagent
                .then(|| crate::session::persistence::SUBAGENT_SESSION_KIND.to_string()),
            parent_session_id: subagent
                .then(|| summary.parent_session_id.clone())
                .flatten(),
            model_id: Some(summary.current_model_id.0.to_string()),
            reasoning_effort: summary.reasoning_effort,
            yolo: false,
            activity: RosterActivity::Dormant,
            last_turn_summary: summary.last_turn_summary.clone(),
            resident: false,
            last_change_unix_ms: summary.last_change_unix_ms(),
            origin: RosterOrigin::Local,
        }
    }));

    // Most-recently-changed first.
    entries.sort_by(|a, b| b.last_change_unix_ms.cmp(&a.last_change_unix_ms));
    entries
}

/// Headless one-shots never list. A hidden summary is excluded unless it is hidden only by
/// subagent kind — subagent children are roster rows (for the subagent lane), not history noise.
fn excluded_from_roster(summary: &Summary) -> bool {
    !crate::session::storage::SessionListVisibility::Roster.admits(summary)
}

#[cfg(test)]
mod merge_roster_tests {
    use super::*;
    use crate::session::info::Info;
    use crate::session::persistence::default_model_id;
    use agent_client_protocol as acp;

    fn summary(id: &str, title: Option<&str>, last_active_ms: i64) -> Summary {
        let mut s = Summary::new(
            &Info {
                id: acp::SessionId::new(id),
                cwd: format!("/repo/{id}"),
            },
            default_model_id(),
        )
        .expect("summary");
        s.generated_title = title.map(String::from);
        s.last_active_at = chrono::DateTime::from_timestamp_millis(last_active_ms);
        s
    }

    fn resident(id: &str, activity: RosterActivity, last_change_unix_ms: i64) -> RosterEntry {
        RosterEntry {
            session_id: id.to_string(),
            title: None,
            cwd: format!("/live/{id}"),
            is_worktree: false,
            session_kind: None,
            parent_session_id: None,
            model_id: Some("grok-4".into()),
            reasoning_effort: None,
            yolo: false,
            activity,
            last_turn_summary: None,
            resident: true,
            last_change_unix_ms,
            origin: RosterOrigin::Local,
        }
    }

    #[test]
    fn idle_resident_adopts_persisted_title_and_last_active() {
        let now = 9_000;
        let out = merge_roster(
            vec![resident("a", RosterActivity::Idle, now)],
            vec![summary("a", Some("Fix the roster"), 1_234)],
        );
        assert_eq!(out.len(), 1, "resident must not be duplicated as dormant");
        let Some(row) = out.first() else {
            panic!("expected one roster row: {out:?}");
        };
        assert_eq!(row.title.as_deref(), Some("Fix the roster"));
        assert_eq!(row.last_change_unix_ms, 1_234, "idle adopts last-active");
        assert!(row.resident);
        assert_eq!(row.cwd, "/live/a", "live cwd is preserved");
        assert_eq!(row.activity, RosterActivity::Idle);
    }

    #[test]
    fn working_resident_keeps_now_but_adopts_title() {
        let now = 9_000;
        let out = merge_roster(
            vec![resident("a", RosterActivity::Working, now)],
            vec![summary("a", Some("Busy turn"), 1_234)],
        );
        let Some(row) = out.first() else {
            panic!("expected one roster row: {out:?}");
        };
        assert_eq!(row.title.as_deref(), Some("Busy turn"));
        assert_eq!(row.last_change_unix_ms, now, "Working stays 'now'");
    }

    #[test]
    fn new_resident_without_summary_stays_titleless_now() {
        let now = 9_000;
        let out = merge_roster(vec![resident("a", RosterActivity::Idle, now)], vec![]);
        let Some(row) = out.first() else {
            panic!("expected one roster row: {out:?}");
        };
        assert_eq!(row.title, None);
        assert_eq!(row.last_change_unix_ms, now);
    }

    #[test]
    fn blank_persisted_title_leaves_row_untitled() {
        let out = merge_roster(
            vec![resident("a", RosterActivity::Idle, 9_000)],
            vec![summary("a", Some("   "), 1_234)],
        );
        let Some(row) = out.first() else {
            panic!("expected one roster row: {out:?}");
        };
        assert_eq!(row.title, None, "blank title normalizes to None");
        assert_eq!(row.last_change_unix_ms, 1_234);
    }

    #[test]
    fn dormant_sessions_are_emitted_and_sorted_after_residents() {
        let out = merge_roster(
            vec![resident("live", RosterActivity::Idle, 5_000)],
            vec![
                summary("live", Some("Live one"), 4_000),
                summary("old", Some("Dormant one"), 1_000),
                summary("new", Some("Newer dormant"), 8_000),
            ],
        );
        let ids: Vec<&str> = out.iter().map(|e| e.session_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["new", "live", "old"],
            "sorted by last-change desc"
        );
        let dormant = out.iter().find(|e| e.session_id == "new").unwrap();
        assert_eq!(dormant.activity, RosterActivity::Dormant);
        assert!(!dormant.resident);
    }

    #[test]
    fn duplicate_summaries_are_deduped() {
        let out = merge_roster(
            vec![],
            vec![
                summary("dup", Some("First"), 1_000),
                summary("dup", Some("Second"), 2_000),
            ],
        );
        assert_eq!(out.len(), 1, "duplicate ids collapse to one row");
    }

    #[test]
    fn subagent_summaries_become_subagent_rows_with_parent_link() {
        let mut sub = summary("sub", Some("Child worker"), 5_000);
        sub.session_kind = Some("subagent".into());
        sub.parent_session_id = Some("parent-1".into());
        let out = merge_roster(vec![], vec![sub]);
        let Some(row) = out.first() else {
            panic!("expected one roster row: {out:?}");
        };
        assert_eq!(row.session_kind.as_deref(), Some("subagent"));
        assert_eq!(row.parent_session_id.as_deref(), Some("parent-1"));
        assert!(!row.resident, "disk-only child is dormant until reconciled");
    }

    #[test]
    fn resident_subagent_row_adopts_kind_and_parent_from_its_summary() {
        let mut sub = summary("sub", Some("Child worker"), 1_234);
        sub.session_kind = Some("subagent".into());
        sub.parent_session_id = Some("parent-1".into());
        let out = merge_roster(
            vec![resident("sub", RosterActivity::Idle, 9_000)],
            vec![sub],
        );
        assert_eq!(out.len(), 1, "resident child must not be duplicated as dormant");
        let Some(row) = out.first() else {
            panic!("expected one roster row: {out:?}");
        };
        assert_eq!(row.session_kind.as_deref(), Some("subagent"));
        assert_eq!(row.parent_session_id.as_deref(), Some("parent-1"));
        assert!(row.resident);
    }

    #[test]
    fn explicitly_hidden_summaries_stay_excluded() {
        let mut hidden = summary("hidden", Some("User-hidden"), 5_000);
        hidden.hidden = Some(true);
        let out = merge_roster(vec![], vec![hidden]);
        assert!(out.is_empty(), "explicitly-hidden summaries are dropped");
    }

    #[test]
    fn explicitly_hidden_children_exclude_dormant_and_resident_rows() {
        for resident_rows in [vec![], vec![resident("child", RosterActivity::Working, 9_000)]] {
            let mut child = summary("child", Some("Hidden child"), 5_000);
            child.session_kind = Some("subagent".into());
            child.hidden = Some(true);
            assert!(merge_roster(resident_rows, vec![child]).is_empty());
        }
    }

    #[test]
    fn headless_summaries_exclude_dormant_and_resident_rows() {
        let mut headless = summary("oneshot", Some("grok -p run"), 5_000);
        headless.session_kind = Some("headless".into());
        let out = merge_roster(
            vec![resident("oneshot", RosterActivity::Working, 9_000)],
            vec![headless],
        );
        assert!(out.is_empty(), "headless one-shots make no roster rows");
    }

    #[test]
    fn dormant_row_carries_persisted_reasoning_effort() {
        let mut s = summary("dorm", Some("Dormant"), 1_000);
        s.reasoning_effort = Some(ReasoningEffort::Xhigh);
        let out = merge_roster(vec![], vec![s]);
        let Some(row) = out.first() else {
            panic!("expected one roster row: {out:?}");
        };
        assert_eq!(row.reasoning_effort, Some(ReasoningEffort::Xhigh));
    }

    #[test]
    fn resident_effort_is_taken_from_the_live_row_not_the_summary() {
        // The live handle is authoritative for a resident session, so the resident row's effort must survive the summary backfill
        let mut live = resident("a", RosterActivity::Idle, 9_000);
        live.reasoning_effort = Some(ReasoningEffort::High);
        let mut s = summary("a", Some("Title"), 1_234);
        s.reasoning_effort = Some(ReasoningEffort::Low);
        let out = merge_roster(vec![live], vec![s]);
        assert_eq!(out.len(), 1);
        let Some(row) = out.first() else {
            panic!("expected one roster row: {out:?}");
        };
        assert_eq!(row.reasoning_effort, Some(ReasoningEffort::High));
    }

    #[test]
    fn last_turn_summary_adopted_from_summary_for_resident_and_dormant() {
        let mut with = summary("a", None, 1);
        with.last_turn_summary = Some("Fixed the roster merge".into());
        let mut dormant = summary("b", None, 2);
        dormant.last_turn_summary = Some("Explained the roster".into());

        let out = merge_roster(
            vec![resident("a", RosterActivity::Idle, 9_000)],
            vec![with, dormant, summary("c", None, 3)],
        );

        let by_id = |id: &str| out.iter().find(|e| e.session_id == id).unwrap();
        assert_eq!(
            by_id("a").last_turn_summary.as_deref(),
            Some("Fixed the roster merge")
        );
        assert_eq!(
            by_id("b").last_turn_summary.as_deref(),
            Some("Explained the roster")
        );
        assert_eq!(by_id("c").last_turn_summary, None);
    }

    /// Disk is authoritative: a rewind-cleared `summary.json` clears a resident row whose delta cache still carries the old value.
    /// The poll is self-healing.
    #[test]
    fn last_turn_summary_cleared_disk_clears_resident_row() {
        let mut entry = resident("a", RosterActivity::Idle, 9_000);
        entry.last_turn_summary = Some("Stale cached".into());
        let out = merge_roster(vec![entry], vec![summary("a", None, 1)]);
        let Some(row) = out.first() else {
            panic!("expected one roster row: {out:?}");
        };
        assert_eq!(row.last_turn_summary, None);
    }

    #[test]
    fn reasoning_effort_serializes_as_camel_case_and_skips_when_none() {
        let with_effort = RosterEntry {
            reasoning_effort: Some(ReasoningEffort::Xhigh),
            ..resident("a", RosterActivity::Idle, 1)
        };
        let json = serde_json::to_string(&with_effort).unwrap();
        assert!(
            json.contains("\"reasoningEffort\":\"xhigh\""),
            "effort must be camelCase and snake_case-valued: {json}"
        );

        let without = resident("b", RosterActivity::Idle, 1);
        let json = serde_json::to_string(&without).unwrap();
        assert!(
            !json.contains("reasoningEffort"),
            "a None effort must not be serialized: {json}"
        );
    }

    #[test]
    fn parent_session_id_serializes_as_camel_case_and_skips_when_none() {
        let mut child = resident("child", RosterActivity::Idle, 1);
        child.session_kind = Some("subagent".into());
        child.parent_session_id = Some("parent-1".into());
        let json = serde_json::to_string(&child).unwrap();
        assert!(
            json.contains("\"parentSessionId\":\"parent-1\""),
            "parent link must be camelCase on the wire: {json}"
        );

        let root = resident("root", RosterActivity::Idle, 1);
        let json = serde_json::to_string(&root).unwrap();
        assert!(
            !json.contains("parentSessionId"),
            "a top-level row must not carry a parent link: {json}"
        );
    }
}
