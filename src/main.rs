use openworkers_runtime_jscore::Runtime;

#[tokio::main]
async fn main() {
    // Initialize logger
    env_logger::init();

    log::info!("OpenWorkers JSCore Runtime - Hello World");

    // Create runtime
    let mut runtime = Runtime::new();

    // Execute some JavaScript
    let script = r#"
        console.log("Hello from JavaScriptCore!");

        // Test basic functionality
        const result = 2 + 2;
        console.log("2 + 2 =", result);

        // Return a value
        ({ message: "Hello from JS", value: 42 })
    "#;

    match runtime.evaluate(script) {
        Ok(result) => {
            log::info!("Script executed successfully");

            // Try to get the result as an object
            if let Ok(obj) = result.to_object(&runtime.context) {
                // Try to get message property
                if let Some(message_val) = obj.get_property(&runtime.context, "message") {
                    if let Ok(msg_str) = message_val.to_js_string(&runtime.context) {
                        println!("Message from JS: {}", msg_str);
                    }
                }

                // Try to get value property
                if let Some(value_val) = obj.get_property(&runtime.context, "value") {
                    if let Ok(val) = value_val.to_number(&runtime.context) {
                        println!("Value from JS: {}", val);
                    }
                }
            }
        }
        Err(e) => {
            if let Ok(err_str) = e.to_js_string(&runtime.context) {
                eprintln!("Script execution failed: {}", err_str);
            } else {
                eprintln!("Script execution failed with unknown error");
            }
        }
    }

    log::info!("Runtime completed successfully");
}
