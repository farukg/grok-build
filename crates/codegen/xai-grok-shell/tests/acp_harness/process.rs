use std::cell::Cell;
use tokio::io::AsyncReadExt as _;
use tokio::process::Command;
use xai_grok_test_support::process::{
    TestOutput, TestProcess as OwnedTestProcess, TestProcessConfig,
};
use xai_grok_test_support::sandbox::TestSandbox;

const ISOLATED_TEST: &str = "GROK_ACP_HARNESS_ISOLATED_TEST";
const ISOLATED_INVOCATION: &str = "GROK_ACP_HARNESS_ISOLATED_INVOCATION";

thread_local! {
    static NEXT_INVOCATION: Cell<usize> = const { Cell::new(0) };
}

pub(super) enum TestProcess {
    Driver,
    Isolated,
}

pub(super) fn enter() -> TestProcess {
    let thread = std::thread::current();
    let name = thread.name().expect("libtest names its test thread");
    let invocation = NEXT_INVOCATION.with(|next| {
        let invocation = next.get();
        next.set(invocation + 1);
        invocation
    });
    let marker = format!("ACP_HARNESS_ENTERED={name}/{invocation}");
    if std::env::var(ISOLATED_TEST).as_deref() == Ok(name) {
        let selected = std::env::var(ISOLATED_INVOCATION)
            .expect("isolated harness invocation")
            .parse::<usize>()
            .expect("numeric harness invocation");
        if invocation == selected {
            println!("\n{marker}");
            return TestProcess::Isolated;
        }
        return TestProcess::Driver;
    }
    // GROK_HOME is cached for the process, including across scenarios in one test.
    let sandbox = TestSandbox::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("isolated ACP process runtime");
    let (status, stdout, stderr) = runtime.block_on(async {
        let mut command = Command::new(std::env::current_exe().expect("test executable"));
        command.args(["--exact", name, "--nocapture", "--test-threads=1"]);
        let mut process = OwnedTestProcess::spawn(
            command,
            &sandbox,
            TestProcessConfig::new()
                .label(format!("ACP {name}/{invocation}"))
                .env(ISOLATED_TEST, name)
                .env(ISOLATED_INVOCATION, invocation.to_string())
                .stdout(TestOutput::Piped)
                .stderr(TestOutput::Piped),
        )
        .expect("isolated ACP harness process");
        let mut stdout = process.take_stdout().expect("isolated stdout pipe");
        let mut stderr = process.take_stderr().expect("isolated stderr pipe");
        let mut stdout_bytes = Vec::new();
        let mut stderr_bytes = Vec::new();
        let output = tokio::time::timeout(super::RPC_TIMEOUT, async {
            tokio::try_join!(
                process.wait_with_deadline(super::RPC_TIMEOUT),
                stdout.read_to_end(&mut stdout_bytes),
                stderr.read_to_end(&mut stderr_bytes),
            )
        })
        .await;
        let cleanup = process.kill().await;
        assert!(
            cleanup.is_ok(),
            "isolated ACP cleanup failed: {cleanup:?}; {}",
            process.diagnostic_summary(),
        );
        let (status, _, _) = output
            .unwrap_or_else(|_| {
                panic!(
                    "isolated ACP deadline expired: {}",
                    process.diagnostic_summary()
                )
            })
            .unwrap_or_else(|error| {
                panic!(
                    "isolated ACP output failed: {error}; {}",
                    process.diagnostic_summary(),
                )
            });
        let status = status.expect("isolated ACP test exited before its deadline");
        (status, stdout_bytes, stderr_bytes)
    });
    let stdout = String::from_utf8_lossy(&stdout);
    assert!(
        status.success(),
        "isolated ACP test {name}/{invocation} failed ({status}):\n{}\n{}",
        stdout,
        String::from_utf8_lossy(&stderr),
    );
    assert!(
        stdout.lines().any(|line| line == marker),
        "isolated ACP test did not enter harness {name}/{invocation}",
    );
    TestProcess::Driver
}
