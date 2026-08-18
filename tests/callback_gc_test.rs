mod common;

use common::TestRunner;
use openworkers_core::{HttpRequest, HttpResponse, OpFuture, OperationsHandler, ResponseBody};
use openworkers_runtime_jsc::OperationsHandle;
use std::sync::Arc;
use std::time::Duration;

struct MockOps;

impl OperationsHandler for MockOps {
    fn handle_fetch(&self, _request: HttpRequest) -> OpFuture<'_, Result<HttpResponse, String>> {
        Box::pin(async move {
            Ok(HttpResponse {
                status: 200,
                headers: vec![],
                body: ResponseBody::Bytes("upstream".into()),
            })
        })
    }
}

/// Allocate until the collector runs, and assert on a canary that it did
fn collect_garbage(runner: &mut TestRunner) {
    let canary = "globalThis.canary = new WeakRef({ collectMe: true });";

    runner.execute(canary).expect("Canary should be created");

    unsafe { rusty_jsc::private::JSGarbageCollect(runner.runtime.context.get_ref()) };

    let churn = r#"
        (function() {
            let sink = null;
            for (let i = 0; i < 500000; i++) {
                sink = { i, pad: 'x'.repeat(32) };
            }
            return sink;
        })();
    "#;

    runner.execute(churn).expect("Churn should execute");

    let collected = runner
        .runtime
        .evaluate("globalThis.canary.deref() === undefined")
        .expect("Canary should be readable")
        .to_bool(&runner.runtime.context);

    assert!(collected, "Nothing was collected, the probe proves nothing");
}

/// A pending timer callback is unreachable from JS user code, so only the
/// runtime keeps it alive
#[tokio::test]
async fn test_timeout_callback_survives_collection() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.fired = false;
        setTimeout(() => { globalThis.fired = true; }, 50);
    "#;

    runner.execute(script).expect("Script should execute");

    collect_garbage(&mut runner);

    runner.process_for(Duration::from_millis(200)).await;

    let fired = runner
        .runtime
        .evaluate("globalThis.fired")
        .expect("Result should be readable")
        .to_bool(&runner.runtime.context);

    assert!(fired, "Timeout callback should survive a collection");

    runner.shutdown().await;
}

/// A collected fetch resolve leaves the request hanging until the wall clock
#[tokio::test]
async fn test_fetch_resolve_survives_collection() {
    let ops: OperationsHandle = Arc::new(MockOps);
    let mut runner = TestRunner::new_with_ops(ops);

    let script = r#"
        globalThis.body = null;
        fetch('https://example.com/get')
            .then(response => response.text())
            .then(text => { globalThis.body = text; });
    "#;

    runner.execute(script).expect("Script should execute");

    collect_garbage(&mut runner);

    runner.process_for(Duration::from_millis(200)).await;

    let body = runner
        .runtime
        .evaluate("String(globalThis.body)")
        .expect("Result should be readable")
        .to_js_string(&runner.runtime.context)
        .expect("Result should be a string")
        .to_string();

    assert_eq!(body, "upstream", "Fetch should resolve after a collection");

    runner.shutdown().await;
}
