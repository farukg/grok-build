#![cfg(feature = "test-support")]
mod acp_harness;
mod admission_support;

use acp_harness::{connect_and_auth, prompt_turn, run_agent_test};
use admission_support::*;
use agent_client_protocol::{self as acp, Agent as _};
use serde_json::json;
use xai_grok_shell::session::delivery_receiver::{DeliveryProgress, DeliveryResult};
use xai_grok_shell::session::storage::{JsonlStorageAdapter, StorageAdapter};
use xai_grok_test_support::{InferenceEndpoint, InferenceRequestMatcher, ScriptedResponse};

const SESSION: &str = "019e0000-0000-7000-8000-0000000000b0";
const CHILD: &str = "019e0000-0000-7000-8000-0000000000b1";
const RESULT: &str = "process-child-actual-result";
const ERROR: &str = "process-child-actual-error";
const ENVELOPE: &str = "restart-owned-native-envelope";

async fn worker(action: &str, delivery_id: &str, crash: Option<&str>, cwd: &std::path::Path) -> std::process::Output {
    let mut command = tokio::process::Command::new(std::env::current_exe().expect("test executable"));
    command.args(["--exact", "admission_process_worker", "--nocapture"])
        .env("GROK_ADMISSION_WORKER_ACTION", action).env("GROK_ADMISSION_WORKER_DELIVERY", delivery_id)
        .env("GROK_ADMISSION_WORKER_CWD", cwd).env_remove("GROK_ADMISSION_TEST_CRASH");
    if let Some(boundary) = crash { command.env("GROK_ADMISSION_TEST_CRASH", boundary); }
    tokio::time::timeout(std::time::Duration::from_secs(180), command.output()).await.expect("worker deadline").expect("worker OS process")
}
fn worker_result(output: &std::process::Output) -> DeliveryResult {
    assert!(output.status.success(), "worker failed: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let raw = stdout.lines().find_map(|line| line.strip_prefix("ADMISSION_RESULT=")).expect("actual receiver result from worker");
    serde_json::from_str(raw).expect("worker receipt JSON")
}
async fn seed(cwd: &std::path::Path) -> acp::SessionId {
    let id = acp::SessionId::new(SESSION);
    JsonlStorageAdapter::new().init_session(&xai_grok_shell::session::info::Info { id: id.clone(), cwd: cwd.to_string_lossy().into_owned() }, acp::ModelId::new("test-model")).await.unwrap();
    id
}

#[test]
fn admission_process_worker() {
    let Ok(action) = std::env::var("GROK_ADMISSION_WORKER_ACTION") else { return; };
    let id = std::env::var("GROK_ADMISSION_WORKER_DELIVERY").unwrap();
    let cwd = std::path::PathBuf::from(std::env::var_os("GROK_ADMISSION_WORKER_CWD").unwrap());
    xai_grok_extra_ca::ensure_default_crypto_provider();
    xai_grok_telemetry::external::mark_external_otel_settings_resolved();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(tokio::task::LocalSet::new().run_until(async {
        let recorder = Recorder::default();
        let (conn, _) = connect_and_auth(recorder.clone(), "admission-process-worker").await;
        let sid = acp::SessionId::new(SESSION);
        match action.as_str() {
            "observe" => {
                let result = delivery(&conn, &sid, &id, ENVELOPE, "observe").await;
                println!("ADMISSION_RESULT={}", serde_json::to_string(&result).unwrap());
            }
            "admit" => {
                conn.load_session(acp::LoadSessionRequest::new(sid.clone(), cwd)).await.unwrap();
                let receipt = admitted(delivery(&conn, &sid, &id, ENVELOPE, "admit").await);
                if matches!(receipt.progress, DeliveryProgress::Queued | DeliveryProgress::Processing { .. }) {
                    recorder.terminal(&id).await;
                }
                let result = delivery(&conn, &sid, &id, ENVELOPE, "observe").await;
                println!("ADMISSION_RESULT={}", serde_json::to_string(&result).unwrap());
            }
            "retry" => {
                let result = delivery(&conn, &sid, &id, ENVELOPE, "admit").await;
                println!("ADMISSION_RESULT={}", serde_json::to_string(&result).unwrap());
            }
            "spawn" => {
                conn.load_session(acp::LoadSessionRequest::new(sid.clone(), cwd)).await.unwrap();
                prompt_turn(&conn, &sid, "process-spawn-seed").await;
                recorder.child_finished(CHILD).await;
            }
            "recover" => {
                conn.load_session(acp::LoadSessionRequest::new(sid, cwd)).await.unwrap();
                recorder.recovered_turn().await;
            }
            other => panic!("unknown worker action {other}"),
        }
    }));
}

#[test]
fn process_restart_after_lost_admission_ack_recovers_the_same_queued_input_once() {
    run_agent_test(|cwd, server| async move {
        let sid = seed(&cwd).await;
        let crashed = worker("admit", "queued-crash", Some("receiver_admitted"), &cwd).await;
        assert_eq!(crashed.status.code(), Some(86));
        let before = main_requests(&server, SESSION).len();
        let observed = admitted(worker_result(&worker("observe", "queued-crash", None, &cwd).await));
        assert_eq!(observed.progress, DeliveryProgress::Queued);
        assert_eq!(observed.revision, 1);
        assert_eq!(main_requests(&server, SESSION).len(), before, "Observe process must stay side-effect-free");
        let completed = admitted(worker_result(&worker("admit", "queued-crash", None, &cwd).await));
        assert_succeeded(&completed);
        assert_eq!(history(&cwd, &sid).await.iter().filter(|item| item.text_content() == ENVELOPE).count(), 1);
        let terminal = admitted(worker_result(&worker("admit", "queued-crash", None, &cwd).await));
        assert_eq!(terminal.revision, completed.revision);
        assert_succeeded(&terminal);
        assert_eq!(main_requests(&server, SESSION).len(), before + 1);
    });
}

#[test]
fn process_restart_after_projection_keeps_interrupted_input_unreplayable() {
    run_agent_test(|cwd, server| async move {
        let sid = seed(&cwd).await;
        let crashed = worker("admit", "projection-crash", Some("input_projected"), &cwd).await;
        assert_eq!(crashed.status.code(), Some(86));
        let observed = admitted(worker_result(&worker("observe", "projection-crash", None, &cwd).await));
        assert_eq!(observed.revision, 3);
        assert!(matches!(observed.progress, DeliveryProgress::Interrupted { .. }));
        let retried = admitted(worker_result(&worker("retry", "projection-crash", None, &cwd).await));
        assert_eq!(retried.revision, observed.revision);
        assert_eq!(retried.progress, observed.progress);
        assert_eq!(history(&cwd, &sid).await.iter().filter(|item| item.text_content() == ENVELOPE).count(), 1);
        assert!(main_requests(&server, SESSION).is_empty(), "uncertain execution must never become a new sampler turn");
    });
}

#[test]
fn process_restart_after_terminal_commit_recovers_success_without_new_execution() {
    run_agent_test(|cwd, server| async move {
        let sid = seed(&cwd).await;
        let crashed = worker("admit", "terminal-crash", Some("receiver_terminal"), &cwd).await;
        assert_eq!(crashed.status.code(), Some(86));
        let observed = admitted(worker_result(&worker("observe", "terminal-crash", None, &cwd).await));
        assert_succeeded(&observed);
        assert_eq!(observed.revision, 4);
        assert_succeeded(&admitted(worker_result(&worker("admit", "terminal-crash", None, &cwd).await)));
        assert_eq!(history(&cwd, &sid).await.iter().filter(|item| item.text_content() == ENVELOPE).count(), 1);
        assert_eq!(main_requests(&server, SESSION).len(), 1);
    });
}

#[test]
fn completed_and_errored_children_recover_into_a_restarted_idle_parent_sampler() {
    for failed in [false, true] {
        for boundary in ["producer_prepared", "producer_published"] {
            run_agent_test(move |cwd, server| async move {
                let sid = seed(&cwd).await;
                let initial = server.expect_response("process parent spawns child", InferenceRequestMatcher::foreground(InferenceEndpoint::ChatCompletions).for_session(SESSION),
                    tool_reply("spawn_subagent", json!({"description":"process child","prompt":"real process child prompt","subagent_type":"general-purpose","background":true,"task_id":CHILD})));
                let marker = if failed { ERROR } else { RESULT };
                let reply = if failed { ScriptedResponse::json(400, json!({"error":{"message":ERROR,"type":"invalid_request_error"}})) } else { text_reply(RESULT) };
                let child = server.expect_response("process actual child outcome", InferenceRequestMatcher::foreground(InferenceEndpoint::ChatCompletions).for_session(CHILD), reply);
                let crashed = worker("spawn", "unused", Some(boundary), &cwd).await;
                assert_eq!(crashed.status.code(), Some(86));
                let mut recovered_sample = server.expect_response_blocked("restarted parent receives child outcome", InferenceRequestMatcher::foreground_containing(InferenceEndpoint::ChatCompletions, marker).for_session(SESSION), text_reply("recovered child result consumed"));
                let resumed = worker("recover", "unused", None, &cwd); tokio::pin!(resumed);
                tokio::select! { _ = recovered_sample.wait_blocked() => {}, output = &mut resumed => panic!("restart exited before actual parent sample: {:?}", output.status) }
                let requests = main_requests(&server, SESSION);
                let body = requests.iter().find(|body| body.to_string().contains(marker)).expect("restart reaches actual parent sampler").to_string();
                assert_eq!(body.matches(marker).count(), 1);
                recovered_sample.release();
                let output = resumed.await;
                assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
                assert_eq!(history(&cwd, &sid).await.iter().filter(|item| item.text_content().contains(marker)).count(), 1);
                initial.assert_satisfied(); child.assert_satisfied(); recovered_sample.assert_satisfied();
            });
        }
    }
}
