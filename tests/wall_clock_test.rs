use openworkers_core::{
    Event, HttpMethod, HttpRequest, RequestBody, ResponseBody, RuntimeLimits, Script,
    TerminationReason,
};
use openworkers_runtime_jsc::Worker;
use std::collections::HashMap;
use std::time::Duration;
use std::time::Instant;

fn request() -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Get,
        url: "https://example.com/test".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    }
}

/// A slow handler is bounded only by the wall-clock limit,
/// never by a fixed internal cap
#[tokio::test]
async fn test_slow_response_within_wall_clock_limit() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(new Promise((resolve) => {
                setTimeout(() => resolve(new Response('late')), 6000);
            }));
        });
    "#;

    let limits = RuntimeLimits {
        max_wall_clock_time_ms: 10_000,
        ..Default::default()
    };

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, Some(limits))
        .await
        .expect("Worker should initialize");

    let start = Instant::now();
    let (task, rx) = Event::fetch(request());
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Channel should not close");
    let elapsed = start.elapsed();

    assert_eq!(response.status, 200);
    assert!(
        elapsed >= Duration::from_secs(6),
        "Response should arrive after the 6s timer, got {:?}",
        elapsed
    );

    let body = match response.body {
        ResponseBody::Bytes(bytes) => String::from_utf8_lossy(&bytes).to_string(),
        ResponseBody::Stream(mut rx) => {
            let mut all_bytes = Vec::new();

            while let Some(chunk) = rx.recv().await {
                all_bytes.extend_from_slice(&chunk.expect("Chunk should not error"));
            }

            String::from_utf8_lossy(&all_bytes).to_string()
        }
        ResponseBody::None => String::new(),
    };

    assert_eq!(body, "late");
}

#[tokio::test]
async fn test_wall_clock_limit_enforced() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(new Promise((resolve) => {
                setTimeout(() => resolve(new Response('too late')), 5000);
            }));
        });
    "#;

    let limits = RuntimeLimits {
        max_wall_clock_time_ms: 300,
        ..Default::default()
    };

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, Some(limits))
        .await
        .expect("Worker should initialize");

    let start = Instant::now();
    let (task, _rx) = Event::fetch(request());
    let result = worker.exec(task).await;

    assert_eq!(result, Err(TerminationReason::WallClockTimeout));
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "Timeout should fire well before the 5s timer"
    );
}
