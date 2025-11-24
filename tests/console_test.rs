mod common;

use common::TestRunner;

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
