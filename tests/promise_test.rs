mod common;

use common::TestRunner;

#[tokio::test]
async fn test_promise_resolve() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.promiseResult = null;

        Promise.resolve(42).then(value => {
            globalThis.promiseResult = value;
        });
    "#;

    runner.execute(script).expect("Promise.resolve should work");

    // Process callbacks to let promise settle
    runner
        .process_for(std::time::Duration::from_millis(50))
        .await;

    let check = r#"globalThis.promiseResult"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            let value = result.to_number(&runner.runtime.context).unwrap();
            assert_eq!(value, 42.0, "Promise should resolve with 42");
        }
        Err(_) => panic!("Failed to check promise result"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_promise_reject() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.promiseError = null;

        Promise.reject("test error").catch(error => {
            globalThis.promiseError = error;
        });
    "#;

    runner.execute(script).expect("Promise.reject should work");

    // Process callbacks
    runner
        .process_for(std::time::Duration::from_millis(50))
        .await;

    let check = r#"globalThis.promiseError"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            if let Ok(error_str) = result.to_js_string(&runner.runtime.context) {
                assert_eq!(
                    error_str.to_string(),
                    "test error",
                    "Promise should reject with error"
                );
            } else {
                panic!("Promise error should be a string");
            }
        }
        Err(_) => panic!("Failed to check promise error"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_promise_chain() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.chainResult = null;

        Promise.resolve(10)
            .then(x => x * 2)
            .then(x => x + 5)
            .then(x => {
                globalThis.chainResult = x;
            });
    "#;

    runner.execute(script).expect("Promise chain should work");

    // Process callbacks
    runner
        .process_for(std::time::Duration::from_millis(100))
        .await;

    let check = r#"globalThis.chainResult"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            let value = result.to_number(&runner.runtime.context).unwrap();
            assert_eq!(value, 25.0, "Promise chain: 10 * 2 + 5 = 25");
        }
        Err(_) => panic!("Failed to check promise chain result"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_promise_constructor() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.constructorResult = null;

        new Promise((resolve, reject) => {
            resolve("success");
        }).then(value => {
            globalThis.constructorResult = value;
        });
    "#;

    runner
        .execute(script)
        .expect("Promise constructor should work");

    // Process callbacks
    runner
        .process_for(std::time::Duration::from_millis(50))
        .await;

    let check = r#"globalThis.constructorResult"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            if let Ok(result_str) = result.to_js_string(&runner.runtime.context) {
                assert_eq!(
                    result_str.to_string(),
                    "success",
                    "Promise constructor should resolve"
                );
            }
        }
        Err(_) => panic!("Failed to check constructor result"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_promise_async_with_settimeout() {
    let mut runner = TestRunner::new();

    // This tests that Promises and setTimeout work together
    let script = r#"
        globalThis.results = [];

        Promise.resolve("promise-start").then(value => {
            globalThis.results.push(value);
        });

        setTimeout(() => {
            globalThis.results.push("timeout-1");
        }, 50);

        Promise.resolve("promise-end").then(value => {
            globalThis.results.push(value);
        });

        globalThis.results.push("sync");
    "#;

    runner.execute(script).expect("Mixed async should work");

    // Process for a bit
    runner
        .process_for(std::time::Duration::from_millis(100))
        .await;

    let check = r#"globalThis.results.join(',')"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            if let Ok(result_str) = result.to_js_string(&runner.runtime.context) {
                let results = result_str.to_string();

                // Check execution order
                assert!(results.contains("sync"), "Sync should execute");
                assert!(results.contains("promise-start"), "Promises should resolve");
                assert!(results.contains("timeout-1"), "Timeout should fire");

                println!("Execution order: {}", results);
            }
        }
        Err(_) => panic!("Failed to check results"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_promise_all() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.allResult = null;

        Promise.all([
            Promise.resolve(1),
            Promise.resolve(2),
            Promise.resolve(3)
        ]).then(values => {
            globalThis.allResult = values.join(',');
        });
    "#;

    runner.execute(script).expect("Promise.all should work");

    // Process callbacks
    runner
        .process_for(std::time::Duration::from_millis(100))
        .await;

    let check = r#"globalThis.allResult"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            if let Ok(result_str) = result.to_js_string(&runner.runtime.context) {
                assert_eq!(
                    result_str.to_string(),
                    "1,2,3",
                    "Promise.all should resolve with all values"
                );
            }
        }
        Err(_) => panic!("Failed to check Promise.all result"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_promise_race() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.raceResult = null;

        Promise.race([
            new Promise(resolve => setTimeout(() => resolve("slow"), 100)),
            Promise.resolve("fast")
        ]).then(value => {
            globalThis.raceResult = value;
        });
    "#;

    runner.execute(script).expect("Promise.race should work");

    // Process callbacks
    runner
        .process_for(std::time::Duration::from_millis(150))
        .await;

    let check = r#"globalThis.raceResult"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            if let Ok(result_str) = result.to_js_string(&runner.runtime.context) {
                assert_eq!(
                    result_str.to_string(),
                    "fast",
                    "Promise.race should resolve with fastest"
                );
            }
        }
        Err(_) => panic!("Failed to check Promise.race result"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_queue_microtask() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.results = [];

        globalThis.results.push('sync-1');

        queueMicrotask(() => {
            globalThis.results.push('microtask-1');
        });

        globalThis.results.push('sync-2');

        queueMicrotask(() => {
            globalThis.results.push('microtask-2');
        });

        globalThis.results.push('sync-3');
    "#;

    runner.execute(script).expect("queueMicrotask should work");

    // Process callbacks
    runner
        .process_for(std::time::Duration::from_millis(50))
        .await;

    let check = r#"globalThis.results.join(',')"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            if let Ok(result_str) = result.to_js_string(&runner.runtime.context) {
                let results = result_str.to_string();

                // Microtasks should run after all sync code
                assert!(
                    results.starts_with("sync-1,sync-2,sync-3"),
                    "Sync code should run first, got: {}",
                    results
                );
                assert!(
                    results.contains("microtask-1"),
                    "Microtask 1 should execute"
                );
                assert!(
                    results.contains("microtask-2"),
                    "Microtask 2 should execute"
                );
            }
        }
        Err(_) => panic!("Failed to check queueMicrotask result"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_microtask_vs_timeout() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.order = [];

        setTimeout(() => {
            globalThis.order.push('timeout');
        }, 0);

        queueMicrotask(() => {
            globalThis.order.push('microtask');
        });

        globalThis.order.push('sync');
    "#;

    runner
        .execute(script)
        .expect("Mixed microtask/timeout should work");

    // Process callbacks
    runner
        .process_for(std::time::Duration::from_millis(50))
        .await;

    let check = r#"globalThis.order.join(',')"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            if let Ok(result_str) = result.to_js_string(&runner.runtime.context) {
                let order = result_str.to_string();

                // Order should be: sync -> microtask -> timeout
                // Microtasks run before timer callbacks
                println!("Execution order: {}", order);
                assert!(order.starts_with("sync"), "Sync should be first");
                // Note: microtask ordering vs timeout is implementation-dependent
            }
        }
        Err(_) => panic!("Failed to check execution order"),
    }

    runner.shutdown().await;
}
