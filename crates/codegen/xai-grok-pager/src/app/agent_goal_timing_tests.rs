use super::{GoalDisplayState, GoalDisplayStatus};
use std::time::{Duration, Instant};

fn delayed_active() -> GoalDisplayState {
    let mut goal = GoalDisplayState::test_stub();
    goal.elapsed_ms = 5_000;
    goal.received_at = Instant::now().checked_sub(Duration::from_secs(20)).unwrap();
    goal
}

#[test]
fn delayed_pause_and_completion_discard_the_active_projection() {
    for status in [GoalDisplayStatus::UserPaused, GoalDisplayStatus::Complete] {
        let active = delayed_active();
        assert!(active.live_elapsed_ms() >= 25_000);
        let mut terminal = active.clone();
        terminal.status = status;
        terminal.elapsed_ms = 6_000;
        terminal.elapsed_floor_ms = active.live_elapsed_ms();
        terminal.received_at = Instant::now();
        assert_eq!(terminal.live_elapsed_ms(), 6_000);
        terminal.retain_active_elapsed_floor(Some(&active));
        assert_eq!(terminal.live_elapsed_ms(), 6_000);
        assert_eq!(terminal.elapsed_ms, 6_000);
    }
}

#[test]
fn resume_starts_from_the_paused_backend_accumulation_not_the_old_floor() {
    let active = delayed_active();
    let mut paused = active.clone();
    paused.status = GoalDisplayStatus::UserPaused;
    paused.elapsed_ms = 6_000;
    paused.elapsed_floor_ms = 25_000;
    let mut resumed = paused.clone();
    resumed.status = GoalDisplayStatus::Active;
    resumed.received_at = Instant::now().checked_sub(Duration::from_secs(2)).unwrap();
    resumed.retain_active_elapsed_floor(Some(&paused));
    assert_eq!(resumed.elapsed_ms, 6_000);
    assert_eq!(resumed.active_elapsed_ms_at(resumed.received_at + Duration::from_secs(2)), 8_000);
}

#[test]
fn active_smoothing_is_scoped_to_the_same_goal_and_nonhistorical_continuation() {
    let previous = delayed_active();
    let mut next = previous.clone();
    next.elapsed_ms = 6_000;
    next.received_at = Instant::now();
    next.retain_active_elapsed_floor(Some(&previous));
    assert!(next.live_elapsed_ms() >= 25_000);
    assert_eq!(next.elapsed_ms, 6_000);

    next.goal_id = "another-goal".into();
    next.retain_active_elapsed_floor(Some(&previous));
    assert_eq!(next.active_elapsed_ms_at(next.received_at), 6_000);

    next.goal_id = previous.goal_id.clone();
    next.retain_active_elapsed_floor(None);
    assert_eq!(next.active_elapsed_ms_at(next.received_at), 6_000);
}
