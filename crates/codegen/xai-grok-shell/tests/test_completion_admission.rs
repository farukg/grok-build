#![cfg(feature = "test-support")]
mod acp_harness;
mod admission_support;

use acp_harness::{connect_and_auth, ext_method, prompt_turn, run_agent_test};
use admission_support::*;
use agent_client_protocol::{self as acp, Agent as _};
use serde_json::json;
use xai_grok_shell::session::delivery_receiver::{DeliveryProgress, DeliveryResult};
use xai_grok_test_support::{InferenceEndpoint, InferenceRequestMatcher, ScriptedResponse};

const PARENT: &str = "019e0000-0000-7000-8000-0000000000a0";
const PAYLOAD: &str = "  SIG exact receiver envelope\n/trusted-looking-command\n  ";

#[test]
fn native_admission_is_durable_before_terminal_and_exact_retries_have_one_input() {
    run_agent_test(|cwd, server| async move {
        let recorder = Recorder::default();
        let (conn, _) = connect_and_auth(recorder.clone(), "receiver-test").await;
        let sid = seed_and_load(&conn, &cwd, PARENT).await;
        assert!(matches!(
            delivery(&conn, &sid, "absent", PAYLOAD, "observe").await,
            DeliveryResult::Absent
        ));
        assert!(
            main_requests(&server, PARENT).is_empty(),
            "Observe cannot schedule a turn"
        );
        let mut sampled = server.expect_response_blocked(
            "receiver input",
            InferenceRequestMatcher::foreground(InferenceEndpoint::ChatCompletions)
                .for_session(PARENT),
            text_reply("receiver finished"),
        );
        let first = admitted(delivery(&conn, &sid, "stable-id", PAYLOAD, "admit").await);
        assert_eq!(first.revision, 1);
        assert_eq!(first.progress, DeliveryProgress::Queued);
        sampled.wait_blocked().await;
        let processing = admitted(delivery(&conn, &sid, "stable-id", PAYLOAD, "observe").await);
        assert_eq!(processing.revision, 2);
        assert!(matches!(
            processing.progress,
            DeliveryProgress::Processing { .. }
        ));
        assert_eq!(
            admitted(delivery(&conn, &sid, "stable-id", PAYLOAD, "admit").await).revision,
            2
        );
        assert!(matches!(
            delivery(&conn, &sid, "stable-id", PAYLOAD.trim(), "admit").await,
            DeliveryResult::Conflict { .. }
        ));
        sampled.release();
        assert_succeeded(&recorder.terminal("stable-id").await);
        let terminal = admitted(delivery(&conn, &sid, "stable-id", PAYLOAD, "observe").await);
        assert_eq!(terminal.revision, 4);
        assert_succeeded(&terminal);
        assert_eq!(
            admitted(delivery(&conn, &sid, "stable-id", PAYLOAD, "admit").await).revision,
            4
        );
        let items = history(&cwd, &sid).await;
        assert_eq!(
            items
                .iter()
                .filter(|item| item.text_content() == PAYLOAD)
                .count(),
            1
        );
        assert_eq!(main_requests(&server, PARENT).len(), 1);
        sampled.assert_satisfied();
    });
}

#[test]
fn native_receiver_records_actual_terminal_error_without_readmission() {
    run_agent_test(|cwd, server| async move {
        let recorder = Recorder::default();
        let (conn, _) = connect_and_auth(recorder.clone(), "receiver-error-test").await;
        let sid = seed_and_load(&conn, &cwd, PARENT).await;
        let error = server.expect_response("receiver actual error", InferenceRequestMatcher::foreground(InferenceEndpoint::ChatCompletions).for_session(PARENT),
            ScriptedResponse::json(400, json!({"error":{"message":"receiver-real-invalid-request-marker","type":"invalid_request_error"}})));
        admitted(delivery(&conn, &sid, "failed-id", "receiver-error-payload", "admit").await);
        let receipt = recorder.terminal("failed-id").await;
        assert!(
            matches!(&receipt.progress, DeliveryProgress::Failed { reason } if reason.contains("receiver-real-invalid-request-marker")),
            "actual error must remain visible: {receipt:?}"
        );
        let retry =
            admitted(delivery(&conn, &sid, "failed-id", "receiver-error-payload", "admit").await);
        assert_eq!(retry.revision, receipt.revision);
        assert_eq!(retry.progress, receipt.progress);
        assert_eq!(
            history(&cwd, &sid)
                .await
                .iter()
                .filter(|item| item.text_content() == "receiver-error-payload")
                .count(),
            1
        );
        error.assert_satisfied();
    });
}

#[test]
fn native_receiver_cancel_retains_its_parent_binding_after_task_teardown() {
    run_agent_test(|cwd, server| async move {
        let recorder = Recorder::default();
        let (conn, _) = connect_and_auth(recorder.clone(), "receiver-cancel-test").await;
        let sid = seed_and_load(&conn, &cwd, PARENT).await;
        let mut sampled = server.expect_response_blocked(
            "receiver cancelled sample",
            InferenceRequestMatcher::foreground(InferenceEndpoint::ChatCompletions)
                .for_session(PARENT),
            text_reply("must not complete"),
        );
        admitted(
            delivery(
                &conn,
                &sid,
                "cancelled-id",
                "cancelled-receiver-payload",
                "admit",
            )
            .await,
        );
        sampled.wait_blocked().await;
        conn.cancel(acp::CancelNotification::new(sid.clone()))
            .await
            .unwrap();
        let receipt = recorder.terminal("cancelled-id").await;
        assert!(matches!(
            receipt.progress,
            DeliveryProgress::Cancelled { .. }
        ));
        let retry = admitted(
            delivery(
                &conn,
                &sid,
                "cancelled-id",
                "cancelled-receiver-payload",
                "admit",
            )
            .await,
        );
        assert_eq!(retry.progress, receipt.progress);
        assert_eq!(retry.revision, receipt.revision);
        assert_eq!(main_requests(&server, PARENT).len(), 1);
        assert_eq!(
            history(&cwd, &sid)
                .await
                .iter()
                .filter(|item| item.text_content() == "cancelled-receiver-payload")
                .count(),
            1
        );
        sampled.release();
        sampled.assert_satisfied();
    });
}

#[test]
fn native_receiver_admits_during_sampling_then_projects_at_the_next_safe_boundary() {
    active_receiver(false);
}
#[test]
fn native_receiver_waits_for_a_real_tool_and_preserves_tool_result_order() {
    active_receiver(true);
}

fn active_receiver(with_tool: bool) {
    run_agent_test(move |cwd, server| async move {
        use tokio::io::AsyncWriteExt;
        let recorder = Recorder::default();
        let (conn, _) = connect_and_auth(recorder.clone(), "receiver-active-test").await;
        let sid = seed_and_load(&conn, &cwd, PARENT).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("tool gate");
        let port = listener.local_addr().unwrap().port();
        let response = if with_tool {
            tool_reply(
                "run_terminal_command",
                json!({"command":format!("bash -c 'exec 3<>/dev/tcp/127.0.0.1/{port}; IFS= read -r line <&3; printf \"%s\\n\" \"$line\"'"),"description":"real receiver tool boundary","block_until_ms":60000}),
            )
        } else {
            text_reply("first sample done")
        };
        let mut first = server.expect_response_blocked(
            "active parent first response",
            InferenceRequestMatcher::foreground_containing(
                InferenceEndpoint::ChatCompletions,
                "receiver-parent-human",
            )
            .for_session(PARENT),
            response,
        );
        let human = prompt_turn(&conn, &sid, "receiver-parent-human");
        tokio::pin!(human);
        tokio::select! { _ = first.wait_blocked() => {}, _ = &mut human => panic!("human turn ended before boundary") }
        let mut socket = None;
        if with_tool {
            first.release();
            tokio::select! { ready = listener.accept() => socket = Some(ready.expect("real tool connected").0), _ = &mut human => panic!("human turn ended while tool must remain active") }
        }
        let mut next = server.expect_response_blocked(
            "receiver after safe boundary",
            InferenceRequestMatcher::foreground_containing(
                InferenceEndpoint::ChatCompletions,
                "receiver-next-boundary-marker",
            )
            .for_session(PARENT),
            text_reply("all inputs handled"),
        );
        let receipt = admitted(
            delivery(
                &conn,
                &sid,
                "active-id",
                "receiver-next-boundary-marker",
                "admit",
            )
            .await,
        );
        assert_eq!(receipt.progress, DeliveryProgress::Queued);
        assert!(
            main_requests(&server, PARENT)
                .iter()
                .all(|body| !body.to_string().contains("receiver-next-boundary-marker"))
        );
        match socket {
            Some(mut socket) => socket.write_all(b"receiver-tool-done\n").await.unwrap(),
            None => first.release(),
        }
        tokio::select! { _ = next.wait_blocked() => {}, _ = &mut human => panic!("receiver was deferred past the active turn") }
        let body = main_requests(&server, PARENT)
            .into_iter()
            .find(|body| body.to_string().contains("receiver-next-boundary-marker"))
            .expect("receiver reached sampler")
            .to_string();
        assert_eq!(body.matches("receiver-next-boundary-marker").count(), 1);
        if with_tool {
            assert!(body.find("receiver-tool-done") < body.find("receiver-next-boundary-marker"));
        }
        next.release();
        human.await;
        assert_succeeded(&recorder.terminal("active-id").await);
        assert_eq!(
            history(&cwd, &sid)
                .await
                .iter()
                .filter(|item| item.text_content() == "receiver-next-boundary-marker")
                .count(),
            1
        );
        first.assert_satisfied();
        next.assert_satisfied();
    });
}

const CHILD: &str = "019e0000-0000-7000-8000-0000000000a1";
const SUCCESS: &str = "actual-child-success-marker";
const ERROR: &str = "actual-child-error-marker";
#[test]
fn foreground_child_success_and_error_commit_the_observed_tool_result_once() {
    for failed in [false, true] {
        run_agent_test(move |cwd, server| async move {
            let recorder = Recorder::default();
            let (conn, _) = connect_and_auth(recorder.clone(), "foreground-completion-test").await;
            let sid = seed_and_load(&conn, &cwd, PARENT).await;
            let marker = if failed { ERROR } else { SUCCESS };
            let initial = server.expect_response("foreground child spawn", InferenceRequestMatcher::foreground(InferenceEndpoint::ChatCompletions).for_session(PARENT),
                tool_reply("spawn_subagent", json!({"description":"foreground result receipt","prompt":"foreground child execution","subagent_type":"general-purpose","background":false,"task_id":CHILD})));
            let child_reply = if failed {
                ScriptedResponse::json(
                    400,
                    json!({"error":{"message":ERROR,"type":"invalid_request_error"}}),
                )
            } else {
                text_reply(SUCCESS)
            };
            let child = server.expect_response(
                "foreground actual child outcome",
                InferenceRequestMatcher::foreground(InferenceEndpoint::ChatCompletions)
                    .for_session(CHILD),
                child_reply,
            );
            let consumed = server.expect_response(
                "parent samples committed child tool result",
                InferenceRequestMatcher::foreground_containing(
                    InferenceEndpoint::ChatCompletions,
                    marker,
                )
                .for_session(PARENT),
                text_reply("foreground child consumed"),
            );
            prompt_turn(&conn, &sid, "foreground-child-seed").await;
            recorder.child_finished(CHILD).await;
            let items = history(&cwd, &sid).await;
            assert_eq!(
                items
                    .iter()
                    .filter(|item| item.text_content().contains(marker))
                    .count(),
                1
            );
            assert!(items.iter().any(|item| matches!(
                item,
                xai_grok_shell::sampling::ConversationItem::ToolResult(_)
            ) && item.text_content().contains(marker)));
            let requests = main_requests(&server, PARENT);
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[1].to_string().matches(marker).count(), 1);
            initial.assert_satisfied();
            child.assert_satisfied();
            consumed.assert_satisfied();
        });
    }
}

#[derive(Clone, Copy)]
enum CallerBoundary {
    Idle,
    Sampling,
    Tool,
    GoalSampling,
    GoalTool,
}

#[test]
fn actual_child_success_and_error_wake_idle_caller_after_ordinary_turn_end() {
    child_completion(CallerBoundary::Idle);
}
#[test]
fn actual_child_success_and_error_reach_active_caller_next_sample() {
    child_completion(CallerBoundary::Sampling);
}
#[test]
fn actual_child_success_and_error_wait_for_active_tool_result() {
    child_completion(CallerBoundary::Tool);
}
#[test]
fn goal_caller_receives_actual_child_success_and_error_at_sampling_boundary() {
    child_completion(CallerBoundary::GoalSampling);
}
#[test]
fn goal_caller_receives_actual_child_success_and_error_after_real_tool() {
    child_completion(CallerBoundary::GoalTool);
}

fn child_completion(boundary: CallerBoundary) {
    for failed in [false, true] {
        run_agent_test(move |cwd, server| async move {
            use tokio::io::AsyncWriteExt;
            let goal = matches!(
                boundary,
                CallerBoundary::GoalSampling | CallerBoundary::GoalTool
            );
            let _planner = xai_grok_test_support::EnvGuard::set("GROK_GOAL_PLANNER", "0");
            let _classifier = xai_grok_test_support::EnvGuard::set("GROK_GOAL_CLASSIFIER", "0");
            let _summary = xai_grok_test_support::EnvGuard::set("GROK_GOAL_SUMMARY", "0");
            let _workflows = xai_grok_test_support::EnvGuard::set("GROK_WORKFLOWS", "0");
            let recorder = Recorder::default();
            let (conn, _) = connect_and_auth(recorder.clone(), "caller-completion-test").await;
            let sid = seed_and_load(&conn, &cwd, PARENT).await;
            let marker = if failed { ERROR } else { SUCCESS };
            let initial = server.expect_response("parent spawns real child", InferenceRequestMatcher::foreground_containing(InferenceEndpoint::ChatCompletions, "caller-spawn-seed").for_session(PARENT),
                tool_reply("spawn_subagent", json!({"description":"completion recovery child","prompt":"child actual execution prompt","subagent_type":"general-purpose","background":true,"task_id":CHILD})));
            let child_response = if failed {
                ScriptedResponse::json(
                    400,
                    json!({"error":{"message":ERROR,"type":"invalid_request_error"}}),
                )
            } else {
                text_reply(SUCCESS)
            };
            let mut child = server.expect_response_blocked(
                "real child terminal",
                InferenceRequestMatcher::foreground(InferenceEndpoint::ChatCompletions)
                    .for_session(CHILD),
                child_response,
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let parent_response = match boundary {
                CallerBoundary::Idle | CallerBoundary::Sampling | CallerBoundary::GoalSampling => {
                    text_reply("ordinary caller turn ended")
                }
                CallerBoundary::Tool | CallerBoundary::GoalTool => tool_reply(
                    "run_terminal_command",
                    json!({"command":format!("bash -c 'exec 3<>/dev/tcp/127.0.0.1/{port}; IFS= read -r line <&3; printf \"%s\\n\" \"$line\"'"),"description":"caller completion tool boundary","block_until_ms":60000}),
                ),
            };
            let mut parent = server.expect_response_blocked(
                "parent after spawn",
                InferenceRequestMatcher::foreground(InferenceEndpoint::ChatCompletions)
                    .for_session(PARENT),
                parent_response,
            );
            let final_reply = if goal {
                tool_reply(
                    "update_goal",
                    json!({"completed":true,"message":"child result handled"}),
                )
            } else {
                text_reply("child result handled")
            };
            let mut woke = server.expect_response_blocked(
                "parent actually samples child result",
                InferenceRequestMatcher::foreground_containing(
                    InferenceEndpoint::ChatCompletions,
                    marker,
                )
                .for_session(PARENT),
                final_reply,
            );
            let human = prompt_turn(
                &conn,
                &sid,
                if goal {
                    "/goal caller-spawn-seed"
                } else {
                    "caller-spawn-seed"
                },
            );
            tokio::pin!(human);
            tokio::select! { _ = async { child.wait_blocked().await; parent.wait_blocked().await; } => {}, _ = &mut human => panic!("caller unexpectedly ended before scripted boundary") }
            let mut socket = None;
            match boundary {
                CallerBoundary::Idle => {
                    parent.release();
                    human.as_mut().await;
                }
                CallerBoundary::Sampling | CallerBoundary::GoalSampling => {}
                CallerBoundary::Tool | CallerBoundary::GoalTool => {
                    parent.release();
                    tokio::select! { ready = listener.accept() => socket = Some(ready.unwrap().0), _ = &mut human => panic!("caller ended with tool still blocked") }
                }
            }
            child.release();
            match boundary {
                CallerBoundary::Idle => recorder.child_finished(CHILD).await,
                CallerBoundary::Sampling
                | CallerBoundary::Tool
                | CallerBoundary::GoalSampling
                | CallerBoundary::GoalTool => {
                    tokio::select! { _ = recorder.child_finished(CHILD) => {}, _ = &mut human => panic!("caller ended before completion admission") }
                }
            }
            if !matches!(boundary, CallerBoundary::Idle) {
                assert!(
                    main_requests(&server, PARENT)
                        .iter()
                        .all(|body| !body.to_string().contains(marker)),
                    "an in-flight operation is a protected boundary"
                );
            }
            match boundary {
                CallerBoundary::Idle => {}
                CallerBoundary::Sampling | CallerBoundary::GoalSampling => parent.release(),
                CallerBoundary::Tool | CallerBoundary::GoalTool => {
                    socket
                        .as_mut()
                        .unwrap()
                        .write_all(b"caller-tool-done\n")
                        .await
                        .unwrap();
                }
            }
            match boundary {
                CallerBoundary::Idle => woke.wait_blocked().await,
                CallerBoundary::Sampling
                | CallerBoundary::Tool
                | CallerBoundary::GoalSampling
                | CallerBoundary::GoalTool => {
                    tokio::select! { _ = woke.wait_blocked() => {}, _ = &mut human => panic!("child result missed active safe point") }
                }
            }
            let body = main_requests(&server, PARENT)
                .into_iter()
                .find(|body| body.to_string().contains(marker))
                .expect("real completion reaches parent sampler")
                .to_string();
            assert_eq!(body.matches(marker).count(), 1);
            if matches!(boundary, CallerBoundary::Tool | CallerBoundary::GoalTool) {
                assert!(body.find("caller-tool-done") < body.find(marker));
            }
            woke.release();
            if !matches!(boundary, CallerBoundary::Idle) {
                human.await;
            }
            let items = history(&cwd, &sid).await;
            assert_eq!(
                items
                    .iter()
                    .filter(|item| item.text_content().contains(marker))
                    .count(),
                1
            );
            initial.assert_satisfied();
            child.assert_satisfied();
            parent.assert_satisfied();
            woke.assert_satisfied();
            let _ = ext_method(&conn, "x.ai/session/delivery", json!({"sessionId":sid.to_string(),"deliveryId":"observe-barrier","payload":"","action":"observe"})).await;
        });
    }
}
