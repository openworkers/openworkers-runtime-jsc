use openworkers_core::{HttpMethod, HttpRequest, RequestBody, ResponseBody, Script, Task};
use openworkers_runtime_jsc::Worker;
use std::collections::HashMap;

#[tokio::test]
async fn test_headers_constructor_from_object() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const headers = new Headers({ 'Content-Type': 'text/plain', 'X-Custom': 'value' });
            const result = headers.get('content-type') === 'text/plain' &&
                           headers.get('x-custom') === 'value' ? 'OK' : 'FAIL';
            event.respondWith(new Response(result));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Task::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}

#[tokio::test]
async fn test_headers_case_insensitive() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const headers = new Headers();
            headers.set('Content-Type', 'text/plain');

            const result = headers.get('content-type') === 'text/plain' &&
                           headers.get('CONTENT-TYPE') === 'text/plain' &&
                           headers.get('Content-Type') === 'text/plain' ? 'OK' : 'FAIL';
            event.respondWith(new Response(result));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Task::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}

#[tokio::test]
async fn test_headers_append() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const headers = new Headers();
            headers.append('Accept', 'text/html');
            headers.append('Accept', 'application/json');

            const value = headers.get('accept');
            const result = value === 'text/html, application/json' ? 'OK' : `FAIL: ${value}`;
            event.respondWith(new Response(result));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Task::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}

#[tokio::test]
async fn test_headers_has() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const headers = new Headers({ 'Content-Type': 'text/plain' });

            const result = headers.has('content-type') === true &&
                           headers.has('x-missing') === false ? 'OK' : 'FAIL';
            event.respondWith(new Response(result));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Task::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}

#[tokio::test]
async fn test_headers_delete() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const headers = new Headers({ 'Content-Type': 'text/plain', 'X-Custom': 'value' });
            headers.delete('x-custom');

            const result = headers.has('content-type') === true &&
                           headers.has('x-custom') === false ? 'OK' : 'FAIL';
            event.respondWith(new Response(result));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Task::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}

#[tokio::test]
async fn test_headers_iteration() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const headers = new Headers({ 'Content-Type': 'text/plain', 'X-Custom': 'value' });

            const entries = [];
            for (const [key, value] of headers) {
                entries.push(`${key}:${value}`);
            }

            // Headers are stored in insertion order
            const result = entries.includes('content-type:text/plain') &&
                           entries.includes('x-custom:value') ? 'OK' : `FAIL: ${entries.join(',')}`;
            event.respondWith(new Response(result));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Task::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}

#[tokio::test]
async fn test_headers_foreach() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const headers = new Headers({ 'Content-Type': 'text/plain' });

            let called = false;
            headers.forEach((value, key) => {
                if (key === 'content-type' && value === 'text/plain') {
                    called = true;
                }
            });

            const result = called ? 'OK' : 'FAIL';
            event.respondWith(new Response(result));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Task::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}

#[tokio::test]
async fn test_headers_clone_from_headers() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const original = new Headers({ 'Content-Type': 'text/plain' });
            const cloned = new Headers(original);

            // Modify original should not affect clone
            original.set('Content-Type', 'text/html');

            const result = cloned.get('content-type') === 'text/plain' &&
                           original.get('content-type') === 'text/html' ? 'OK' : 'FAIL';
            event.respondWith(new Response(result));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Task::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}
