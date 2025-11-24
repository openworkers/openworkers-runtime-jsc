mod common;

use common::TestRunner;
use std::time::Duration;

#[tokio::test]
async fn test_comprehensive_timer_scenario() {
    let mut runner = TestRunner::new();

    // Complex scenario mixing all timer features
    let script = r#"
        globalThis.events = [];

        // Immediate timeout
        setTimeout(() => {
            globalThis.events.push('timeout-1');
        }, 10);

        // Delayed timeout
        setTimeout(() => {
            globalThis.events.push('timeout-2');
        }, 100);

        // Interval that auto-clears
        let intervalCount = 0;
        const intervalId = setInterval(() => {
            intervalCount++;
            globalThis.events.push('interval-' + intervalCount);
            if (intervalCount >= 3) {
                clearInterval(intervalId);
            }
        }, 50);

        // Timeout that gets cleared
        const clearedId = setTimeout(() => {
            globalThis.events.push('SHOULD_NOT_APPEAR');
        }, 200);
        clearTimeout(clearedId);

        globalThis.events.push('script-end');
    "#;

    runner.execute(script).expect("Script should execute");

    // Process for enough time
    runner.process_for(Duration::from_millis(250)).await;

    // Verify the event sequence
    let check = r#"globalThis.events.join(',')"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            if let Ok(events_str) = result.to_js_string(&runner.runtime.context) {
                let events = events_str.to_string();

                // Should contain all expected events
                assert!(events.contains("script-end"), "Script should complete");
                assert!(events.contains("timeout-1"), "First timeout should fire");
                assert!(events.contains("timeout-2"), "Second timeout should fire");
                assert!(events.contains("interval-1"), "Interval should tick 1");
                assert!(events.contains("interval-2"), "Interval should tick 2");
                assert!(events.contains("interval-3"), "Interval should tick 3");

                // Should NOT contain cleared timeout
                assert!(
                    !events.contains("SHOULD_NOT_APPEAR"),
                    "Cleared timeout should not execute"
                );

                // Script-end should be first (synchronous)
                assert!(
                    events.starts_with("script-end"),
                    "Synchronous code should execute first"
                );
            }
        }
        Err(_) => panic!("Failed to check events"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_date_now_works() {
    let mut runner = TestRunner::new();

    let script = r#"
        const start = Date.now();
        globalThis.hasDateNow = typeof start === 'number' && start > 0;
    "#;

    runner.execute(script).expect("Date.now should work");

    let check = r#"globalThis.hasDateNow"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            assert!(
                result.to_bool(&runner.runtime.context),
                "Date.now() should return a valid timestamp"
            );
        }
        Err(_) => panic!("Failed to check Date.now"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_math_operations() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.result = Math.sqrt(16) + Math.max(5, 10) - Math.min(2, 8);
    "#;

    runner.execute(script).expect("Math operations should work");

    let check = r#"globalThis.result"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            let value = result.to_number(&runner.runtime.context).unwrap();
            assert_eq!(
                value, 12.0,
                "Math: sqrt(16)=4 + max(5,10)=10 - min(2,8)=2 = 12"
            );
        }
        Err(_) => panic!("Failed to check Math result"),
    }

    runner.shutdown().await;
}
