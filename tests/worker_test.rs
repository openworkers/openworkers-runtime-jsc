use bytes::Bytes;
use openworkers_runtime_jsc::{HttpRequest, ResponseBody, Task, Worker};
use std::collections::HashMap;

#[tokio::test]
async fn test_worker_basic_fetch_handler() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const response = new Response('Hello from worker!');
            event.respondWith(response);
        });
    "#;

    let script_obj = openworkers_runtime_jsc::Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    // Create a fetch task
    let request = HttpRequest {
        method: "GET".to_string(),
        url: "https://example.com/test".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task, _rx) = Task::fetch(request);

    // Execute the task
    let response = worker.exec_http(task).await.expect("Task should execute");

    assert_eq!(response.status, 200, "Should return 200 status");
    assert!(!response.body.is_none(), "Should have response body");

    if let ResponseBody::Bytes(body) = response.body {
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            body_str.contains("Hello"),
            "Response should contain worker message"
        );
    }
}

#[tokio::test]
async fn test_worker_json_response() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const data = { message: 'success', value: 42 };
            const response = new Response(JSON.stringify(data), {
                status: 200,
                headers: { 'Content-Type': 'application/json' }
            });
            event.respondWith(response);
        });
    "#;

    let script_obj = openworkers_runtime_jsc::Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: "GET".to_string(),
        url: "/api/data".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task, _rx) = Task::fetch(request);
    let response = worker.exec_http(task).await.expect("Task should execute");

    assert_eq!(response.status, 200);

    if let ResponseBody::Bytes(body) = response.body {
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("success"));
        assert!(body_str.contains("42"));
    }
}

#[tokio::test]
async fn test_worker_response_headers() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const response = new Response('Hello with headers!', {
                status: 201,
                headers: {
                    'Content-Type': 'text/plain',
                    'X-Custom-Header': 'custom-value',
                    'X-Worker-Id': 'test-worker-123'
                }
            });
            event.respondWith(response);
        });
    "#;

    let script_obj = openworkers_runtime_jsc::Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: "GET".to_string(),
        url: "/test".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task, _rx) = Task::fetch(request);
    let response = worker.exec_http(task).await.expect("Task should execute");

    assert_eq!(response.status, 201, "Should return 201 status");

    // Check headers are present
    assert!(!response.headers.is_empty(), "Should have headers");

    // Check specific headers (Headers class normalizes keys to lowercase per WHATWG spec)
    let headers_map: HashMap<String, String> = response.headers.into_iter().collect();
    assert_eq!(
        headers_map.get("content-type"),
        Some(&"text/plain".to_string()),
        "Should have content-type header"
    );
    assert_eq!(
        headers_map.get("x-custom-header"),
        Some(&"custom-value".to_string()),
        "Should have custom header"
    );
    assert_eq!(
        headers_map.get("x-worker-id"),
        Some(&"test-worker-123".to_string()),
        "Should have worker id header"
    );

    // Check body
    if let ResponseBody::Bytes(body) = response.body {
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(body_str, "Hello with headers!");
    }
}

#[tokio::test]
async fn test_worker_access_request_data() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const req = event.request;
            const response = new Response(
                `Method: ${req.method}, URL: ${req.url}`
            );
            event.respondWith(response);
        });
    "#;

    let script_obj = openworkers_runtime_jsc::Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: "POST".to_string(),
        url: "/api/create".to_string(),
        headers: HashMap::new(),
        body: Some(Bytes::from("test data")),
    };

    let (task, _rx) = Task::fetch(request);
    let response = worker.exec_http(task).await.expect("Task should execute");

    if let ResponseBody::Bytes(body) = response.body {
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("POST"), "Should include method");
        assert!(body_str.contains("/api/create"), "Should include URL");
    }
}

#[tokio::test]
async fn test_worker_no_handler_error() {
    let script = r#"
        // No event handler registered
        console.log("Worker loaded without handler");
    "#;

    let script_obj = openworkers_runtime_jsc::Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should load");

    let request = HttpRequest {
        method: "GET".to_string(),
        url: "/".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task, _rx) = Task::fetch(request);
    let result = worker.exec_http(task).await;

    // Should error when no handler is registered
    // Either immediate error or timeout waiting for response
    assert!(result.is_err(), "Should error when no handler registered");

    if let Err(e) = result {
        assert!(
            e.contains("No fetch handler")
                || e.contains("not a function")
                || e.contains("timeout")
                || e.contains("Response timeout")
                || e.contains("2s"),
            "Error should mention missing handler or timeout, got: {}",
            e
        );
    }
}

#[tokio::test]
async fn test_worker_scheduled_event() {
    let script = r#"
        globalThis.scheduledRan = false;

        addEventListener('scheduled', (event) => {
            globalThis.scheduledRan = true;
            console.log('Scheduled event fired at:', event.scheduledTime);
        });
    "#;

    let script_obj = openworkers_runtime_jsc::Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    // Create scheduled task
    let (task, _rx) = Task::scheduled(Date::now());

    worker
        .exec_http(task)
        .await
        .expect("Scheduled task should run");

    // Check that handler ran
    let check = r#"globalThis.scheduledRan"#;
    match worker.evaluate(check) {
        Ok(result) => {
            assert!(
                result.to_bool(worker.context()),
                "Scheduled handler should have run"
            );
        }
        Err(_) => panic!("Failed to check if scheduled ran"),
    }
}

// Helper for Date::now()
struct Date;
impl Date {
    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }
}
