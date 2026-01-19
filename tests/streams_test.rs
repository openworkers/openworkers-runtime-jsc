use openworkers_core::{Event, HttpMethod, HttpRequest, RequestBody, ResponseBody, Script};
use openworkers_runtime_jsc::Worker;
use std::collections::HashMap;

#[tokio::test]
async fn test_readable_stream_creation() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const stream = new ReadableStream({
                start(controller) {
                    controller.enqueue('Hello');
                    controller.close();
                }
            });

            const result = stream instanceof ReadableStream ? 'OK' : 'FAIL';
            event.respondWith(new Response(result));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Event::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}

#[tokio::test]
async fn test_readable_stream_locked() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const stream = new ReadableStream();
            const reader = stream.getReader();

            let error = null;
            try {
                stream.getReader(); // Should throw
            } catch (e) {
                error = e.message;
            }

            const result = error && error.includes('locked') ? 'OK' : `FAIL: ${error}`;
            event.respondWith(new Response(result));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Event::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}

#[tokio::test]
async fn test_readable_stream_with_then() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const stream = new ReadableStream({
                start(controller) {
                    controller.enqueue('Hello');
                    controller.enqueue(' World');
                    controller.close();
                }
            });

            const reader = stream.getReader();

            // Read using Promise.then chains
            reader.read().then(r1 => {
                reader.read().then(r2 => {
                    reader.read().then(r3 => {
                        const result = r1.value + r2.value;
                        const response = (result === 'Hello World' && r3.done)
                            ? 'OK' : `FAIL: ${result}, done=${r3.done}`;
                        event.respondWith(new Response(response));
                    });
                });
            });
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Event::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), "OK");
}
