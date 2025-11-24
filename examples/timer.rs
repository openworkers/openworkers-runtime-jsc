// Example: Timer implementation (conceptual)
// This shows how we could implement setTimeout with JSCore + Tokio
//
// To run: cargo run --example timer

use openworkers_runtime_jscore::Runtime;

#[tokio::main]
async fn main() {
    env_logger::init();

    let mut runtime = Runtime::new();

    // Basic hello world
    let script = r#"
        console.log("Starting timer example...");

        // This is what we want to support in the future:
        // setTimeout(() => {
        //     console.log("Timer fired after 1000ms!");
        // }, 1000);

        console.log("For now, we only have synchronous console.log");
        console.log("But the architecture is ready for async operations!");
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

    // Here we could run the event loop to process async tasks:
    // runtime.run_event_loop().await;

    log::info!("Timer example completed");
}
