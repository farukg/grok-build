#![cfg(feature = "test-support")]
mod acp_harness;
mod admission_support;

use acp_harness::{connect_and_auth, prompt_turn, run_agent_test_isolated as run_agent_test};
use admission_support::*;
use serde_json::json;
use xai_grok_test_support::{InferenceEndpoint, InferenceRequestMatcher, ScriptedResponse};

const PARENT: &str = "019e0000-0000-7000-8000-0000000000d0";
const CHILD: &str = "019e0000-0000-7000-8000-0000000000d1";

#[test]
fn child_created_before_parent_goal_activation_delivers_success_and_error_to_the_later_goal() {
    for failed in [false, true] {
        run_agent_test(move |cwd, server| async move {
            let _planner = xai_grok_test_support::EnvGuard::set("GROK_GOAL_PLANNER", "0");
            let _classifier = xai_grok_test_support::EnvGuard::set("GROK_GOAL_CLASSIFIER", "0");
            let _summary = xai_grok_test_support::EnvGuard::set("GROK_GOAL_SUMMARY", "0");
            let _workflows = xai_grok_test_support::EnvGuard::set("GROK_WORKFLOWS", "0");
            let recorder = Recorder::default();
            let (conn, _) = connect_and_auth(recorder.clone(), "later-goal-completion-test").await;
            let sid = seed_and_load(&conn, &cwd, PARENT).await;
            let marker = if failed {
                "later-goal-actual-child-error"
            } else {
                "later-goal-actual-child-success"
            };
            let spawn = server.expect_response("spawn before any goal", InferenceRequestMatcher::foreground_containing(InferenceEndpoint::ChatCompletions, "initial-nongoal-spawn").for_session(PARENT),
                tool_reply("spawn_subagent", json!({"description":"child precedes goal activation","prompt":"execute before parent goal","subagent_type":"general-purpose","background":true,"task_id":CHILD})));
            let reply = if failed {
                ScriptedResponse::json(
                    400,
                    json!({"error":{"message":marker,"type":"invalid_request_error"}}),
                )
            } else {
                text_reply(marker)
            };
            let mut child = server.expect_response_blocked(
                "already-created child",
                InferenceRequestMatcher::foreground(InferenceEndpoint::ChatCompletions)
                    .for_session(CHILD),
                reply,
            );
            let parent_end = server.expect_response(
                "ordinary caller turn ends independently",
                InferenceRequestMatcher::foreground(InferenceEndpoint::ChatCompletions)
                    .for_session(PARENT),
                text_reply("ordinary turn finished while child lives"),
            );
            let _ = futures::join!(
                prompt_turn(&conn, &sid, "initial-nongoal-spawn"),
                child.wait_blocked()
            );
            let mut goal_sample = server.expect_response_blocked(
                "parent activates a later goal",
                InferenceRequestMatcher::foreground_containing(
                    InferenceEndpoint::ChatCompletions,
                    "later-goal-seed",
                )
                .for_session(PARENT),
                text_reply("goal sample in flight"),
            );
            let mut consumed = server.expect_response_blocked(
                "later goal receives earlier child's actual outcome",
                InferenceRequestMatcher::foreground_containing(
                    InferenceEndpoint::ChatCompletions,
                    marker,
                )
                .for_session(PARENT),
                tool_reply(
                    "update_goal",
                    json!({"completed":true,"message":"earlier child outcome handled"}),
                ),
            );
            let goal = prompt_turn(&conn, &sid, "/goal later-goal-seed");
            tokio::pin!(goal);
            tokio::select! {
                _ = goal_sample.wait_blocked() => {},
                _ = &mut goal => panic!(
                    "later goal ended before its active sampling boundary; {}; routes={:?}",
                    goal_sample.diagnostic(),
                    server.requests().into_iter()
                        .map(|entry| (entry.method, entry.path, entry.endpoint, entry.conversation))
                        .collect::<Vec<_>>()
                ),
            }
            child.release();
            tokio::select! { _ = recorder.child_finished(CHILD) => {}, _ = &mut goal => panic!("later goal ended before the earlier child terminal") }
            goal_sample.release();
            tokio::select! { _ = consumed.wait_blocked() => {}, _ = &mut goal => panic!("later goal lost the earlier child's outcome") }
            let body = main_requests(&server, PARENT)
                .into_iter()
                .find(|body| body.to_string().contains(marker))
                .expect("actual later goal sampling")
                .to_string();
            assert_eq!(body.matches(marker).count(), 1);
            assert!(body.contains(&format!("Exact durable result for completion {CHILD}/")));
            let final_report = server.expect_response(
                "later goal final report after actual goal acknowledgement",
                InferenceRequestMatcher::foreground_containing(
                    InferenceEndpoint::ChatCompletions,
                    "Goal marked complete.",
                )
                .for_session(PARENT),
                text_reply("later goal final report"),
            );
            consumed.release();
            goal.await;
            final_report.assert_satisfied();
            let items = history(&cwd, &sid).await;
            assert_goal_final_report(&items, "later goal final report");
            assert_completion_projection_once(
                &cwd,
                &sid,
                CHILD,
                &items,
                marker,
                CompletionHistoryKind::SubagentCompletion,
            );
            assert_eq!(
                items
                    .iter()
                    .filter(|item| item.text_content().contains(marker))
                    .count(),
                1,
                "original outcome must occur once in durable history; matches={:?}",
                matching_history_metadata(&items, marker)
            );
            spawn.assert_satisfied();
            child.assert_satisfied();
            parent_end.assert_satisfied();
            goal_sample.assert_satisfied();
            consumed.assert_satisfied();
        });
    }
}
