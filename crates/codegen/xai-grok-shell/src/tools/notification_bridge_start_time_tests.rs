use super::*;
use std::time::{Duration, SystemTime};

#[tokio::test]
async fn backgrounded_start_survives_delayed_delivery_and_legacy_wire() {
    let delivery_time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_060);
    let registry_start = delivery_time - Duration::from_secs(60);
    for start_time in [Some(registry_start), None] {
        let (config, mut gateway_rx, mut persistence_rx, _cmd_rx) = make_test_config_full_raw();
        let notification = ToolNotification::BashExecutionBackgrounded(
            xai_grok_tools::notification::types::BashExecutionBackgrounded {
                base: xai_grok_tools::notification::types::BashNotificationBase {
                    start_time,
                    tool_call_id: "delayed-call".into(),
                    command: "sleep 100".into(),
                    output: Vec::new(),
                    total_bytes: 0,
                    truncated: false,
                    cwd: PathBuf::from("/tmp"),
                },
                output_file: PathBuf::from("/tmp/delayed.log"),
                task_id: "delayed-task".into(),
                monitor_description: None,
                description: None,
            },
        );
        handle_notification(
            &config,
            &crate::extensions::tool_timing::ToolCallOwnership::Unknown,
            notification,
            &mut HashMap::new(),
        ).await;
        let PersistenceMsg::Update(crate::session::storage::SessionUpdate::Xai(saved)) =
            persistence_rx.try_recv().expect("backgrounded persisted before registry response")
        else { panic!("expected backgrounded update") };
        let crate::extensions::notification::SessionUpdate::TaskBackgrounded {
            start_time: saved_start, ..
        } = &saved.update else { panic!("expected backgrounded variant") };
        assert_eq!(*saved_start, start_time);
        let wire = serde_json::to_value(&*saved).unwrap();
        match start_time {
            Some(start) => assert_eq!(wire["update"]["start_time"], serde_json::to_value(start).unwrap()),
            None => assert!(wire["update"].get("start_time").is_none()),
        }
        let mut delivered = None;
        while let Ok(message) = gateway_rx.try_recv() {
            if let xai_acp_lib::AcpClientMessage::ExtNotification(args) = message
                && args.request.method.as_ref() == "x.ai/task_backgrounded"
            {
                delivered = Some(serde_json::from_str::<serde_json::Value>(args.request.params.get()).unwrap());
            }
        }
        assert_eq!(delivered, Some(wire.clone()));
        let decoded: crate::extensions::notification::SessionNotification = serde_json::from_value(wire).unwrap();
        let crate::extensions::notification::SessionUpdate::TaskBackgrounded {
            start_time: decoded_start, ..
        } = decoded.update else { panic!("expected backgrounded variant") };
        assert_eq!(decoded_start, start_time);
    }
}
