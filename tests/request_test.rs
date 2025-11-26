use openworkers_runtime_jsc::{HttpRequest, Script, Task, Worker};
use std::collections::HashMap;

/// Test basic Request construction with URL string
#[tokio::test]
async fn test_request_basic_construction() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const req = new Request('https://example.com/api');

            event.respondWith(new Response(JSON.stringify({
                url: req.url,
                method: req.method,
                hasHeaders: req.headers instanceof Headers,
                bodyUsed: req.bodyUsed,
                mode: req.mode,
                credentials: req.credentials
            })));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: "GET".to_string(),
        url: "https://test.com/".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task, rx) = Task::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.as_bytes().expect("Should have body");
    let body_str = String::from_utf8_lossy(&body);
    let result: serde_json::Value = serde_json::from_str(&body_str).expect("Valid JSON");

    assert_eq!(result["url"], "https://example.com/api");
    assert_eq!(result["method"], "GET");
    assert_eq!(result["hasHeaders"], true);
    assert_eq!(result["bodyUsed"], false);
    assert_eq!(result["mode"], "cors");
    assert_eq!(result["credentials"], "same-origin");
}

/// Test Request with POST method and body
#[tokio::test]
async fn test_request_with_body() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const req = new Request('https://example.com/api', {
                method: 'POST',
                body: 'Hello World',
                headers: { 'Content-Type': 'text/plain' }
            });

            const bodyText = await req.text();

            event.respondWith(new Response(JSON.stringify({
                url: req.url,
                method: req.method,
                bodyText: bodyText,
                bodyUsed: req.bodyUsed,
                contentType: req.headers.get('Content-Type')
            })));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: "GET".to_string(),
        url: "https://test.com/".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task, rx) = Task::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.as_bytes().expect("Should have body");
    let body_str = String::from_utf8_lossy(&body);
    let result: serde_json::Value = serde_json::from_str(&body_str).expect("Valid JSON");

    assert_eq!(result["method"], "POST");
    assert_eq!(result["bodyText"], "Hello World");
    assert_eq!(result["bodyUsed"], true);
    assert_eq!(result["contentType"], "text/plain");
}

/// Test Request clone
#[tokio::test]
async fn test_request_clone() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const original = new Request('https://example.com/api', {
                method: 'POST',
                headers: { 'X-Custom': 'value' }
            });

            const cloned = original.clone();

            event.respondWith(new Response(JSON.stringify({
                sameUrl: original.url === cloned.url,
                sameMethod: original.method === cloned.method,
                clonedHeader: cloned.headers.get('X-Custom'),
                notSameObject: original !== cloned
            })));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: "GET".to_string(),
        url: "https://test.com/".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task, rx) = Task::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.as_bytes().expect("Should have body");
    let body_str = String::from_utf8_lossy(&body);
    let result: serde_json::Value = serde_json::from_str(&body_str).expect("Valid JSON");

    assert_eq!(result["sameUrl"], true);
    assert_eq!(result["sameMethod"], true);
    assert_eq!(result["clonedHeader"], "value");
    assert_eq!(result["notSameObject"], true);
}

/// Test Request from another Request
#[tokio::test]
async fn test_request_from_request() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const original = new Request('https://example.com/api', {
                method: 'POST',
                headers: { 'X-Original': 'yes' }
            });

            // Create new Request from existing, override method
            const modified = new Request(original, {
                method: 'PUT',
                headers: { 'X-Modified': 'yes' }
            });

            event.respondWith(new Response(JSON.stringify({
                originalMethod: original.method,
                modifiedMethod: modified.method,
                originalUrl: original.url,
                modifiedUrl: modified.url,
                modifiedHeader: modified.headers.get('X-Modified')
            })));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: "GET".to_string(),
        url: "https://test.com/".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task, rx) = Task::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.as_bytes().expect("Should have body");
    let body_str = String::from_utf8_lossy(&body);
    let result: serde_json::Value = serde_json::from_str(&body_str).expect("Valid JSON");

    assert_eq!(result["originalMethod"], "POST");
    assert_eq!(result["modifiedMethod"], "PUT");
    assert_eq!(result["originalUrl"], "https://example.com/api");
    assert_eq!(result["modifiedUrl"], "https://example.com/api");
    assert_eq!(result["modifiedHeader"], "yes");
}

/// Test Request.json() method
#[tokio::test]
async fn test_request_json() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const req = new Request('https://example.com/api', {
                method: 'POST',
                body: JSON.stringify({ name: 'test', value: 42 })
            });

            const data = await req.json();

            event.respondWith(new Response(JSON.stringify({
                name: data.name,
                value: data.value
            })));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: "GET".to_string(),
        url: "https://test.com/".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task, rx) = Task::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.as_bytes().expect("Should have body");
    let body_str = String::from_utf8_lossy(&body);
    let result: serde_json::Value = serde_json::from_str(&body_str).expect("Valid JSON");

    assert_eq!(result["name"], "test");
    assert_eq!(result["value"], 42);
}

/// Test Request.arrayBuffer() method
#[tokio::test]
async fn test_request_arraybuffer() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const req = new Request('https://example.com/api', {
                method: 'POST',
                body: new Uint8Array([1, 2, 3, 4, 5])
            });

            const buffer = await req.arrayBuffer();
            const view = new Uint8Array(buffer);

            event.respondWith(new Response(JSON.stringify({
                length: view.length,
                first: view[0],
                last: view[view.length - 1]
            })));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: "GET".to_string(),
        url: "https://test.com/".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task, rx) = Task::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.as_bytes().expect("Should have body");
    let body_str = String::from_utf8_lossy(&body);
    let result: serde_json::Value = serde_json::from_str(&body_str).expect("Valid JSON");

    assert_eq!(result["length"], 5);
    assert_eq!(result["first"], 1);
    assert_eq!(result["last"], 5);
}
