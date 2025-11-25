// Example: fetch API with promises
//
// To run: cargo run --example fetch

use openworkers_runtime_jsc::{Runtime, run_event_loop};
use std::time::Duration;

#[tokio::main]
async fn main() {
    env_logger::init();

    log::info!("Starting fetch example");

    // Create runtime and event loop
    let (mut runtime, scheduler_rx, callback_tx, stream_manager) = Runtime::new();

    // Spawn the background event loop
    let event_loop_handle = tokio::spawn(async move {
        run_event_loop(scheduler_rx, callback_tx, stream_manager).await;
    });

    // Execute JavaScript with fetch
    let script = r#"
        console.log("=== Fetch API Example ===\n");

        // Example 1: Simple GET request
        console.log("1. GET request...");
        fetch('https://echo.workers.rocks/get')
            .then(response => {
                console.log("  Status:", response.status, response.statusText);
                console.log("  OK:", response.ok);
                console.log("  Has content-type:", response.headers.has('content-type'));
                return response.json();
            })
            .then(data => {
                console.log("  Data type:", typeof data);
            })
            .catch(error => {
                console.log("  Error:", error);
            });

        // Example 2: POST with headers and body
        console.log("\n2. POST with custom headers and body...");
        fetch('https://echo.workers.rocks/post', {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
                'X-Custom-Header': 'test-value'
            },
            body: JSON.stringify({
                message: 'Hello from JSCore!',
                timestamp: Date.now()
            })
        })
        .then(response => {
            console.log("  POST Status:", response.status);
            return response.text();
        })
        .then(text => {
            console.log("  Response length:", text.length);
        })
        .catch(error => {
            console.log("  Error:", error);
        });

        // Example 3: Response headers API
        console.log("\n3. Testing headers API...");
        fetch('https://echo.workers.rocks/get')
            .then(response => {
                console.log("  Content-Type:", response.headers.get('content-type'));
                console.log("  Has Server header:", response.headers.has('server'));
                console.log("  Non-existent:", response.headers.get('x-nonexistent'));
            })
            .catch(error => {
                console.log("  Error:", error);
            });

        // Example 4: Different HTTP methods
        console.log("\n4. Testing PUT method...");
        fetch('https://echo.workers.rocks/put', {
            method: 'PUT',
            body: 'Updated data'
        })
        .then(response => {
            console.log("  PUT Status:", response.status);
        })
        .catch(error => {
            console.log("  Error:", error);
        });

        console.log("\n=== All fetches scheduled! ===\n");
    "#;

    match runtime.evaluate(script) {
        Ok(_) => {
            log::info!("Script executed successfully");
        }
        Err(e) => {
            if let Ok(err_str) = e.to_js_string(&runtime.context) {
                eprintln!("Error: {}", err_str);
            }
        }
    }

    // Process callbacks for a while to let fetches complete
    log::info!("Processing callbacks...");
    for _ in 0..100 {
        runtime.process_callbacks();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    log::info!("Fetch example completed");

    // Cleanup
    drop(runtime);
    let _ = tokio::time::timeout(Duration::from_secs(1), event_loop_handle).await;
}
