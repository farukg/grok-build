use super::*;

#[derive(Clone, Copy)]
enum End {
    Shutdown,
    ChannelClosed,
}

fn end_session(fixture: &mut Fixture, end: End) {
    match end {
        End::Shutdown => fixture.sender.send(SessionCommand::Shutdown(crate::session::ShutdownKind::Graceful)).unwrap(),
        End::ChannelClosed => {
            let (replacement, _receiver) = mpsc::unbounded_channel();
            drop(std::mem::replace(&mut fixture.sender, replacement));
        }
    }
}

async fn finish_teardown(fixture: &mut Fixture) {
    loop {
        tokio::select! {
            result = &mut fixture._task => {
                result.expect("session teardown must join normally");
                return;
            }
            message = fixture.persistence.recv() => match message.unwrap() {
                PersistenceMsg::FlushAndAck { respond_to } | PersistenceMsg::ProbeWritable { respond_to } => {
                    let _ = respond_to.send(Ok(()));
                }
                PersistenceMsg::AppendUpdateDurablyAndAck { respond_to, .. } => {
                    let _ = respond_to.send(Ok(()));
                }
                _ => {}
            },
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_and_channel_close_join_a_reserved_compact_before_teardown() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            for end in [End::Shutdown, End::ChannelClosed] {
                let mut fixture = fixture().await;
                let server = route_to_mock(&fixture.actor).await;
                let (reached, reserved) = oneshot::channel();
                let (release, released) = oneshot::channel();
                fixture.actor.compaction.cancel.manual_reservation_pause.replace(Some((reached, released)));
                let compact = submit_compact(&fixture);
                reserved.await.unwrap();
                let _later = submit_named_input(&fixture, "after-reservation", false);
                command_boundary(&fixture).await;
                assert!(fixture.actor.state.lock().await.finalization_gate.is_active());
                assert!(!fixture.actor.compaction.cancel.is_active(), "reservation precedes token-scope entry");

                end_session(&mut fixture, end);
                fixture.actor.compaction.cancel.manual_teardown_started.notified().await;
                let error = compact.await.unwrap().unwrap_err();
                assert_eq!(crate::session::helpers::session_compact::compact_error_kind(&error),
                    Some(crate::session::helpers::session_compact::CompactErrorKind::Cancelled));
                assert!(!fixture._task.is_finished(), "teardown must join the reserved worker, not just drain token holders");
                assert!(fixture.actor.state.lock().await.finalization_gate.is_active());
                release.send(()).unwrap();
                finish_teardown(&mut fixture).await;
                let state = fixture.actor.state.lock().await;
                assert!(!state.finalization_gate.is_active());
                assert!(state.running_task.is_none());
                assert!(fixture.actor.compaction.cancel.manual_is_sealed());
                assert!(!fixture.actor.has_pending_compact());
                assert!(server.requests().iter().all(|request| request.endpoint.is_none()),
                    "neither cancelled maintenance nor the later prompt may infer after reservation teardown");
            }
        }).await.expect("reserved compaction teardown must remain owned until the worker releases its lease");
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_and_channel_close_wait_for_compacts_own_ack_without_promoting_later_input() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            for end in [End::Shutdown, End::ChannelClosed] {
                let mut fixture = fixture().await;
                let server = route_to_mock(&fixture.actor).await;
                let mut compact = submit_compact(&fixture);
                let persisted = compact_barrier(&mut fixture).await;
                let _later = submit_named_input(&fixture, "after-committed-compact", false);
                command_boundary(&fixture).await;
                end_session(&mut fixture, end);
                fixture.actor.compaction.cancel.manual_teardown_started.notified().await;
                assert!(fixture.actor.compaction.cancel.manual_is_sealed());
                assert!(!fixture._task.is_finished(), "teardown cannot outrun compact durability");
                assert!(fixture.actor.state.lock().await.finalization_gate.is_active());
                assert!(matches!(compact.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
                persisted.send(Ok(())).unwrap();
                compact.await.unwrap().expect("committed compaction must finish its acknowledged boundary");
                finish_teardown(&mut fixture).await;
                let state = fixture.actor.state.lock().await;
                assert!(!state.finalization_gate.is_active());
                assert!(state.running_task.is_none());
                assert_eq!(server.requests().iter().filter(|request| request.conversation.is_some()).count(), 0,
                    "the maintenance worker must not re-kick a later foreground turn during teardown");
                assert!(!fixture.actor.has_pending_compact());
            }
        }).await.expect("committed compact teardown must join the worker after the original ack");
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn channel_close_during_generation_cancels_and_joins_the_original_compact() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let mut fixture = fixture().await;
            let server = route_to_mock(&fixture.actor).await;
            server.set_auxiliary_hold();
            let compact = submit_compact(&fixture);
            server.wait_until_parked_replies(1).await;
            let _later = submit_named_input(&fixture, "after-active-compact", false);
            command_boundary(&fixture).await;
            end_session(&mut fixture, End::ChannelClosed);
            fixture.actor.compaction.cancel.manual_teardown_started.notified().await;
            let error = compact.await.unwrap().unwrap_err();
            assert_eq!(crate::session::helpers::session_compact::compact_error_kind(&error),
                Some(crate::session::helpers::session_compact::CompactErrorKind::Cancelled));
            finish_teardown(&mut fixture).await;
            assert!(!fixture.actor.compaction.cancel.is_active());
            assert!(!fixture.actor.state.lock().await.finalization_gate.is_active());
            assert_eq!(server.requests().iter().filter(|request| request.conversation.is_some()).count(), 0);
            assert!(!fixture.actor.has_pending_compact());
            server.release_parked_replies();
        }).await.expect("command closure must join generation before session teardown completes");
    }).await;
}
