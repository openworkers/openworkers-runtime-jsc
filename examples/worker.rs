// Example: Worker with fetch event handler (OpenWorkers-compatible)
//
// To run: cargo run --example worker

use bytes::Bytes;
use openworkers_runtime_jsc::{HttpRequest, ResponseBody, Task, Worker};
use std::collections::HashMap;

#[tokio::main]
async fn main() {
    env_logger::init();

    log::info!("Creating worker with fetch handler");

    // Define worker script with event handler
    let script = r#"
        console.log("Worker initializing...");

        addEventListener('fetch', async (event) => {
            const request = event.request;

            console.log("Fetch event received!");
            console.log("  Method:", request.method);
            console.log("  URL:", request.url);

            // Simple router
            if (request.url.includes('/api/hello')) {
                const data = {
                    message: "Hello from JSCore Worker!",
                    timestamp: Date.now(),
                    method: request.method
                };

                const response = new Response(JSON.stringify(data), {
                    status: 200,
                    headers: {
                        'Content-Type': 'application/json',
                        'X-Worker': 'JSCore'
                    }
                });

                event.respondWith(response);
            } else if (request.url.includes('/api/echo')) {
                // Echo back the request body
                const body = await request.text();
                const response = new Response(`Echo: ${body}`, {
                    status: 200
                });
                event.respondWith(response);
            } else {
                const response = new Response('Not Found', {
                    status: 404
                });
                event.respondWith(response);
            }
        });

        console.log("Worker ready!");
    "#;

    // Create worker
    let script_obj = openworkers_runtime_jsc::Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should load");

    println!("\n=== Test 1: GET /api/hello ===");
    let request1 = HttpRequest {
        method: "GET".to_string(),
        url: "https://example.com/api/hello".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task1, _) = Task::fetch(request1);
    let response1 = worker.exec_http(task1).await.expect("Task should execute");

    println!("Status: {}", response1.status);
    if let ResponseBody::Bytes(body) = response1.body {
        println!("Body: {}", String::from_utf8_lossy(&body));
    }

    println!("\n=== Test 2: POST /api/echo ===");
    let request2 = HttpRequest {
        method: "POST".to_string(),
        url: "https://example.com/api/echo".to_string(),
        headers: HashMap::new(),
        body: Some(Bytes::from("Test message from Rust")),
    };

    let (task2, _) = Task::fetch(request2);
    let response2 = worker.exec_http(task2).await.expect("Task should execute");

    println!("Status: {}", response2.status);
    if let ResponseBody::Bytes(body) = response2.body {
        println!("Body: {}", String::from_utf8_lossy(&body));
    }

    println!("\n=== Test 3: GET /unknown ===");
    let request3 = HttpRequest {
        method: "GET".to_string(),
        url: "https://example.com/unknown".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task3, _) = Task::fetch(request3);
    let response3 = worker.exec_http(task3).await.expect("Task should execute");

    println!("Status: {}", response3.status);
    if let ResponseBody::Bytes(body) = response3.body {
        println!("Body: {}", String::from_utf8_lossy(&body));
    }

    println!("\n=== Worker example completed ===");
}
