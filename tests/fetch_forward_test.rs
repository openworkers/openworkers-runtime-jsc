use openworkers_core::{
    Event, HttpMethod, HttpRequest, HttpResponse, OpFuture, OperationsHandler, RequestBody,
    ResponseBody, Script,
};
use openworkers_runtime_jsc::{OperationsHandle, Worker};
use std::collections::HashMap;
use std::sync::Arc;

/// Mock operations handler for testing fetch
struct MockOps;

impl OperationsHandler for MockOps {
    fn handle_fetch(&self, request: HttpRequest) -> OpFuture<'_, Result<HttpResponse, String>> {
        Box::pin(async move {
            // Return a mock response based on the URL
            let body = format!(
                r#"{{"url":"{}","method":"{:?}","headers":{}}}"#,
                request.url,
                request.method,
                serde_json::to_string(&request.headers).unwrap_or_default()
            );

            Ok(HttpResponse {
                status: 200,
                headers: vec![
                    ("content-type".to_string(), "application/json".to_string()),
                    ("x-custom".to_string(), "test-value".to_string()),
                ],
                body: ResponseBody::Bytes(body.into()),
            })
        })
    }
}

fn ops() -> OperationsHandle {
    Arc::new(MockOps)
}

/// Test fetch forward - when the response from fetch() is directly passed to respondWith()
#[tokio::test]
async fn test_fetch_forward_basic() {
    let script = r#"
        addEventListener('fetch', (event) => {
            // Forward the fetch response directly
            event.respondWith(fetch('https://echo.workers.rocks/get'));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new_with_ops(script_obj, None, ops())
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/test".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Event::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = tokio::time::timeout(std::time::Duration::from_secs(10), rx)
        .await
        .expect("Should receive response within timeout")
        .expect("Channel should not close");

    assert_eq!(response.status, 200, "Should forward 200 status from mock");
}

/// Test that fetch forward preserves headers from upstream
#[tokio::test]
async fn test_fetch_forward_headers() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(fetch('https://echo.workers.rocks/response-headers?X-Custom=test-value'));
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new_with_ops(script_obj, None, ops())
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/test".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Event::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = tokio::time::timeout(std::time::Duration::from_secs(10), rx)
        .await
        .expect("Should receive response within timeout")
        .expect("Channel should not close");

    assert_eq!(response.status, 200);
    assert!(!response.headers.is_empty(), "Should have headers");
}

/// Test streaming response body with _nativeStreamId detection
#[tokio::test]
async fn test_native_stream_id_propagation() {
    let script = r#"
        globalThis.testResult = null;

        addEventListener('fetch', async (event) => {
            try {
                const response = await fetch('https://echo.workers.rocks/get');

                // Check if the response has the expected properties
                testResult = {
                    status: response.status,
                    hasBody: response.body !== null
                };

                event.respondWith(new Response(JSON.stringify(testResult)));
            } catch (e) {
                event.respondWith(new Response('Error: ' + e.message, { status: 500 }));
            }
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new_with_ops(script_obj, None, ops())
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/test".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Event::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = tokio::time::timeout(std::time::Duration::from_secs(10), rx)
        .await
        .expect("Should receive response within timeout")
        .expect("Channel should not close");

    // Get body content
    let body_str = match response.body {
        ResponseBody::Bytes(bytes) => String::from_utf8_lossy(&bytes).to_string(),
        ResponseBody::Stream(mut rx) => {
            let mut all_bytes = Vec::new();
            while let Some(chunk_result) = rx.recv().await {
                if let Ok(bytes) = chunk_result {
                    all_bytes.extend_from_slice(&bytes);
                }
            }
            String::from_utf8_lossy(&all_bytes).to_string()
        }
        ResponseBody::None => String::new(),
    };

    // Parse the JSON result
    let result: serde_json::Value = serde_json::from_str(&body_str)
        .unwrap_or_else(|_| panic!("Failed to parse JSON: {}", body_str));

    assert_eq!(result["status"], 200, "Mock should return 200");
    assert!(
        result["hasBody"].as_bool().unwrap_or(false),
        "Response should have body"
    );
}
