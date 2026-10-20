//! Loads LSP config from disk and asks the server this test binary starts.

use xai_grok_tools::implementations::lsp::{
    config::load_servers, start_lsp_backend, LspBackend, LspOperation, LspToolInput,
};

#[tokio::test]
async fn configured_server_answers_and_missing_config_attaches_nothing() {
    let home = tempfile::tempdir().expect("home");
    // `std::env::set_var` is unsafe on the pinned nightly.
    unsafe { std::env::set_var("GROK_HOME", home.path()) };

    let empty = tempfile::tempdir().expect("empty");
    let none = start_lsp_backend(load_servers(empty.path()), empty.path().to_path_buf(), None);
    assert!(none.is_none());

    let workspace = tempfile::tempdir().expect("workspace");
    let config_dir = workspace.path().join(".grok");
    std::fs::create_dir_all(&config_dir).expect("mkdir");
    let fixture = env!("CARGO_BIN_EXE_lsp_fixture");
    let lsp_json = serde_json::json!({
        "fixture": {
            "command": fixture,
            "extensions": { ".rs": "rust" },
            "startupTimeout": 10000
        }
    });
    std::fs::write(
        config_dir.join("lsp.json"),
        serde_json::to_string(&lsp_json).expect("json"),
    )
    .expect("write lsp.json");

    let servers = load_servers(workspace.path());
    assert!(servers.contains_key("fixture"));
    let backend = start_lsp_backend(servers, workspace.path().to_path_buf(), None)
        .expect("session attaches the configured server");
    let source = workspace.path().join("lib.rs");
    std::fs::write(&source, "fn fixture() {}\n").expect("source");
    let answer = backend
        .dispatch(&LspToolInput {
            operation: LspOperation::Hover,
            file_path: Some(source.to_string_lossy().into_owned()),
            line: Some(0),
            character: Some(0),
            query: None,
        })
        .await;
    assert!(!answer.is_error, "{}", answer.text);
    assert!(
        answer.text.contains("fixture-hover-token"),
        "{}",
        answer.text
    );
}
