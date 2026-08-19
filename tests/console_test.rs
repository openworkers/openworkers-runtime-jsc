mod common;

use common::TestRunner;
use openworkers_runtime_jsc::runtime::bindings::setup_console;
use openworkers_runtime_jsc::{LogLevel, OperationsHandler, Runtime, Script, Worker};
use std::sync::Arc;
use std::sync::Mutex;

#[tokio::test]
async fn test_console_log_basic() {
    let mut runner = TestRunner::new();

    let script = r#"
        console.log("Hello, World!");
    "#;

    runner.execute(script).expect("Script should execute");
    runner.shutdown().await;
}

#[tokio::test]
async fn test_console_log_multiple_args() {
    let mut runner = TestRunner::new();

    let script = r#"
        console.log("Number:", 42, "String:", "test", "Boolean:", true);
    "#;

    runner.execute(script).expect("Script should execute");
    runner.shutdown().await;
}

#[tokio::test]
async fn test_console_log_objects() {
    let mut runner = TestRunner::new();

    let script = r#"
        console.log({ key: "value", number: 123 });
        console.log([1, 2, 3]);
    "#;

    runner.execute(script).expect("Script should execute");
    runner.shutdown().await;
}

#[tokio::test]
async fn test_console_log_special_values() {
    let mut runner = TestRunner::new();

    let script = r#"
        console.log("null:", null);
        console.log("undefined:", undefined);
        console.log("NaN:", NaN);
    "#;

    runner.execute(script).expect("Script should execute");
    runner.shutdown().await;
}

/// Records what console sent, and echoes it on stderr for the child below
#[derive(Default)]
struct RecordingOps {
    logs: Mutex<Vec<(LogLevel, String)>>,
}

impl OperationsHandler for RecordingOps {
    fn handle_log(&self, level: LogLevel, message: String) {
        eprintln!("[{}] {}", level, message);

        self.logs.lock().unwrap().push((level, message));
    }
}

#[tokio::test]
async fn console_reaches_the_ops_handler_with_levels() {
    let ops = Arc::new(RecordingOps::default());

    let script = r#"
        console.error("boom");
        console.warn("careful");
        console.log("hello");
        console.info("fyi");
        console.debug("noisy");
    "#;

    Worker::new_with_ops(Script::new(script), None, ops.clone())
        .await
        .expect("worker creation failed");

    let logs = ops.logs.lock().unwrap();

    assert_eq!(
        *logs,
        vec![
            (LogLevel::Error, "boom".to_string()),
            (LogLevel::Warn, "careful".to_string()),
            (LogLevel::Info, "hello".to_string()),
            (LogLevel::Info, "fyi".to_string()),
            (LogLevel::Debug, "noisy".to_string()),
        ]
    );
}

/// JSC claims the JSC_ prefix for its own options, so keep clear of it
const CHILD_ENV: &str = "OW_CONSOLE_STDOUT_CHILD";
const OPS_MARKER: &str = "routed-through-ops";
const FALLBACK_MARKER: &str = "printed-by-fallback";

/// libtest hides what a test prints, so the stdout check runs in a child
#[tokio::test]
async fn console_keeps_stdout_clean_when_ops_are_present() {
    if std::env::var(CHILD_ENV).is_ok() {
        log_from_child().await;
        return;
    }

    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "console_keeps_stdout_clean_when_ops_are_present",
            "--exact",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .output()
        .expect("failed to run the child test");

    let stdout = String::from_utf8_lossy(&child.stdout);
    let stderr = String::from_utf8_lossy(&child.stderr);

    assert!(
        child.status.success(),
        "child failed:\n{}\n{}",
        stdout,
        stderr
    );
    assert!(
        stderr.contains(OPS_MARKER),
        "ops handler saw nothing:\n{}",
        stderr
    );
    assert!(
        !stdout.contains(OPS_MARKER),
        "console wrote to stdout:\n{}",
        stdout
    );
    assert!(
        stdout.contains(FALLBACK_MARKER),
        "fallback wrote nothing:\n{}",
        stdout
    );
}

async fn log_from_child() {
    let script = format!(r#"console.log("{}");"#, OPS_MARKER);

    Worker::new_with_ops(
        Script::new(script.as_str()),
        None,
        Arc::new(RecordingOps::default()),
    )
    .await
    .expect("worker creation failed");

    let (mut runtime, ..) = Runtime::new();

    setup_console(&mut runtime.context, None);

    let script = format!(r#"console.log("{}");"#, FALLBACK_MARKER);

    assert!(runtime.evaluate(&script).is_ok());
}
