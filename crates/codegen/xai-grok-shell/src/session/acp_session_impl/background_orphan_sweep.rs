//! Pure ownership decisions for the leader-startup orphan sweep of task
//! process groups.
//!
//! `kill_on_drop` only fires while the spawning process lives; a SIGKILL'd (or
//! crashed) leader never drops its `Child` handles, so spawned task process
//! groups survive as OS orphans after every in-memory registry died with the
//! leader. The sweep consumes a persisted process-group registry (one row per
//! spawned task group, stamped with the spawning leader generation) plus
//! startup liveness facts, and decides per group whether the running leader may
//! reap it. Decision-only by contract: no process signals are sent, probed, or
//! simulated here — the caller owns liveness probing and any actual kill.

/// One persisted process-group registry row: a task process group spawned by a
/// specific leader generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredTaskProcessGroup {
    /// Registry key (the task id); carried into the decision log.
    pub task_id: String,
    /// Lead pid of the group, as recorded at spawn time.
    pub pid: u32,
    /// Process-group id to signal when this row is an orphan.
    pub process_group_id: u32,
    /// Session that spawned the task. Diagnostics only: the kill decision is
    /// owned by the leader generation, not the session.
    pub owner_session_id: Option<String>,
    /// Leader generation that spawned the group. `None` on rows persisted
    /// before generation stamping existed.
    pub leader_generation: Option<String>,
}

/// Startup facts the decision runs on. Gathered by the caller (pid liveness,
/// leader-generation liveness registry); never probed here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepFacts {
    /// Generation id of the leader performing the sweep.
    pub current_leader_generation: Option<String>,
    /// Leader generations known to still have a live leader process.
    pub live_leader_generations: std::collections::HashSet<String>,
    /// Pids confirmed alive at sweep time.
    pub alive_pids: std::collections::HashSet<u32>,
}

/// What the sweep decided about one registered group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SweepDecision {
    /// A live leader owns the group (this one, or another verified-live
    /// generation); the sweeper must not touch it.
    OwnedByLiveLeader { generation: String },
    /// The spawning leader generation is dead and the recorded pid is alive:
    /// a SIGKILL-orphaned group the sweeper should reap.
    ReapOrphanedGroup,
    /// The recorded pid is gone; there is nothing to signal and the registry
    /// row is stale.
    AlreadyGone,
    /// Ownership cannot be established (the row predates generation
    /// stamping). Never kill what cannot be attributed.
    UnknownOwner,
}

/// The sweep's outcome: which process groups to reap, plus a per-task decision
/// log for the sweep record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartupSweepPlan {
    /// Unique process-group ids to reap, ascending. Deterministic so the sweep
    /// log is reproducible across restarts.
    pub reap_process_group_ids: Vec<u32>,
    /// One decision per registry row, in registry order, keyed by task id.
    pub decisions: Vec<(String, SweepDecision)>,
}

/// Decide the fate of one registered group against the startup facts.
pub fn classify_registered_group(
    record: &RegisteredTaskProcessGroup,
    facts: &SweepFacts,
) -> SweepDecision {
    let Some(generation) = record.leader_generation.as_ref() else {
        return SweepDecision::UnknownOwner;
    };
    let owned_by_live_leader = facts.current_leader_generation.as_deref() == Some(generation.as_str())
        || facts.live_leader_generations.contains(generation);
    if owned_by_live_leader {
        SweepDecision::OwnedByLiveLeader {
            generation: generation.clone(),
        }
    } else if facts.alive_pids.contains(&record.pid) {
        SweepDecision::ReapOrphanedGroup
    } else {
        SweepDecision::AlreadyGone
    }
}

/// Classify every registry row and collect the unique orphaned process groups
/// to reap. Rows the current leader owns, rows owned by another live leader
/// generation, rows whose pid is already gone, and unattributable rows never
/// produce a reap.
pub fn decide_startup_sweep(
    records: &[RegisteredTaskProcessGroup],
    facts: &SweepFacts,
) -> StartupSweepPlan {
    let mut reap_process_group_ids = std::collections::BTreeSet::new();
    let mut decisions = Vec::with_capacity(records.len());
    for record in records {
        let decision = classify_registered_group(record, facts);
        if decision == SweepDecision::ReapOrphanedGroup {
            reap_process_group_ids.insert(record.process_group_id);
        }
        decisions.push((record.task_id.clone(), decision));
    }
    StartupSweepPlan {
        reap_process_group_ids: reap_process_group_ids.into_iter().collect(),
        decisions,
    }
}

#[cfg(test)]
#[path = "background_orphan_sweep_tests.rs"]
mod tests;
