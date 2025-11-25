// Example: setTimeout with JSCore + Tokio
//
// To run: cargo run --example timer

use openworkers_runtime_jsc::{Runtime, run_event_loop};
use std::time::Duration;

#[tokio::main]
async fn main() {
    env_logger::init();

    log::info!("Starting timer example with setTimeout");

    // Create runtime and event loop
    let (mut runtime, scheduler_rx, callback_tx, stream_manager) = Runtime::new();

    // Spawn the background event loop
    let event_loop_handle = tokio::spawn(async move {
        run_event_loop(scheduler_rx, callback_tx, stream_manager).await;
    });

    // Execute JavaScript with setTimeout
    let script = r#"
        console.log("Starting timer example...");

        const start = +Date.now();
        const diff = () => `+${(+Date.now()) - start}ms`;

        setTimeout(() => {
            console.log("Timer 1 fired after 1000ms", diff());
        }, 1000);

        setTimeout(() => {
            console.log("Timer 2 fired after 300ms", diff());
        }, 300);

        setTimeout(() => {
            console.log("Timer 3 fired after 2000ms", diff());
        }, 2000);

        console.log("All timers scheduled!");
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

    // Process callbacks for 3 seconds
    log::info!("Processing callbacks...");
    for _ in 0..60 {
        runtime.process_callbacks();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    log::info!("Timer example completed");

    // Cleanup: runtime will be dropped, sending shutdown message
    drop(runtime);

    // Wait for event loop to finish
    let _ = tokio::time::timeout(Duration::from_secs(1), event_loop_handle).await;
}
