use openworkers_core::{HttpMethod, HttpRequest, RequestBody, ResponseBody, Script, Task};
use openworkers_runtime_jsc::Worker;
use std::collections::HashMap;

#[tokio::test]
async fn test_response_body_is_readable_stream() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const response = new Response('Hello World');
            const result = response.body instanceof ReadableStream ? 'OK' : 'FAIL';
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
async fn test_response_text_method() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const response = new Response('Hello World');
            const text = await response.text();
            const result = text === 'Hello World' ? 'OK' : `FAIL: ${text}`;
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
async fn test_response_json_method() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const data = { name: 'Claude', version: 3 };
            const response = new Response(JSON.stringify(data));
            const parsed = await response.json();
            const result = parsed.name === 'Claude' && parsed.version === 3 ? 'OK' : 'FAIL';
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
async fn test_response_array_buffer_method() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const response = new Response('Hello');
            const buffer = await response.arrayBuffer();
            const result = buffer instanceof ArrayBuffer && buffer.byteLength === 5 ? 'OK' : 'FAIL';
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
async fn test_response_bytes_method() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const response = new Response('Hello');
            const bytes = await response.bytes();
            const result = bytes instanceof Uint8Array && bytes.length === 5 ? 'OK' : 'FAIL';
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
async fn test_response_body_used_throws() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const response = new Response('Hello');
            await response.text();  // First read

            let error = null;
            try {
                await response.text();  // Should throw
            } catch (e) {
                error = e.message;
            }

            const result = error && error.includes('consumed') ? 'OK' : `FAIL: ${error}`;
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
async fn test_response_from_uint8array() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const bytes = new Uint8Array([72, 101, 108, 108, 111]);  // "Hello"
            const response = new Response(bytes);
            const text = await response.text();
            const result = text === 'Hello' ? 'OK' : `FAIL: ${text}`;
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
async fn test_response_json_static_method() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const response = Response.json({ hello: 'world' });
            const contentType = response.headers.get('content-type');
            const data = await response.json();
            const result = contentType === 'application/json' && data.hello === 'world'
                ? 'OK' : `FAIL: ${contentType}, ${JSON.stringify(data)}`;
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
async fn test_response_redirect_static_method() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const response = Response.redirect('https://example.com/new', 302);
            const location = response.headers.get('location');
            const result = response.status === 302 && location === 'https://example.com/new'
                ? 'OK' : `FAIL: ${response.status}, ${location}`;
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
async fn test_response_read_body_stream() {
    let script = r#"
        addEventListener('fetch', async (event) => {
            const response = new Response('Hello World');
            const reader = response.body.getReader();

            const chunks = [];
            while (true) {
                const { done, value } = await reader.read();
                if (done) break;
                chunks.push(value);
            }

            // Concatenate chunks
            const totalLength = chunks.reduce((sum, c) => sum + c.length, 0);
            const combined = new Uint8Array(totalLength);
            let offset = 0;
            for (const chunk of chunks) {
                combined.set(chunk, offset);
                offset += chunk.length;
            }

            const text = new TextDecoder().decode(combined);
            const result = text === 'Hello World' ? 'OK' : `FAIL: ${text}`;
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
