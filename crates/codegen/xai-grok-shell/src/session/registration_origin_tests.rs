use super::*;

#[tokio::test(flavor = "current_thread")]
async fn registration_origin_is_rebuilt_from_actual_session_and_audience() {
    tokio::task::LocalSet::new()
        .run_until(async {
            for (audience, session_id, expected_context) in [
                (PromptAudience::Subagent, "child-session", "subagent"),
                (PromptAudience::Primary, "main-session", "main"),
            ] {
                let mut spec = test_rebuild_spec_default();
                let fields = Arc::get_mut(&mut spec).unwrap();
                fields.session_id_str = session_id.to_owned();
                fields.prompt_audience = audience;
                fields.session_env = Arc::new(HashMap::from([
                    ("GROK_SESSION_ID".to_owned(), "parent-session".to_owned()),
                    (
                        "GROK_SESSION_ORIGIN".to_owned(),
                        serde_json::json!({
                            "version": 1,
                            "runtime": "grok",
                            "session_id": "parent-session",
                            "context": "main",
                        })
                        .to_string(),
                    ),
                    ("CODEX_SESSION_ID".to_owned(), "inherited-parent".to_owned()),
                    ("SIG_PARENT_AGENT".to_owned(), "ParentName".to_owned()),
                ]));
                for _ in 0..2 {
                    let agent = spec
                        .build_agent(
                            AgentDefinition::default_grok_build(),
                            xai_grok_agent::DEFAULT_SYSTEM_PROMPT_LABEL,
                        )
                        .await
                        .unwrap();
                    let env = agent
                        .tool_bridge()
                        .read_resource::<xai_grok_tools::types::resources::SessionEnv>()
                        .await
                        .unwrap();
                    assert_eq!(env.0.get("GROK_SESSION_ID").unwrap(), session_id);
                    let origin: serde_json::Value =
                        serde_json::from_str(env.0.get("GROK_SESSION_ORIGIN").unwrap()).unwrap();
                    assert_eq!(
                        origin,
                        serde_json::json!({
                            "version": 1,
                            "runtime": "grok",
                            "session_id": session_id,
                            "context": expected_context,
                        })
                    );
                }
            }
        })
        .await;
}
