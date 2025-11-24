mod common;

use common::TestRunner;
use std::time::Duration;

#[tokio::test]
async fn test_settimeout_basic() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.timeoutFired = false;
        setTimeout(() => {
            globalThis.timeoutFired = true;
        }, 50);
    "#;

    runner.execute(script).expect("Script should execute");

    // Wait for timeout to fire
    runner.process_for(Duration::from_millis(100)).await;

    // Check that timeout fired
    let check = r#"globalThis.timeoutFired"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            assert!(
                result.to_bool(&runner.runtime.context),
                "Timeout should have fired"
            );
        }
        Err(_) => panic!("Failed to check timeout result"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_settimeout_with_delay() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.value = 0;
        setTimeout(() => {
            globalThis.value = 42;
        }, 100);
    "#;

    runner.execute(script).expect("Script should execute");

    // Check value before timeout
    let check_before = r#"globalThis.value"#;
    match runner.runtime.evaluate(check_before) {
        Ok(result) => {
            assert_eq!(
                result.to_number(&runner.runtime.context).unwrap(),
                0.0,
                "Value should be 0 before timeout"
            );
        }
        Err(_) => panic!("Failed to check value before"),
    }

    // Wait for timeout
    runner.process_for(Duration::from_millis(150)).await;

    // Check value after timeout
    let check_after = r#"globalThis.value"#;
    match runner.runtime.evaluate(check_after) {
        Ok(result) => {
            assert_eq!(
                result.to_number(&runner.runtime.context).unwrap(),
                42.0,
                "Value should be 42 after timeout"
            );
        }
        Err(_) => panic!("Failed to check value after"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_setinterval_basic() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.counter = 0;
        globalThis.intervalId = setInterval(() => {
            globalThis.counter++;
        }, 50);
    "#;

    runner.execute(script).expect("Script should execute");

    // Wait for ~3 ticks
    runner.process_for(Duration::from_millis(200)).await;

    // Check counter
    let check = r#"globalThis.counter"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            let count = result.to_number(&runner.runtime.context).unwrap();
            assert!(count >= 3.0, "Counter should be at least 3, got {}", count);
        }
        Err(_) => panic!("Failed to check counter"),
    }

    // Clear interval
    let clear = r#"clearInterval(globalThis.intervalId)"#;
    runner.execute(clear).expect("Clear should work");

    runner.shutdown().await;
}

#[tokio::test]
async fn test_clearinterval_stops_execution() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.counter = 0;
        const id = setInterval(() => {
            globalThis.counter++;
        }, 50);

        // Clear after 2 ticks
        setTimeout(() => {
            clearInterval(id);
        }, 120);
    "#;

    runner.execute(script).expect("Script should execute");

    // Wait for interval to run and then be cleared
    runner.process_for(Duration::from_millis(150)).await;

    let check1 = r#"globalThis.counter"#;
    let count1 = match runner.runtime.evaluate(check1) {
        Ok(result) => result.to_number(&runner.runtime.context).unwrap(),
        Err(_) => panic!("Failed to check counter"),
    };

    // Wait more time - counter should not increase
    runner.process_for(Duration::from_millis(150)).await;

    let check2 = r#"globalThis.counter"#;
    let count2 = match runner.runtime.evaluate(check2) {
        Ok(result) => result.to_number(&runner.runtime.context).unwrap(),
        Err(_) => panic!("Failed to check counter after clear"),
    };

    assert_eq!(
        count1, count2,
        "Counter should not increase after clearInterval"
    );

    runner.shutdown().await;
}

#[tokio::test]
async fn test_cleartimeout_prevents_execution() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.shouldNotRun = false;
        const id = setTimeout(() => {
            globalThis.shouldNotRun = true;
        }, 100);
        clearTimeout(id);
    "#;

    runner.execute(script).expect("Script should execute");

    // Wait past when timeout would have fired
    runner.process_for(Duration::from_millis(150)).await;

    // Check that timeout did not run
    let check = r#"globalThis.shouldNotRun"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            assert!(
                !result.to_bool(&runner.runtime.context),
                "Cleared timeout should not have executed"
            );
        }
        Err(_) => panic!("Failed to check result"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_multiple_timers_execution_order() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.order = [];

        setTimeout(() => globalThis.order.push('A'), 100);
        setTimeout(() => globalThis.order.push('B'), 50);
        setTimeout(() => globalThis.order.push('C'), 150);
    "#;

    runner.execute(script).expect("Script should execute");

    // Wait for all timers
    runner.process_for(Duration::from_millis(200)).await;

    // Check execution order (B before A before C)
    let check = r#"globalThis.order.join(',')"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            if let Ok(order_str) = result.to_js_string(&runner.runtime.context) {
                assert_eq!(
                    order_str.to_string(),
                    "B,A,C",
                    "Timers should execute in order based on delay"
                );
            } else {
                panic!("Failed to convert result to string");
            }
        }
        Err(_) => panic!("Failed to check execution order"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_nested_timers() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.result = [];

        setTimeout(() => {
            globalThis.result.push('outer');
            setTimeout(() => {
                globalThis.result.push('inner');
            }, 50);
        }, 50);
    "#;

    runner.execute(script).expect("Script should execute");

    // Wait for both timers
    runner.process_for(Duration::from_millis(150)).await;

    // Check both executed
    let check = r#"globalThis.result.join(',')"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            if let Ok(result_str) = result.to_js_string(&runner.runtime.context) {
                assert_eq!(
                    result_str.to_string(),
                    "outer,inner",
                    "Nested timers should both execute"
                );
            }
        }
        Err(_) => panic!("Failed to check nested timers"),
    }

    runner.shutdown().await;
}
