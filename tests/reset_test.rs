//! Worker::reset between two events: the timers and callbacks of the first do
//! not reach the second, and a guest cannot change what the reset runs.

use openworkers_core::{Event, HttpMethod, HttpRequest, HttpResponse, RequestBody, Script};
use openworkers_runtime_jsc::Worker;
use std::collections::HashMap;
use std::time::{Duration, Instant};

fn get(path: &str) -> (Event, tokio::sync::oneshot::Receiver<HttpResponse>) {
    Event::fetch(HttpRequest {
        method: HttpMethod::Get,
        url: format!("http://localhost{path}"),
        headers: HashMap::new(),
        body: RequestBody::None,
    })
}

async fn run(worker: &mut Worker, path: &str) -> String {
    let (task, rx) = get(path);

    worker.exec(task).await.unwrap();

    let response = rx.await.unwrap();
    let bytes = response.body.collect().await.unwrap().unwrap_or_default();

    String::from_utf8_lossy(&bytes).into_owned()
}

/// /set leaves a timeout and an interval behind; /check waits 80 ms and says
/// whether they ran since.
const CODE: &str = "globalThis.ran = 0;
addEventListener('fetch', (event) => {
    const path = new URL(event.request.url).pathname;

    if (path === '/set') {
        setTimeout(() => { globalThis.ran++; }, 30);
        setInterval(() => { globalThis.ran++; }, 10);
        globalThis.ran = 0;
        event.respondWith(new Response('set'));
        return;
    }

    event.respondWith(new Promise((resolve) => setTimeout(() => resolve(new Response(String(globalThis.ran))), 80)));
});";

#[tokio::test]
async fn the_timers_of_an_event_do_not_reach_the_next() {
    let mut worker = Worker::new(Script::new(CODE), None).await.unwrap();

    assert_eq!(run(&mut worker, "/set").await, "set");

    worker.reset().unwrap();

    assert_eq!(run(&mut worker, "/check").await, "0");
}

#[tokio::test]
async fn a_guest_cannot_change_the_reset() {
    let code = "Map.prototype.clear = function () { for (;;) {} };
        try { globalThis.__clearCallbacks = function () { for (;;) {} }; } catch {}
        addEventListener('fetch', (event) => event.respondWith(new Response('ok')));";
    let mut worker = Worker::new(Script::new(code), None).await.unwrap();
    let start = Instant::now();

    for _ in 0..3 {
        assert_eq!(run(&mut worker, "/").await, "ok");
        worker.reset().unwrap();
    }

    assert!(
        start.elapsed() < Duration::from_millis(500),
        "{:?}",
        start.elapsed()
    );
}

// An interval of 1 ms left by an event: without a stop at the end of the
// event, an idle worker queues about 1000 callbacks a second
#[tokio::test]
async fn an_idle_worker_queues_no_callback() {
    let code = "addEventListener('fetch', (event) => {
        setInterval(() => {}, 1);
        event.respondWith(new Response('ok'));
    });";
    let mut worker = Worker::new(Script::new(code), None).await.unwrap();

    assert_eq!(run(&mut worker, "/").await, "ok");

    tokio::time::sleep(Duration::from_secs(2)).await;

    let queued = worker.queued_callbacks();

    assert!(queued < 10, "{queued} callbacks queued");
}

#[cfg(feature = "invoke")]
mod invoke {
    use super::*;
    use openworkers_core::{
        BindingInfo, InvokeResult, OpFuture, OperationsHandle, OperationsHandler,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Sets its flag when dropped.
    struct Dropped(Arc<AtomicBool>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    /// Answers after 300 ms; says whether a call finished or was dropped.
    #[derive(Default)]
    struct SlowHost {
        finished: Arc<AtomicBool>,
        dropped: Arc<AtomicBool>,
    }

    impl OperationsHandler for SlowHost {
        fn handle_binding_invoke(
            &self,
            _binding: &str,
            _name: &str,
            _args: serde_json::Value,
        ) -> OpFuture<'_, InvokeResult> {
            let finished = self.finished.clone();
            let dropped = Dropped(self.dropped.clone());

            Box::pin(async move {
                let _dropped = dropped;

                tokio::time::sleep(Duration::from_millis(300)).await;
                finished.store(true, Ordering::SeqCst);

                InvokeResult::Value(serde_json::json!(1))
            })
        }
    }

    // An event that answers 50 ms after it calls an invoke of 300 ms: the
    // invoke started, and stops with the event
    #[tokio::test]
    async fn the_invokes_of_an_event_stop_with_it() {
        let code = "addEventListener('fetch', (event) => {
            env.SLOW.invoke('wait', {});
            event.respondWith(new Promise((resolve) => setTimeout(() => resolve(new Response('ok')), 50)));
        });";
        let host = Arc::new(SlowHost::default());
        let ops: OperationsHandle = host.clone();
        let script = Script {
            code: code.into(),
            env: None,
            bindings: vec![BindingInfo::invoke("SLOW")],
        };
        let mut worker = Worker::new_with_ops(script, None, ops).await.unwrap();

        assert_eq!(run(&mut worker, "/").await, "ok");

        tokio::time::sleep(Duration::from_millis(500)).await;

        assert!(host.dropped.load(Ordering::SeqCst), "the invoke still runs");
        assert!(!host.finished.load(Ordering::SeqCst), "the invoke finished");
    }
}
