use openworkers_core::{HttpMethod, HttpRequest, RequestBody, ResponseBody, Script, Task};
use openworkers_runtime_jsc::Worker;
use std::collections::HashMap;

#[tokio::test]
async fn test_text_encoder_basic() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const encoder = new TextEncoder();
            const bytes = encoder.encode('Hello');

            // Check encoding property
            const encoding = encoder.encoding;

            // Verify bytes
            const result = bytes.length === 5
                && bytes[0] === 72  // H
                && bytes[1] === 101 // e
                && bytes[2] === 108 // l
                && bytes[3] === 108 // l
                && bytes[4] === 111 // o
                && encoding === 'utf-8'
                ? 'OK' : 'FAIL';

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
async fn test_text_decoder_basic() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const decoder = new TextDecoder();
            const bytes = new Uint8Array([72, 101, 108, 108, 111]); // Hello
            const text = decoder.decode(bytes);

            const result = text === 'Hello' && decoder.encoding === 'utf-8'
                ? 'OK' : `FAIL: ${text}`;

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
async fn test_text_encoder_emoji() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const encoder = new TextEncoder();
            const decoder = new TextDecoder();

            // Test emoji (4-byte UTF-8)
            const emoji = '🌐';
            const bytes = encoder.encode(emoji);
            const decoded = decoder.decode(bytes);

            const result = decoded === emoji
                ? 'OK'
                : `FAIL: encoded ${bytes.length} bytes, decoded to "${decoded}"`;

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
async fn test_text_encoder_roundtrip() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const encoder = new TextEncoder();
            const decoder = new TextDecoder();

            const tests = [
                'Hello, World!',
                'Привет мир',
                '你好世界',
                '🎉🚀✨',
                'Mixed: Hello 世界 🌍'
            ];

            let allPassed = true;
            let failedTest = '';

            for (const test of tests) {
                const encoded = encoder.encode(test);
                const decoded = decoder.decode(encoded);
                if (decoded !== test) {
                    allPassed = false;
                    failedTest = `"${test}" -> "${decoded}"`;
                    break;
                }
            }

            const result = allPassed ? 'OK' : `FAIL: ${failedTest}`;
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
