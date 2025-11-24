use openworkers_runtime_jscore::{run_event_loop, Runtime};
use std::time::Duration;

#[tokio::main]
async fn main() {
    // Initialize logger
    env_logger::init();

    log::info!("OpenWorkers JSCore Runtime with setTimeout");

    // Create runtime and event loop
    let (mut runtime, scheduler_rx, callback_tx) = Runtime::new();

    // Spawn the background event loop
    let event_loop_handle = tokio::spawn(async move {
        run_event_loop(scheduler_rx, callback_tx).await;
    });

    // Execute JavaScript with all timer functions
    let script = r#"
        console.log("=== Testing all timer functions ===");

        const start = +Date.now();
        const diff = () => `+${(+Date.now()) - start}ms`;

        // Test 1: setTimeout
        console.log("\n1. Testing setTimeout...");
        setTimeout(() => {
            console.log("  setTimeout fired after 200ms", diff());
        }, 200);

        // Test 2: setInterval
        console.log("\n2. Testing setInterval...");
        let count = 0;
        const intervalId = setInterval(() => {
            count++;
            console.log("  setInterval tick", count, diff());
            if (count >= 3) {
                clearInterval(intervalId);
                console.log("  Interval cleared after 3 ticks");
            }
        }, 150);

        // Test 3: clearTimeout
        console.log("\n3. Testing clearTimeout...");
        const timeoutId = setTimeout(() => {
            console.log("  This should NOT print (cleared)");
        }, 100);
        clearTimeout(timeoutId);
        console.log("  Timeout cleared immediately");

        // Test 4: Multiple timers
        console.log("\n4. Testing multiple timers...");
        setTimeout(() => console.log("  Timer A", diff()), 300);
        setTimeout(() => console.log("  Timer B", diff()), 100);
        setTimeout(() => console.log("  Timer C", diff()), 400);

        console.log("\n=== All timers scheduled! ===\n");
    "#;

    match runtime.evaluate(script) {
        Ok(_) => {
            log::info!("Script executed successfully");
        }
        Err(e) => {
            if let Ok(err_str) = e.to_js_string(&runtime.context) {
                eprintln!("Script execution failed: {}", err_str);
            } else {
                eprintln!("Script execution failed with unknown error");
            }
        }
    }

    // Process callbacks for 1 second
    for _ in 0..20 {
        runtime.process_callbacks();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    log::info!("Runtime completed");

    // Cleanup: runtime will be dropped, sending shutdown message
    drop(runtime);

    // Wait for event loop to finish
    let _ = tokio::time::timeout(Duration::from_secs(1), event_loop_handle).await;
}
