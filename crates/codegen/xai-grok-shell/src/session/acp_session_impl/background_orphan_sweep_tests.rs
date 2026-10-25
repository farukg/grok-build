//! Pure-decision tests for the leader-startup orphan sweep: ownership
//! classification and plan collection only, never real process operations.

use super::*;

fn record(task_id: &str, pid: u32, pgid: u32, generation: Option<&str>) -> RegisteredTaskProcessGroup {
    RegisteredTaskProcessGroup {
        task_id: task_id.to_owned(),
        pid,
        process_group_id: pgid,
        owner_session_id: Some("sess-1".to_owned()),
        leader_generation: generation.map(str::to_owned),
    }
}

fn facts(current: Option<&str>, live: &[&str], alive: &[u32]) -> SweepFacts {
    SweepFacts {
        current_leader_generation: current.map(str::to_owned),
        live_leader_generations: live.iter().map(|g| g.to_string()).collect(),
        alive_pids: alive.iter().copied().collect(),
    }
}

#[test]
fn current_generation_rows_stay_owned() {
    let owned = record("t-own", 10, 100, Some("gen-now"));
    let decision = classify_registered_group(&owned, &facts(Some("gen-now"), &[], &[10]));
    assert_eq!(
        decision,
        SweepDecision::OwnedByLiveLeader {
            generation: "gen-now".to_owned()
        }
    );
}

#[test]
fn other_live_generation_rows_stay_owned() {
    let foreign = record("t-foreign", 20, 200, Some("gen-old"));
    let decision = classify_registered_group(
        &foreign,
        &facts(Some("gen-now"), &["gen-old"], &[20]),
    );
    assert_eq!(
        decision,
        SweepDecision::OwnedByLiveLeader {
            generation: "gen-old".to_owned()
        }
    );
}

#[test]
fn dead_generation_alive_pid_is_reaped() {
    let orphan = record("t-orphan", 30, 300, Some("gen-dead"));
    let decision = classify_registered_group(
        &orphan,
        &facts(Some("gen-now"), &["gen-other"], &[30, 99]),
    );
    assert_eq!(decision, SweepDecision::ReapOrphanedGroup);
}

#[test]
fn dead_generation_dead_pid_is_already_gone() {
    let stale = record("t-stale", 40, 400, Some("gen-dead"));
    let decision = classify_registered_group(&stale, &facts(Some("gen-now"), &[], &[99]));
    assert_eq!(decision, SweepDecision::AlreadyGone);
}

#[test]
fn untagged_rows_have_an_unknown_owner() {
    let legacy = record("t-legacy", 50, 500, None);
    let decision = classify_registered_group(&legacy, &facts(Some("gen-now"), &[], &[50]));
    assert_eq!(decision, SweepDecision::UnknownOwner);
}

#[test]
fn plan_reaps_unique_groups_and_keeps_the_decision_log() {
    let records = vec![
        record("t-a", 10, 100, Some("gen-dead")),
        record("t-b", 11, 100, Some("gen-dead")),
        record("t-c", 20, 200, Some("gen-now")),
        record("t-d", 30, 300, Some("gen-other")),
        record("t-e", 40, 400, Some("gen-dead")),
        record("t-f", 50, 500, None),
    ];
    let plan = decide_startup_sweep(
        &records,
        &facts(Some("gen-now"), &["gen-other"], &[10, 11, 40, 50]),
    );

    // t-a and t-b share a group; t-e's pid is gone; t-c and t-d are owned; t-f is unattributable.
    assert_eq!(plan.reap_process_group_ids, vec![100]);
    let task_ids: Vec<&str> = plan.decisions.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(task_ids, ["t-a", "t-b", "t-c", "t-d", "t-e", "t-f"]);
    assert_eq!(plan.decisions[0].1, SweepDecision::ReapOrphanedGroup);
    assert_eq!(plan.decisions[1].1, SweepDecision::ReapOrphanedGroup);
    assert_eq!(
        plan.decisions[2].1,
        SweepDecision::OwnedByLiveLeader {
            generation: "gen-now".to_owned()
        }
    );
    assert_eq!(
        plan.decisions[3].1,
        SweepDecision::OwnedByLiveLeader {
            generation: "gen-other".to_owned()
        }
    );
    assert_eq!(plan.decisions[4].1, SweepDecision::AlreadyGone);
    assert_eq!(plan.decisions[5].1, SweepDecision::UnknownOwner);
}

#[test]
fn empty_facts_never_reap() {
    let records = vec![record("t-1", 10, 100, Some("gen-dead"))];
    let plan = decide_startup_sweep(&records, &SweepFacts::default());
    assert!(plan.reap_process_group_ids.is_empty());
    assert_eq!(plan.decisions[0].1, SweepDecision::AlreadyGone);
}
