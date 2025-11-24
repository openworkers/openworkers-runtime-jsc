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

    // Execute JavaScript with setTimeout
    let script = r#"
        console.log("Starting setTimeout test...");

        const start = +Date.now();
        const diff = () => `+${(+Date.now()) - start}`;

        setTimeout(() => {
            console.log("Timeout 1: This should print after 100ms", diff());
        }, 100);

        setTimeout(() => {
            console.log("Timeout 2: This should print after 500ms", diff());
        }, 500);

        setTimeout(() => {
            console.log("Timeout 3: This should print after 200ms", diff());
        }, 200);

        console.log("All timeouts scheduled!");
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
    log::info!("Processing callbacks...");
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
