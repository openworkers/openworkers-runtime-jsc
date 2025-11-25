use openworkers_runtime_jsc::{HttpRequest, ResponseBody, Task, Worker};
use std::collections::HashMap;

/// Test fetch forward - when the response from fetch() is directly passed to respondWith()
/// The response body should be streamed directly without buffering
#[tokio::test]
async fn test_fetch_forward_basic() {
    let script = r#"
        addEventListener('fetch', (event) => {
            // Forward the fetch response directly
            event.respondWith(fetch('https://echo.workers.rocks/get'));
        });
    "#;

    let script_obj = openworkers_runtime_jsc::Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: "GET".to_string(),
        url: "https://example.com/test".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task, rx) = Task::fetch(request);

    // Execute via the Task channel instead of exec_http
    // This tests the full streaming path
    worker.exec(task).await.expect("Task should execute");

    // Wait for response
    let response = tokio::time::timeout(std::time::Duration::from_secs(10), rx)
        .await
        .expect("Should receive response within timeout")
        .expect("Channel should not close");

    assert_eq!(
        response.status, 200,
        "Should forward 200 status from upstream"
    );

    // For streaming responses, the body comes through the channel
    match response.body {
        ResponseBody::Stream(mut rx) => {
            // Collect all chunks
            let mut all_bytes = Vec::new();
            while let Some(chunk_result) = rx.recv().await {
                match chunk_result {
                    Ok(bytes) => all_bytes.extend_from_slice(&bytes),
                    Err(e) => panic!("Stream error: {}", e),
                }
            }

            let body_str = String::from_utf8_lossy(&all_bytes);
            assert!(body_str.len() > 0, "Should have forwarded body content");
            // echo.workers.rocks/get returns request info as JSON with headers
            assert!(
                body_str.contains("accept")
                    || body_str.contains("host")
                    || body_str.contains("echo.workers.rocks"),
                "Body should contain request info: {}",
                body_str
            );
        }
        ResponseBody::Bytes(bytes) => {
            // Fallback if not streaming (buffered response)
            let body_str = String::from_utf8_lossy(&bytes);
            assert!(body_str.len() > 0, "Should have body content");
        }
        ResponseBody::None => {
            panic!("Should have response body");
        }
    }
}

/// Test that fetch forward preserves headers from upstream
#[tokio::test]
async fn test_fetch_forward_headers() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(fetch('https://echo.workers.rocks/response-headers?X-Custom=test-value'));
        });
    "#;

    let script_obj = openworkers_runtime_jsc::Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: "GET".to_string(),
        url: "https://example.com/test".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task, rx) = Task::fetch(request);

    worker.exec(task).await.expect("Task should execute");

    let response = tokio::time::timeout(std::time::Duration::from_secs(10), rx)
        .await
        .expect("Should receive response within timeout")
        .expect("Channel should not close");

    assert_eq!(response.status, 200);

    // Headers should be forwarded from upstream
    assert!(!response.headers.is_empty(), "Should have headers");
}

/// Test streaming response body with _nativeStreamId detection
#[tokio::test]
async fn test_native_stream_id_propagation() {
    // This test verifies that _nativeStreamId is properly propagated
    // from the ReadableStream body to the Response object
    let script = r#"
        globalThis.testResult = null;

        addEventListener('fetch', async (event) => {
            try {
                const response = await fetch('https://echo.workers.rocks/get');

                // Check if the native stream id is propagated to the Response
                testResult = {
                    bodyIsStream: response.body instanceof ReadableStream,
                    bodyHasNativeId: response.body && response.body._nativeStreamId !== undefined,
                    responseHasNativeId: response._nativeStreamId !== undefined && response._nativeStreamId !== null,
                    nativeStreamIdValue: response._nativeStreamId
                };

                event.respondWith(new Response(JSON.stringify(testResult)));
            } catch (e) {
                event.respondWith(new Response('Error: ' + e.message, { status: 500 }));
            }
        });
    "#;

    let script_obj = openworkers_runtime_jsc::Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: "GET".to_string(),
        url: "https://example.com/test".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task, rx) = Task::fetch(request);

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

    assert!(
        result["bodyIsStream"].as_bool().unwrap_or(false),
        "Body should be a ReadableStream"
    );
    assert!(
        result["bodyHasNativeId"].as_bool().unwrap_or(false),
        "Body stream should have _nativeStreamId"
    );
    assert!(
        result["responseHasNativeId"].as_bool().unwrap_or(false),
        "Response should have _nativeStreamId propagated from body"
    );
}
