use openworkers_core::{Event, Script};
use openworkers_runtime_jsc::Worker;

/// Test that a task id which is not valid JS source cannot break out of the event object
#[tokio::test]
async fn test_task_id_is_not_js_source() {
    let script = r#"
        addEventListener('task', (event) => {
            event.respondWith({ success: true, data: event.taskId });
        });
    "#;

    let script_obj = Script::new(script);
    let mut worker = Worker::new(script_obj, None)
        .await
        .expect("Worker should initialize");

    let task_id = "job\", zzz: (globalThis.injected = 1)\nback\\slash";

    let (event, rx) = Event::task(task_id.to_string(), None, None, 1);
    worker.exec(event).await.expect("Task should execute");

    let result = rx.await.expect("Should receive result");
    assert_eq!(result.data, Some(serde_json::Value::String(task_id.into())));

    let injected = worker
        .evaluate("typeof globalThis.injected")
        .expect("Should evaluate");
    let injected = injected
        .to_js_string(worker.context())
        .expect("Should be a string");

    assert_eq!(injected.to_string(), "undefined");
}
