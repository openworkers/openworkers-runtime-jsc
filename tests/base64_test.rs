use openworkers_runtime_jsc::{HttpRequest, ResponseBody, Task, Worker};
use std::collections::HashMap;

#[tokio::test]
async fn test_btoa_basic() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const encoded = btoa('Hello, World!');
            const result = encoded === 'SGVsbG8sIFdvcmxkIQ=='
                ? 'OK' : `FAIL: ${encoded}`;
            event.respondWith(new Response(result));
        });
    "#;

    let script_obj = openworkers_runtime_jsc::Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: "GET".to_string(),
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task, _) = Task::fetch(request);
    let response = worker.exec_http(task).await.expect("Task should execute");

    if let ResponseBody::Bytes(body) = response.body {
        assert_eq!(String::from_utf8_lossy(&body), "OK");
    } else {
        panic!("Expected bytes body");
    }
}

#[tokio::test]
async fn test_atob_basic() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const decoded = atob('SGVsbG8sIFdvcmxkIQ==');
            const result = decoded === 'Hello, World!'
                ? 'OK' : `FAIL: ${decoded}`;
            event.respondWith(new Response(result));
        });
    "#;

    let script_obj = openworkers_runtime_jsc::Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: "GET".to_string(),
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task, _) = Task::fetch(request);
    let response = worker.exec_http(task).await.expect("Task should execute");

    if let ResponseBody::Bytes(body) = response.body {
        assert_eq!(String::from_utf8_lossy(&body), "OK");
    } else {
        panic!("Expected bytes body");
    }
}

#[tokio::test]
async fn test_base64_roundtrip() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const tests = [
                'Hello',
                'Hello, World!',
                'a',
                'ab',
                'abc',
                'The quick brown fox jumps over the lazy dog'
            ];

            let allPassed = true;
            let failedTest = '';

            for (const test of tests) {
                const encoded = btoa(test);
                const decoded = atob(encoded);
                if (decoded !== test) {
                    allPassed = false;
                    failedTest = `"${test}" -> "${encoded}" -> "${decoded}"`;
                    break;
                }
            }

            const result = allPassed ? 'OK' : `FAIL: ${failedTest}`;
            event.respondWith(new Response(result));
        });
    "#;

    let script_obj = openworkers_runtime_jsc::Script::new(script);
    let mut worker = Worker::new(script_obj, None, None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: "GET".to_string(),
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task, _) = Task::fetch(request);
    let response = worker.exec_http(task).await.expect("Task should execute");

    if let ResponseBody::Bytes(body) = response.body {
        assert_eq!(String::from_utf8_lossy(&body), "OK");
    } else {
        panic!("Expected bytes body");
    }
}
