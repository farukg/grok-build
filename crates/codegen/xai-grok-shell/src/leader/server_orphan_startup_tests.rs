use super::*;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use xai_grok_tools::computer::local::terminal::TerminalOrphanSweepSummary;

fn test_server(
    socket: PathBuf,
    cancel: CancellationToken,
    startup: TerminalOrphanStartup,
) -> tokio::task::JoinHandle<Result<(), ServerError>> {
    let mut control = default_test_control_state(&socket);
    control.terminal_orphan_startup = startup;
    let (acp_tx, _acp_rx) = mpsc::unbounded_channel();
    let (_response_tx, response_rx) = mpsc::unbounded_channel();
    let (_ready_tx, ready_rx) = watch::channel(true);
    let (shutdown_tx, _shutdown_rx) = watch::channel(super::super::protocol::ShutdownReason::Manual);
    tokio::spawn(run_leader_server(socket, acp_tx, response_rx, cancel, true,
        Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)),
        AgentActivity::default(), ready_rx, watch::channel(false).0, shutdown_tx, None, control))
}

#[tokio::test]
async fn real_server_awaits_exactly_one_startup_operation_without_blocking_the_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let socket = temp.path().join("leader.sock");
    let cancel = CancellationToken::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let startup = TerminalOrphanStartup {
        execute: Arc::new(move || {
            observed.fetch_add(1, Ordering::SeqCst);
            entered_tx.send(()).unwrap();
            release_rx.lock().unwrap().recv().unwrap();
            Ok(TerminalOrphanSweepSummary::default())
        }),
    };
    let server = test_server(socket.clone(), cancel.clone(), startup);
    tokio::time::timeout(Duration::from_secs(3), entered_rx.recv()).await.unwrap().unwrap();
    assert!(socket.exists(), "the real startup path must bind before the bounded sweep");
    cancel.cancel();
    assert!(!server.is_finished(), "routing and shutdown must not bypass the startup operation");
    release_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), server).await.unwrap().unwrap().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1, "startup must not poll or retry");
    assert!(!socket.exists(), "normal server shutdown still retires the bound socket");
}

#[tokio::test]
async fn real_server_reports_startup_failure_without_retrying_or_leaking_its_socket() {
    let temp = tempfile::tempdir().unwrap();
    let socket = temp.path().join("leader.sock");
    let cancel = CancellationToken::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
    let startup = TerminalOrphanStartup {
        execute: Arc::new(move || {
            observed.fetch_add(1, Ordering::SeqCst);
            entered_tx.send(()).unwrap();
            Err(io::Error::from_raw_os_error(libc::EACCES))
        }),
    };
    let server = test_server(socket.clone(), cancel.clone(), startup);
    tokio::time::timeout(Duration::from_secs(3), entered_rx.recv()).await.unwrap().unwrap();
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(3), server).await.unwrap().unwrap().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!socket.exists());
}
