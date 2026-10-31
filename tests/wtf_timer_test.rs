//! The static build fires the WTF timers from the event loop: a
//! FinalizationRegistry callback runs while an event waits.

#![cfg(feature = "static")]

use openworkers_core::{Event, HttpMethod, HttpRequest, RequestBody, RuntimeLimits, Script};
use openworkers_runtime_jsc::Worker;
use std::collections::HashMap;

/// Registers objects, then makes garbage in steps of 5 ms until a cleanup
/// callback ran or 2 s passed, and answers the number of callbacks.
const CODE: &str = "addEventListener('fetch', (event) => {
    let cleaned = 0;
    const registry = new FinalizationRegistry(() => { cleaned++; });
    const end = Date.now() + 2000;

    event.respondWith(new Promise((resolve) => {
        const step = () => {
            for (let i = 0; i < 1000; i++) registry.register({ i }, i);
            for (let i = 0; i < 20000; i++) globalThis.garbage = [i, 'x' + i];

            if (cleaned > 0 || Date.now() > end) {
                resolve(new Response(String(cleaned)));
            } else {
                setTimeout(step, 5);
            }
        };

        step();
    }));
});";

#[tokio::test]
async fn finalization_callbacks_run_during_an_event() {
    let limits = RuntimeLimits {
        max_cpu_time_ms: 0,
        ..Default::default()
    };
    let mut worker = Worker::new(Script::new(CODE), Some(limits)).await.unwrap();
    let (task, rx) = Event::fetch(HttpRequest {
        method: HttpMethod::Get,
        url: "http://localhost/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    });

    worker.exec(task).await.unwrap();

    let response = rx.await.unwrap();
    let bytes = response.body.collect().await.unwrap().unwrap_or_default();
    let cleaned: u32 = String::from_utf8_lossy(&bytes).parse().unwrap();

    assert!(cleaned > 0, "no FinalizationRegistry callback ran");
}
