use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new(version: u8) -> Self {
        let path = std::env::temp_dir().join(format!(
            "grok-audit-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let session = path.join("copy/sessions/g1/session");
        std::fs::create_dir_all(&session).unwrap();
        let summary = format!(
            r#"{{"info":{{"id":"session","cwd":"/workspace"}},"session_summary":"private-sentinel","created_at":"2026-10-05T00:00:00Z","updated_at":"2026-10-05T00:00:00Z","num_messages":1,"current_model_id":"grok-4","chat_format_version":{version}}}"#
        );
        std::fs::write(session.join("summary.json"), summary).unwrap();
        std::fs::write(
            session.join("chat_history.jsonl"),
            if version == 0 {
                r#"{"role":"user","content":"private-sentinel"}"#
            } else {
                r#"{"type":"user","content":[{"type":"text","text":"private-sentinel"}]}"#
            },
        )
        .unwrap();
        std::fs::write(session.join("events.jsonl"), "{\"ts\":\"2026-10-05T00:00:00Z\",\"type\":\"tool_completed\",\"tool_name\":\"read_file\",\"duration_ms\":1,\"outcome\":\"success\"}\n").unwrap();
        std::fs::create_dir_all(path.join("home/.grok")).unwrap();
        std::fs::write(path.join("home/.grok/config.toml"), "poison = [").unwrap();
        Self(path)
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_with_sampling(args, Some("invalid-sampling-sentinel"))
    }

    fn run_with_sampling(&self, args: &[&str], sampling: Option<&str>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xai-grok-pager"));
        command
            .args(args)
            .env_clear()
            .env("HOME", self.0.join("home"))
            .env("GROK_HOME", self.0.join("home/.grok"))
            .env("PATH", "/nonexistent")
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("HTTPS_PROXY", "http://127.0.0.1:1");
        if let Some(sampling) = sampling {
            command.env("GROK_LOG_SAMPLING", sampling);
        }
        command.output().unwrap()
    }

    fn audit(&self) -> Output {
        self.run(&[
            "session-audit",
            "tree",
            "--sessions-root",
            self.0.join("copy/sessions").to_str().unwrap(),
        ])
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug, PartialEq, Eq)]
enum EntryContents {
    Directory,
    File(Vec<u8>),
}

#[derive(Debug, PartialEq, Eq)]
struct EntrySnapshot {
    contents: EntryContents,
    size: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, u32, i64, i64),
}

fn inventory(root: &Path) -> BTreeMap<PathBuf, EntrySnapshot> {
    fn visit(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, EntrySnapshot>) {
        let metadata = std::fs::symlink_metadata(path).unwrap();
        let contents = if metadata.is_dir() {
            for entry in std::fs::read_dir(path).unwrap() {
                visit(root, &entry.unwrap().path(), files);
            }
            EntryContents::Directory
        } else {
            EntryContents::File(std::fs::read(path).unwrap())
        };
        files.insert(
            path.strip_prefix(root).unwrap().to_path_buf(),
            EntrySnapshot {
                contents,
                size: metadata.len(),
                modified: metadata.modified().unwrap(),
                #[cfg(unix)]
                identity: {
                    use std::os::unix::fs::MetadataExt;
                    (
                        metadata.dev(),
                        metadata.ino(),
                        metadata.mode(),
                        metadata.ctime(),
                        metadata.ctime_nsec(),
                    )
                },
            },
        );
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

#[test]
fn capability_and_legacy_current_audits_return_before_configuration_or_runtime_startup() {
    for version in [0, 1] {
        let fixture = Fixture::new(version);
        let before = inventory(&fixture.0);
        let capability = fixture.run(&["session-audit", "capabilities"]);
        assert!(capability.status.success());
        assert!(capability.stderr.is_empty());
        let text = String::from_utf8(capability.stdout).unwrap();
        assert!(text.contains("grok-native-session-audit-v1"));
        assert!(text.contains("build = "));
        let result = fixture.audit();
        assert!(result.status.success(), "{:?}", result.stderr);
        assert!(result.stderr.is_empty());
        let text = String::from_utf8(result.stdout).unwrap();
        assert!(text.contains("result = \"supported_no_transform\""));
        assert!(text.contains("sha256 = "));
        assert!(!text.contains("private-sentinel"));
        assert_eq!(inventory(&fixture.0), before);
    }
}

#[test]
fn malformed_and_unknown_native_entries_refuse_without_repair_or_payload_diagnostics() {
    for (filename, record) in [
        (
            "summary.json",
            r#"{"info":42,"private":"private-sentinel"}"#,
        ),
        (
            "chat_history.jsonl",
            r#"{"type":"future","private":"private-sentinel"}"#,
        ),
        (
            "chat_history.jsonl",
            r#"{"type":"assistant","content":"kept","raw_output":[{"type":"future","private":"private-sentinel"}]}"#,
        ),
        (
            "events.jsonl",
            r#"{"ts":"t","type":"tool_completed","duration_ms":"private-sentinel"}"#,
        ),
    ] {
        let fixture = Fixture::new(1);
        std::fs::write(
            fixture.0.join("copy/sessions/g1/session").join(filename),
            record,
        )
        .unwrap();
        let before = inventory(&fixture.0);
        let result = fixture.audit();
        assert_eq!(result.status.code(), Some(2));
        assert!(result.stderr.is_empty());
        let text = String::from_utf8(result.stdout).unwrap();
        assert!(text.contains("result = \"refused\""));
        assert!(text.contains(&format!("path = \"g1/session/{filename}\"")));
        assert!(!text.contains("private-sentinel"));
        assert_eq!(inventory(&fixture.0), before);
    }
}

#[test]
fn prefixed_audit_uses_existing_cli_grammar_without_reading_logging_environment() {
    let fixture = Fixture::new(1);
    let before = inventory(&fixture.0);
    let root = fixture.0.join("copy/sessions");
    for args in [
        vec!["--cwd", "/nonexistent", "session-audit", "capabilities"],
        vec![
            "--log-sampling",
            "session-audit",
            "tree",
            "--sessions-root",
            root.to_str().unwrap(),
        ],
        vec!["--sandbox", "off", "session-audit", "tree", "--help"],
    ] {
        let clean = fixture.run_with_sampling(&args, None);
        let poisoned = fixture.run(&args);
        assert!(clean.status.success());
        assert!(poisoned.status.success());
        assert_eq!(poisoned.stdout, clean.stdout);
        assert_eq!(poisoned.stderr, clean.stderr);
        assert!(!String::from_utf8_lossy(&poisoned.stdout).contains("invalid-sampling-sentinel"));
    }
    assert_eq!(inventory(&fixture.0), before);
}

#[test]
fn normal_cli_retains_environment_bound_sampling_validation() {
    let fixture = Fixture::new(1);
    let before = inventory(&fixture.0);
    let clean = fixture.run_with_sampling(&["--version"], None);
    let poisoned = fixture.run(&["--version"]);
    assert!(clean.status.success());
    assert_eq!(poisoned.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&poisoned.stderr).contains("--log-sampling"));
    let value_not_command = fixture.run(&["--cwd", "session-audit", "--version"]);
    assert_eq!(value_not_command.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&value_not_command.stderr).contains("--log-sampling"));
    for sampling in ["true", "false"] {
        let valid = fixture.run_with_sampling(&["--version"], Some(sampling));
        assert!(valid.status.success(), "{:?}", valid.stderr);
        assert_eq!(valid.stdout, clean.stdout);
        assert_eq!(valid.stderr, clean.stderr);
    }
    assert_eq!(inventory(&fixture.0), before);
}

#[test]
fn prefixed_audit_misuse_does_not_fall_back_to_environment_bound_parsing() {
    let fixture = Fixture::new(1);
    let before = inventory(&fixture.0);
    let args = ["--cwd", "/nonexistent", "session-audit", "tree"];
    let clean = fixture.run_with_sampling(&args, None);
    let poisoned = fixture.run(&args);
    assert_eq!(clean.status.code(), Some(2));
    assert_eq!(poisoned.status.code(), Some(2));
    assert_eq!(poisoned.stdout, clean.stdout);
    assert_eq!(poisoned.stderr, clean.stderr);
    assert!(String::from_utf8_lossy(&poisoned.stderr).contains("--sessions-root <SESSIONS_ROOT>"));
    assert!(!String::from_utf8_lossy(&poisoned.stderr).contains("invalid-sampling-sentinel"));
    assert_eq!(inventory(&fixture.0), before);
}

#[test]
fn direct_audit_help_and_misuse_ignore_logging_environment() {
    let fixture = Fixture::new(1);
    let before = inventory(&fixture.0);
    for args in [
        vec!["session-audit", "--help"],
        vec!["session-audit", "tree", "--help"],
        vec!["session-audit", "capabilities", "--help"],
    ] {
        let clean = fixture.run_with_sampling(&args, None);
        let poisoned = fixture.run(&args);
        assert!(clean.status.success());
        assert!(poisoned.status.success());
        assert_eq!(poisoned.stdout, clean.stdout);
        assert_eq!(poisoned.stderr, clean.stderr);
        assert!(!String::from_utf8_lossy(&poisoned.stdout).contains("invalid-sampling-sentinel"));
    }
    for args in [
        vec!["session-audit", "tree"],
        vec!["session-audit", "unknown"],
    ] {
        let clean = fixture.run_with_sampling(&args, None);
        let poisoned = fixture.run(&args);
        assert_eq!(clean.status.code(), Some(2));
        assert_eq!(poisoned.status.code(), Some(2));
        assert_eq!(poisoned.stdout, clean.stdout);
        assert_eq!(poisoned.stderr, clean.stderr);
        assert!(!String::from_utf8_lossy(&poisoned.stderr).contains("invalid-sampling-sentinel"));
    }
    assert_eq!(inventory(&fixture.0), before);
}

#[test]
fn issue_budget_stops_at_first_rejected_history_diagnostic_without_positive_evidence() {
    let fixture = Fixture::new(1);
    std::fs::write(
        fixture
            .0
            .join("copy/sessions/g1/session/chat_history.jsonl"),
        "{torn\n".repeat(200),
    )
    .unwrap();
    let before = inventory(&fixture.0);
    let result = fixture.run(&[
        "session-audit",
        "tree",
        "--sessions-root",
        fixture.0.join("copy/sessions").to_str().unwrap(),
        "--max-issues",
        "2",
    ]);
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stderr.is_empty());
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(text.lines().any(|line| line == "response = \"incomplete\""));
    assert!(text.lines().any(|line| line == "budget = \"issues\""));
    assert!(text.lines().any(|line| line == "observed_issues = 3"));
    for forbidden in [
        "result =",
        "records =",
        "[[files]]",
        "files =",
        "sha256 =",
        "[[issues]]",
        "{torn",
    ] {
        assert!(
            !text.lines().any(|line| line.starts_with(forbidden)),
            "unexpected evidence: {forbidden}"
        );
    }
    assert_eq!(inventory(&fixture.0), before);
}
