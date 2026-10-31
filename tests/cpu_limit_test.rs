//! The CPU budget of a request: it cuts JavaScript that does not yield, it
//! holds over all the entries of a request, and it counts CPU time, not wall
//! time.

use openworkers_core::{
    Event, HttpMethod, HttpRequest, HttpResponse, LogLevel, OperationsHandle, OperationsHandler,
    RequestBody, RuntimeLimits, Script, TerminationReason,
};
use openworkers_runtime_jsc::Worker;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// /spin never yields; /wait logs 4 lines, and the host of the test sleeps 60
/// ms in each, so the entry takes 240 ms with almost no CPU; / answers at once.
/// /chain spins 40 ms per entry and chains entries with setTimeout(0) for 2 s;
/// /then spins 45 ms, then enters again with a loop that never yields;
/// /microtasks never leaves the microtasks of its first entry; /30 spins 30 ms;
/// /throw spins 45 ms in a callback, then throws a value whose text never ends.
const CODE: &str = "addEventListener('fetch', (event) => {
    const path = new URL(event.request.url).pathname;
    const spin = (ms) => { const stop = Date.now() + ms; while (Date.now() < stop) {} };

    if (path === '/spin') {
        for (;;) {}
    }

    if (path === '/chain') {
        const end = Date.now() + 2000;
        const step = () => {
            spin(40);
            if (Date.now() < end) { setTimeout(step, 0); } else { event.respondWith(new Response('done')); }
        };
        setTimeout(step, 0);
        return;
    }

    if (path === '/then') {
        spin(45);
        setTimeout(() => { for (;;) {} }, 0);
        return;
    }

    if (path === '/microtasks') {
        event.respondWith((async () => { for (;;) await null; })());
        return;
    }

    if (path === '/throw') {
        setTimeout(() => {
            spin(45);
            throw { toString() { for (;;) {} } };
        }, 0);
        return;
    }

    if (path === '/30') {
        spin(30);
    }

    if (path === '/wait') {
        for (let i = 0; i < 4; i++) console.log('wait');
    }

    event.respondWith(new Response('ok'));
});";

fn get(path: &str) -> (Event, tokio::sync::oneshot::Receiver<HttpResponse>) {
    Event::fetch(HttpRequest {
        method: HttpMethod::Get,
        url: format!("http://localhost{path}"),
        headers: HashMap::new(),
        body: RequestBody::None,
    })
}

async fn body(rx: tokio::sync::oneshot::Receiver<HttpResponse>) -> String {
    let response = rx.await.unwrap();
    let bytes = response.body.collect().await.unwrap().unwrap_or_default();

    String::from_utf8_lossy(&bytes).into_owned()
}

/// The CPU time of the calling thread.
fn thread_cpu_time() -> Duration {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };

    // SAFETY: `now` is a valid timespec to write into
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut now) };

    Duration::new(now.tv_sec as u64, now.tv_nsec as u32)
}

#[tokio::test]
async fn a_loop_that_never_yields_is_cut() {
    let mut worker = Worker::new(Script::new(CODE), None).await.unwrap();
    let (task, _rx) = get("/spin");
    let start = Instant::now();
    let answer = worker.exec(task).await;
    let took = start.elapsed();

    eprintln!("a loop that never yields was cut after {took:?}");
    assert!(
        matches!(answer, Err(TerminationReason::CpuTimeLimit)),
        "{answer:?}"
    );
    assert!(took < Duration::from_millis(200), "{took:?}");

    // The worker holds half an event: it refuses the next one
    let (task, _rx) = get("/");

    assert!(matches!(
        worker.exec(task).await,
        Err(TerminationReason::CpuTimeLimit)
    ));
}

/// A host whose log call sleeps: time inside the entry that uses no CPU.
struct SleepyHost;

impl OperationsHandler for SleepyHost {
    fn handle_log(&self, _level: LogLevel, _message: String) {
        std::thread::sleep(Duration::from_millis(60));
    }
}

// 240 ms of sleep inside one entry, longer than the limit of 50 ms: not cut,
// so the budget takes the CPU time of the thread, not the wall time
#[tokio::test]
async fn a_wait_that_uses_no_cpu_is_not_cut() {
    let ops: OperationsHandle = Arc::new(SleepyHost);
    let mut worker = Worker::new_with_ops(Script::new(CODE), None, ops)
        .await
        .unwrap();
    let (task, rx) = get("/wait");
    let start = Instant::now();
    let answer = worker.exec(task).await;
    let took = start.elapsed();

    assert!(answer.is_ok(), "{answer:?}");
    assert!(took >= Duration::from_millis(240), "{took:?}");
    assert_eq!(body(rx).await, "ok");
}

// Cut at the floor of a load, 400 ms
#[tokio::test]
async fn a_top_level_that_never_ends_makes_no_worker() {
    let start = Instant::now();
    let made = Worker::new(Script::new("for (;;) {}"), None).await.err();

    assert!(
        matches!(made, Some(TerminationReason::CpuTimeLimit)),
        "{made:?}"
    );
    assert!(
        start.elapsed() < Duration::from_millis(1000),
        "{:?}",
        start.elapsed()
    );
}

// 3 events of 30 ms each: 90 ms in all, past the limit of 50 ms, so each
// event gets the whole budget
#[tokio::test]
async fn a_worker_serves_events_in_a_row() {
    let mut worker = Worker::new(Script::new(CODE), None).await.unwrap();

    for _ in 0..3 {
        let (task, rx) = get("/30");

        worker.exec(task).await.unwrap();
        assert_eq!(body(rx).await, "ok");
    }
}

// Entries of 40 ms, each under the limit of 50 ms: the budget of the request
// ends the chain at its second entry
#[tokio::test]
async fn a_chain_of_entries_is_cut_by_the_request_budget() {
    let mut worker = Worker::new(Script::new(CODE), None).await.unwrap();
    let (task, _rx) = get("/chain");
    let start = Instant::now();
    let answer = worker.exec(task).await;
    let took = start.elapsed();

    eprintln!("a chain of 40 ms entries was cut after {took:?}");
    assert!(
        matches!(answer, Err(TerminationReason::CpuTimeLimit)),
        "{answer:?}"
    );
    assert!(took < Duration::from_millis(200), "{took:?}");
}

// 45 ms, then a loop: the watchdog gets what is left (5 ms), not a whole
// limit, so the request ends near 50 ms of CPU, not near 95 ms
#[tokio::test]
async fn an_entry_gets_what_is_left_of_the_budget() {
    let mut worker = Worker::new(Script::new(CODE), None).await.unwrap();
    let (task, _rx) = get("/then");
    let start = thread_cpu_time();
    let answer = worker.exec(task).await;
    let took = thread_cpu_time() - start;

    eprintln!("45 ms and a loop were cut after {took:?}");
    assert!(
        matches!(answer, Err(TerminationReason::CpuTimeLimit)),
        "{answer:?}"
    );
    assert!(took < Duration::from_millis(80), "{took:?}");
}

// A cut in the microtasks of an entry ends the request when the entry
// returns, not at the wall clock (3 s here)
#[tokio::test]
async fn a_cut_in_the_microtasks_ends_the_request_at_once() {
    let limits = RuntimeLimits {
        max_wall_clock_time_ms: 3000,
        ..Default::default()
    };
    let mut worker = Worker::new(Script::new(CODE), Some(limits)).await.unwrap();
    let (task, _rx) = get("/microtasks");
    let start = Instant::now();
    let answer = worker.exec(task).await;
    let took = start.elapsed();

    eprintln!("a microtask loop was cut after {took:?}");
    assert!(
        matches!(answer, Err(TerminationReason::CpuTimeLimit)),
        "{answer:?}"
    );
    assert!(took < Duration::from_millis(500), "{took:?}");
}

// The host reads the text of the thrown value: a call into guest code of its
// own, so it gets what is left of the budget (5 ms), not a whole limit
#[tokio::test]
async fn a_thrown_value_is_read_under_the_budget() {
    let mut worker = Worker::new(Script::new(CODE), None).await.unwrap();
    let (task, _rx) = get("/throw");
    let start = thread_cpu_time();
    let answer = worker.exec(task).await;
    let took = thread_cpu_time() - start;

    eprintln!("45 ms and a thrown value were cut after {took:?}");
    assert!(
        matches!(answer, Err(TerminationReason::CpuTimeLimit)),
        "{answer:?}"
    );
    assert!(took < Duration::from_millis(80), "{took:?}");
}

// The same at the top level of the script, under its floor of 400 ms: 300
// ms, so the throw is reached, then a toString that never ends.
// JavaScriptCore reads the text of the value for its inspector inside the
// same API call, in an entry of its own. CPU time of the thread, after a
// first worker that takes the costs of the first start.
#[tokio::test]
async fn a_value_thrown_at_the_top_level_is_read_under_the_budget() {
    Worker::new(Script::new(CODE), None).await.unwrap();

    let code = "const stop = Date.now() + 300; while (Date.now() < stop) {}
        throw { toString() { for (;;) {} } };";
    let start = thread_cpu_time();
    let made = Worker::new(Script::new(code), None).await.err();
    let took = thread_cpu_time() - start;

    assert!(
        matches!(made, Some(TerminationReason::CpuTimeLimit)),
        "{made:?}"
    );
    assert!(took < Duration::from_millis(430), "{took:?}");
}

// A guest that replaces the globals the host calls changes nothing: the host
// holds its own, made before guest code
#[tokio::test]
async fn the_host_calls_no_global_the_guest_replaced() {
    let code = "const NativeResponse = Response;
        const called = [];
        const spy = (name) => function() { called.push(name); };

        for (const name of ['Request', 'Response', 'TypeError', '__triggerFetch',
                            '__runCallback', '__storeCallback', '__clearCallbacks']) {
            globalThis[name] = spy(name);
        }

        JSON.parse = spy('JSON.parse');

        addEventListener('fetch', (event) => {
            event.respondWith(new Promise((resolve) => {
                setTimeout(() => resolve(new NativeResponse(called.join(',') || 'none')), 0);
            }));
        });";
    let mut worker = Worker::new(Script::new(code), None).await.unwrap();
    let (task, rx) = get("/");

    worker.exec(task).await.unwrap();
    assert_eq!(body(rx).await, "none");

    worker.reset().unwrap();

    let (task, rx) = get("/");

    worker.exec(task).await.unwrap();
    assert_eq!(body(rx).await, "none");
}

// Worker::evaluate runs under what is left of the budget, and a cut there
// ends the worker as any other. Right after the load, that is the budget of
// a request, not what is left of the 400 ms of the load (402 ms without the
// refill after the load).
#[tokio::test]
async fn evaluate_is_under_the_budget() {
    let mut worker = Worker::new(Script::new(CODE), None).await.unwrap();
    let start = thread_cpu_time();
    let answer = worker.evaluate("for (;;) {}");
    let took = thread_cpu_time() - start;

    assert!(answer.unwrap_err().contains("CpuTimeLimit"));
    assert!(took < Duration::from_millis(80), "{took:?}");

    let (task, _rx) = get("/");

    assert!(matches!(
        worker.exec(task).await,
        Err(TerminationReason::CpuTimeLimit)
    ));
}

/// A top level that spins `ms` milliseconds
fn top_level_of(ms: u64) -> String {
    format!("const stop = Date.now() + {ms}; while (Date.now() < stop) {{}}")
}

// The top level gets the budget of a request or 400 ms, the larger: a bundle
// that needs 100 ms to start makes a worker under a budget of 50 ms
#[tokio::test]
async fn a_top_level_gets_at_least_400_ms() {
    let made = Worker::new(Script::new(top_level_of(100).as_str()), None).await;

    assert!(made.is_ok(), "{:?}", made.err());

    let made = Worker::new(Script::new(top_level_of(500).as_str()), None)
        .await
        .err();

    assert!(
        matches!(made, Some(TerminationReason::CpuTimeLimit)),
        "{made:?}"
    );
}

// The host reads the text of a value that a callback threw even when logging
// is off: a log macro skips its arguments then, and the read must run under
// the budget in every build. The system framework reads the text once for its
// inspector, the host once more: 2 reads (1 when the host read was skipped).
#[tokio::test]
async fn a_thrown_value_is_read_with_logging_off() {
    log::set_max_level(log::LevelFilter::Off);

    let code = "globalThis.reads = 0;
        addEventListener('fetch', (event) => {
            setTimeout(() => { throw { toString() { globalThis.reads++; return 'thrown'; } }; }, 0);
            setTimeout(() => event.respondWith(new Response('ok')), 5);
        });";
    let mut worker = Worker::new(Script::new(code), None).await.unwrap();
    let (task, rx) = get("/");

    worker.exec(task).await.unwrap();
    assert_eq!(body(rx).await, "ok");

    assert_eq!(worker.evaluate("globalThis.reads").unwrap(), "2");
}
