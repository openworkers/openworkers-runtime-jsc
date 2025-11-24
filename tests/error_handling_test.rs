mod common;

use common::TestRunner;

#[tokio::test]
async fn test_js_syntax_error() {
    let mut runner = TestRunner::new();

    let script = r#"
        this is not valid javascript
    "#;

    let result = runner.execute(script);
    assert!(result.is_err(), "Syntax error should be caught");

    runner.shutdown().await;
}

#[tokio::test]
async fn test_undefined_variable() {
    let mut runner = TestRunner::new();

    let script = r#"
        nonExistentVariable.foo();
    "#;

    let result = runner.execute(script);
    assert!(result.is_err(), "Undefined variable should error");

    runner.shutdown().await;
}

#[tokio::test]
async fn test_settimeout_with_missing_args() {
    let mut runner = TestRunner::new();

    // No arguments - should error
    let script = r#"
        try {
            setTimeout();
        } catch (e) {
            globalThis.caughtError = true;
        }
    "#;

    runner.execute(script).expect("Script should execute");

    // Check that error was caught
    let check = r#"globalThis.caughtError === true"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            assert!(
                result.to_bool(&runner.runtime.context),
                "setTimeout with no args should throw error"
            );
        }
        Err(_) => {} // That's also acceptable
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_error_in_timeout_callback() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.beforeError = true;
        setTimeout(() => {
            throw new Error("Test error");
        }, 50);
        globalThis.afterTimeout = true;
    "#;

    // Script execution should succeed
    runner.execute(script).expect("Script setup should work");

    // Wait for timeout to execute (and fail)
    runner
        .process_for(std::time::Duration::from_millis(100))
        .await;

    // Check that script executed before error
    let check = r#"globalThis.beforeError && globalThis.afterTimeout"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            assert!(
                result.to_bool(&runner.runtime.context),
                "Script should execute before timeout error"
            );
        }
        Err(_) => panic!("Failed to check state"),
    }

    runner.shutdown().await;
}
