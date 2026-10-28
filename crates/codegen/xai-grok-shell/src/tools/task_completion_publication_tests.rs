use super::*;

#[tokio::test(flavor = "current_thread")]
async fn available_terminal_burst_uses_one_admission_and_one_trace() {
    tokio::task::LocalSet::new().run_until(async {
        let (config, mut gateway, _persistence, mut commands) = make_test_config_full_raw();
        let (trace_tx, mut traces) = mpsc::unbounded_channel();
        *config.synthetic_trace_tx.lock().unwrap() = Some(trace_tx);
        let handle = spawn_notification_bridge(config);
        handle.send_task_complete(make_task_snapshot("burst-a", TaskKind::Monitor));
        handle.send_monitor_event(xai_grok_tools::notification::types::MonitorEvent {
            task_id: "burst-a".into(), description: "finished monitor".into(), event_text: "terminal observation".into(),
            raw_text: "terminal observation".into(), owner_session_id: Some("test-session".into()),
        });
        for id in ["burst-b", "burst-c"] {
            handle.send_task_complete(make_task_snapshot(id, TaskKind::Bash));
        }
        let command = tokio::time::timeout(std::time::Duration::from_secs(5), commands.recv()).await.unwrap().unwrap();
        let SessionCommand::TaskCompletionBatch(batch) = command else { panic!("one completion batch required") };
        assert_eq!(batch.admissions.len(), 3);
        assert!(batch.automatic);
        let prompt_id = batch.prompt_id;
        for admission in batch.admissions { admission.respond_to.send(true).unwrap(); }
        let mut hooks = 0;
        let mut copies = 0;
        let mut monitor_drops = 0;
        while hooks < 3 {
            match tokio::time::timeout(std::time::Duration::from_secs(5), commands.recv()).await.unwrap().unwrap() {
                SessionCommand::CopyFile { respond_to } => { copies += 1; drop(respond_to); }
                SessionCommand::DispatchNotificationHook { notification_type, .. } => {
                    assert_eq!(notification_type, "task_complete");
                    hooks += 1;
                }
                SessionCommand::EmitBackgroundTasksSnapshot { .. } => {}
                SessionCommand::DropMonitorNotifications { task_id } => { assert_eq!(task_id, "burst-a"); monitor_drops += 1; }
                _ => panic!("terminal burst must not enqueue another wake"),
            }
        }
        assert_eq!(copies, 1);
        assert_eq!(monitor_drops, 1);
        assert_eq!(traces.try_recv().unwrap().prompt_id, prompt_id);
        assert!(traces.try_recv().is_err());
        let mut publications = 0;
        while let Ok(message) = gateway.try_recv() {
            if let xai_acp_lib::AcpClientMessage::ExtNotification(message) = message
                && message.request.method.as_ref() == "x.ai/task_completed"
            {
                let notification: crate::extensions::notification::SessionNotification = serde_json::from_str(message.request.params.get()).unwrap();
                assert!(matches!(notification.update, crate::extensions::notification::SessionUpdate::TaskCompleted { will_wake: true, .. }));
                publications += 1;
            }
        }
        assert_eq!(publications, 3);
    }).await;
}

#[tokio::test(start_paused = true)]
async fn stalled_burst_has_one_shared_admission_budget_and_preserves_every_fallback() {
    tokio::task::LocalSet::new().run_until(async {
        for (goal_active, acknowledge_first) in [(false, false), (true, false), (true, true)] {
            let (config, mut gateway, _persistence, mut commands) = make_test_config_full_raw();
            config.goal_loop_active.store(goal_active, Ordering::Release);
            let reservations = config.task_completion_reservations.clone();
            let (trace_tx, mut traces) = mpsc::unbounded_channel();
            *config.synthetic_trace_tx.lock().unwrap() = Some(trace_tx);
            let handle = spawn_notification_bridge(config);
            let started = tokio::time::Instant::now();
            for id in ["slow-a", "slow-b", "slow-c"] {
                handle.send_task_complete(make_task_snapshot(id, TaskKind::Bash));
            }
            let command = commands.recv().await.unwrap();
            let SessionCommand::TaskCompletionBatch(mut batch) = command else { panic!("goal policy must share receipt-first batch admission") };
            assert_eq!(batch.admissions.len(), 3);
            assert!(batch.automatic);
            if acknowledge_first {
                batch.admissions.remove(0).respond_to.send(true).unwrap();
            }
            tokio::time::advance(TASK_WAKE_ADMISSION_TIMEOUT + std::time::Duration::from_millis(1)).await;
            let mut publications = 0;
            while publications < 3 {
                let message = tokio::time::timeout(std::time::Duration::from_millis(1), gateway.recv()).await.unwrap().unwrap();
                if let xai_acp_lib::AcpClientMessage::ExtNotification(message) = message
                    && message.request.method.as_ref() == "x.ai/task_completed"
                {
                    let notification: crate::extensions::notification::SessionNotification = serde_json::from_str(message.request.params.get()).unwrap();
                    let crate::extensions::notification::SessionUpdate::TaskCompleted { task_snapshot, will_wake } = notification.update else { panic!("terminal publication required") };
                    assert_eq!(will_wake, acknowledge_first && task_snapshot.task_id == "slow-a");
                    publications += 1;
                }
            }
            assert!(started.elapsed() <= TASK_WAKE_ADMISSION_TIMEOUT + std::time::Duration::from_millis(1));
            let pending_ids = ["slow-a", "slow-b", "slow-c"].into_iter().skip(usize::from(acknowledge_first));
            for (admission, id) in batch.admissions.into_iter().zip(pending_ids) {
                assert_eq!(admission.fallback.source.task_id(), id);
                assert!(!admission.fallback.prompt_blocks.is_empty());
                assert!(reservations.contains(id));
                assert!(admission.respond_to.send(true).is_err(), "late admission must use the actor's retained fallback");
            }
            if acknowledge_first {
                assert_eq!(traces.try_recv().unwrap().prompt_id, batch.prompt_id);
            }
            assert!(traces.try_recv().is_err(), "one cohort has at most one trace even with goal policy active");
            let mut copies = 0;
            while let Ok(command) = commands.try_recv() {
                match command {
                    SessionCommand::CopyFile { .. } => { copies += 1; }
                    SessionCommand::DispatchNotificationHook { .. } | SessionCommand::EmitBackgroundTasksSnapshot { .. } => {}
                    _ => panic!("stalled cohort must not add policy handshakes or wakes"),
                }
            }
            assert_eq!(copies, usize::from(acknowledge_first));
        }
    }).await;
}
