use openworkers_core::{Event, HttpMethod, HttpRequest, RequestBody, Script};
use openworkers_runtime_jsc::Worker;
use std::collections::HashMap;

#[tokio::test]
async fn test_env_value_with_quotes_and_newlines() {
    let script = r#"
        addEventListener('fetch', (event) => {
            event.respondWith(new Response(env.SECRET));
        });
    "#;

    let value = "line1\nline2\t\"quoted\" back\\slash";

    let mut env = HashMap::new();
    env.insert("SECRET".to_string(), value.to_string());

    let script_obj = Script::with_env(script, env);
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
    let body = response
        .body
        .collect()
        .await
        .expect("The body should not error")
        .expect("Should have body");
    assert_eq!(String::from_utf8_lossy(&body), value);
}
