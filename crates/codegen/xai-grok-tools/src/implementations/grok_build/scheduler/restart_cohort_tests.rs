use super::*;
use super::tests::make_boundary_actor;
use tokio::sync::oneshot;
use crate::types::restart_recovery::LeaderRestartRecovery;

#[tokio::test]
async fn actor_reconstruction_never_cancels_post_startup_tasks() {
    let startup = LeaderRestartRecovery::stale(Utc::now());
    let recovery = startup.register("parent", Utc::now() - chrono::Duration::hours(2), Default::default());
    let old = ScheduledTask::new(3600, "old cohort".into(), true, false);
    let old_id = old.id.clone();
    let (mut actor, _) = make_boundary_actor(vec![old], 0);
    actor.resources.lock().await.insert(recovery.clone());
    let resources = actor.resources.clone();
    let cancel = actor.cancel_token.clone();
    let (commands, rx) = mpsc::unbounded_channel();
    actor.cmd_rx = rx;
    let running = tokio::spawn(actor.run());
    let (reply, response) = oneshot::channel();
    commands.send(SchedulerCommand::List { reply }).unwrap();
    assert!(response.await.unwrap().tasks.is_empty());
    recovery.acknowledge(recovery.admission().after_user_message());
    let new = ScheduledTask::new(3600, "new work".into(), true, false);
    let new_id = new.id.clone();
    let (reply, response) = oneshot::channel();
    commands.send(SchedulerCommand::Create { task: new, reply }).unwrap();
    response.await.unwrap().unwrap();
    cancel.cancel();
    running.await.unwrap();
    let saved_tasks = resources.lock().await.get::<State<SchedulerState>>().unwrap().tasks.clone();
    for _ in 0..2 {
        let (mut rebuilt, _) = make_boundary_actor(saved_tasks.clone(), 0);
        rebuilt.resources.lock().await.insert(startup.session("parent"));
        let cancel = rebuilt.cancel_token.clone();
        let (commands, rx) = mpsc::unbounded_channel();
        rebuilt.cmd_rx = rx;
        let running = tokio::spawn(rebuilt.run());
        let (reply, response) = oneshot::channel();
        commands.send(SchedulerCommand::List { reply }).unwrap();
        let tasks = response.await.unwrap().tasks;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, new_id);
        assert_ne!(tasks[0].id, old_id);
        cancel.cancel();
        running.await.unwrap();
    }
}

#[tokio::test]
async fn stale_startup_refuses_non_message_schedule_creation() {
    let startup = LeaderRestartRecovery::stale(Utc::now());
    let (mut actor, _) = make_boundary_actor(Vec::new(), 0);
    actor.resources.lock().await.insert(startup.register(
        "parent", Utc::now() - chrono::Duration::hours(2), Default::default(),
    ));
    let cancel = actor.cancel_token.clone();
    let (commands, rx) = mpsc::unbounded_channel();
    actor.cmd_rx = rx;
    let running = tokio::spawn(actor.run());
    let (reply, response) = oneshot::channel();
    commands.send(SchedulerCommand::Create {
        task: ScheduledTask::new(1, "not a user message".into(), true, false), reply,
    }).unwrap();
    assert!(matches!(response.await.unwrap(), Err(SchedulerError::RestartQuiescent)));
    let (reply, response) = oneshot::channel();
    commands.send(SchedulerCommand::List { reply }).unwrap();
    assert!(response.await.unwrap().tasks.is_empty());
    cancel.cancel();
    running.await.unwrap();
}

#[tokio::test]
async fn continuing_startup_preserves_existing_scheduled_work() {
    let task = ScheduledTask::new(3600, "preserved".into(), true, false);
    let id = task.id.clone();
    let (mut actor, _) = make_boundary_actor(vec![task], 0);
    actor.resources.lock().await.insert(LeaderRestartRecovery::default().register(
        "parent", Utc::now() - chrono::Duration::hours(2), Default::default(),
    ));
    let cancel = actor.cancel_token.clone();
    let (commands, rx) = mpsc::unbounded_channel();
    actor.cmd_rx = rx;
    let running = tokio::spawn(actor.run());
    let (reply, response) = oneshot::channel();
    commands.send(SchedulerCommand::List { reply }).unwrap();
    let tasks = response.await.unwrap().tasks;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, id);
    cancel.cancel();
    running.await.unwrap();
}

#[tokio::test]
async fn failed_startup_ack_retries_only_after_explicit_message_admission() {
    use crate::notification::ToolNotification;
    let startup = LeaderRestartRecovery::stale(Utc::now());
    let recovery = startup.register("parent", Utc::now() - chrono::Duration::hours(2), Default::default());
    let old = ScheduledTask::new(3600, "old".into(), true, false);
    let old_id = old.id.clone();
    let (mut actor, _) = make_boundary_actor(vec![old], 0);
    actor.resources.lock().await.insert(recovery.clone());
    let (plain, mut observed) = ToolNotificationHandle::channel();
    let (durable, mut deliveries) = ToolNotificationHandle::acknowledged_channel();
    tokio::spawn(async move {
        let mut first_ack = true;
        while let Some(delivery) = deliveries.recv().await {
            if let Some(ack) = delivery.acknowledgement {
                let response = if first_ack { first_ack = false; Err("injected durable ACK failure".into()) } else { Ok(()) };
                let _ = ack.send(response);
            }
        }
    });
    actor.notification_handle = ToolNotificationHandle::tee(vec![plain, durable]);
    let cancel = actor.cancel_token.clone();
    let (commands, rx) = mpsc::unbounded_channel();
    actor.cmd_rx = rx;
    let running = tokio::spawn(actor.run());
    let (reply, response) = oneshot::channel();
    commands.send(SchedulerCommand::List { reply }).unwrap();
    assert!(response.await.unwrap().tasks.is_empty());
    let (reply, response) = oneshot::channel();
    commands.send(SchedulerCommand::Create { task: ScheduledTask::new(3600, "not admitted".into(), true, false), reply }).unwrap();
    assert!(matches!(response.await.unwrap(), Err(SchedulerError::RestartQuiescent)));
    recovery.acknowledge(recovery.admission().after_user_message());
    let new = ScheduledTask::new(3600, "new user work".into(), true, false);
    let new_id = new.id.clone();
    let (reply, response) = oneshot::channel();
    commands.send(SchedulerCommand::Create { task: new, reply }).unwrap();
    assert_eq!(response.await.unwrap().unwrap().id, new_id);
    let (reply, response) = oneshot::channel();
    commands.send(SchedulerCommand::List { reply }).unwrap();
    let tasks = response.await.unwrap().tasks;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, new_id);
    assert_ne!(tasks[0].id, old_id);
    cancel.cancel();
    running.await.unwrap();
    while let Ok(notification) = observed.try_recv() {
        assert!(!matches!(notification, ToolNotification::ScheduledTaskFired(_)));
    }
}
