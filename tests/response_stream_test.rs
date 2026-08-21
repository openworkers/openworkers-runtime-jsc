use bytes::Bytes;
use openworkers_core::{
    Event, HttpMethod, HttpRequest, HttpResponse, RequestBody, ResponseBody, Script,
    TerminationReason,
};
use openworkers_runtime_jsc::Worker;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

type Exec<'a> = Pin<&'a mut (dyn Future<Output = Result<(), TerminationReason>> + 'a)>;
type ExecResult = Option<Result<(), TerminationReason>>;

fn request(path: &str) -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Get,
        url: format!("https://example.com{}", path),
        headers: HashMap::new(),
        body: RequestBody::None,
    }
}

/// Runs `fut` while `exec` keeps making progress
///
/// The guest only runs while `exec` is polled, so a test that awaited it before
/// reading the body would stall against the body's own backpressure.
async fn while_running<F: Future>(exec: &mut Exec<'_>, done: &mut ExecResult, fut: F) -> F::Output {
    tokio::pin!(fut);

    loop {
        tokio::select! {
            biased;

            output = &mut fut => return output,
            result = exec.as_mut(), if done.is_none() => *done = Some(result),
        }
    }
}

/// Everything the host saw of one response
struct Seen {
    response: HttpResponse,
    chunks: Vec<Bytes>,
    error: Option<String>,
}

async fn fetch(worker: &mut Worker, path: &str) -> Seen {
    let (event, head_rx) = Event::fetch(request(path));
    let future = worker.exec(event);

    tokio::pin!(future);

    let mut exec: Exec<'_> = future;
    let mut done: ExecResult = None;

    let response = while_running(&mut exec, &mut done, head_rx)
        .await
        .expect("no response");

    let mut seen = Seen {
        chunks: Vec::new(),
        error: None,
        response: HttpResponse {
            status: response.status,
            headers: response.headers,
            body: ResponseBody::None,
        },
    };

    let ResponseBody::Stream(mut body) = response.body else {
        panic!("the body was not a stream");
    };

    loop {
        match while_running(&mut exec, &mut done, body.recv()).await {
            Some(Ok(bytes)) => seen.chunks.push(bytes),
            Some(Err(e)) => {
                seen.error = Some(e);

                break;
            }
            None => break,
        }
    }

    seen
}

fn text(seen: &Seen) -> String {
    let mut out = Vec::new();

    for chunk in &seen.chunks {
        out.extend_from_slice(chunk);
    }

    String::from_utf8(out).expect("the body is utf-8")
}

/// A guest that spaces its chunks out in time has to reach the host and end,
/// or else nobody closes the channel and the host waits out its own deadline
#[tokio::test]
async fn test_a_paced_response_stream_ends() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const encoder = new TextEncoder();
            let sent = 0;

            const stream = new ReadableStream({
                start(controller) {
                    function step() {
                        if (sent >= 4) {
                            controller.close();
                            return;
                        }

                        controller.enqueue(encoder.encode('chunk' + sent));
                        sent += 1;
                        setTimeout(step, 20);
                    }

                    setTimeout(step, 20);
                }
            });

            event.respondWith(new Response(stream));
        });
    "#;

    let mut worker = Worker::new(Script::new(script), None)
        .await
        .expect("Worker should initialize");

    let seen = fetch(&mut worker, "/").await;

    assert_eq!(seen.response.status, 200);
    assert_eq!(seen.chunks.len(), 4);
    assert_eq!(text(&seen), "chunk0chunk1chunk2chunk3");
}

/// More chunks than the pipeline holds, so the guest has to wait for the reader
/// rather than drop what does not fit
#[tokio::test]
async fn test_a_response_longer_than_the_buffer_keeps_every_chunk() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const encoder = new TextEncoder();
            let sent = 0;

            const stream = new ReadableStream({
                pull(controller) {
                    if (sent >= 64) {
                        controller.close();
                        return;
                    }

                    controller.enqueue(encoder.encode(sent + ','));
                    sent += 1;
                }
            });

            event.respondWith(new Response(stream));
        });
    "#;

    let mut worker = Worker::new(Script::new(script), None)
        .await
        .expect("Worker should initialize");

    let seen = fetch(&mut worker, "/").await;
    let expected: String = (0..64).map(|i| format!("{},", i)).collect();

    assert_eq!(seen.chunks.len(), 64);
    assert_eq!(text(&seen), expected);
}

/// A guest that errors its own stream must not read as a body that just stopped
#[tokio::test]
async fn test_a_guest_stream_error_reaches_the_host() {
    let script = r#"
        addEventListener('fetch', (event) => {
            const encoder = new TextEncoder();
            let sent = 0;

            const stream = new ReadableStream({
                pull(controller) {
                    if (sent >= 2) {
                        controller.error(new Error('guest gave up'));
                        return;
                    }

                    controller.enqueue(encoder.encode('chunk' + sent));
                    sent += 1;
                }
            });

            event.respondWith(new Response(stream));
        });
    "#;

    let mut worker = Worker::new(Script::new(script), None)
        .await
        .expect("Worker should initialize");

    let seen = fetch(&mut worker, "/").await;

    assert_eq!(text(&seen), "chunk0chunk1");
    assert_eq!(seen.error.as_deref(), Some("guest gave up"));
}

/// A client that walks away mid-stream leaves the worker usable
#[tokio::test]
async fn test_the_worker_survives_a_client_that_hangs_up() {
    let script = r#"
        addEventListener('fetch', (event) => {
            if (new URL(event.request.url).pathname === '/short') {
                event.respondWith(new Response('short'));
                return;
            }

            const encoder = new TextEncoder();
            let sent = 0;

            const stream = new ReadableStream({
                start(controller) {
                    function step() {
                        if (sent >= 200) {
                            controller.close();
                            return;
                        }

                        try {
                            controller.enqueue(encoder.encode('chunk' + sent));
                        } catch (e) {
                            return;
                        }

                        sent += 1;
                        setTimeout(step, 10);
                    }

                    setTimeout(step, 10);
                }
            });

            event.respondWith(new Response(stream));
        });
    "#;

    let mut worker = Worker::new(Script::new(script), None)
        .await
        .expect("Worker should initialize");

    {
        let (event, head_rx) = Event::fetch(request("/long"));
        let future = worker.exec(event);

        tokio::pin!(future);

        let mut exec: Exec<'_> = future;
        let mut done: ExecResult = None;

        let response = while_running(&mut exec, &mut done, head_rx)
            .await
            .expect("no response");

        let ResponseBody::Stream(mut body) = response.body else {
            panic!("the body was not a stream");
        };

        let mut taken = 0;

        while taken < 3 {
            match while_running(&mut exec, &mut done, body.recv()).await {
                Some(Ok(_)) => taken += 1,
                _ => break,
            }
        }

        assert_eq!(taken, 3);

        drop(body);

        let ended = tokio::time::timeout(Duration::from_secs(5), exec).await;

        assert!(
            matches!(ended, Ok(Ok(()))),
            "exec should return once the client is gone, got {:?}",
            ended
        );
    }

    let seen = fetch(&mut worker, "/short").await;

    assert_eq!(text(&seen), "short");
}

/// A body this backend cannot read has to say so, or else the guest answers 200
/// on an empty body and the caller cannot tell the difference
#[tokio::test]
async fn test_a_streaming_request_body_is_refused() {
    let script = "addEventListener('fetch', (e) => e.respondWith(new Response('ok')));";

    let mut worker = Worker::new(Script::new(script), None)
        .await
        .expect("Worker should initialize");

    let (_tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, String>>(1);

    let (event, _head_rx) = Event::fetch(HttpRequest {
        method: HttpMethod::Post,
        url: "https://example.com/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::Stream(rx),
    });

    let result = worker.exec(event).await;

    let Err(TerminationReason::Other(message)) = result else {
        panic!(
            "a streaming request body should be refused, got {:?}",
            result
        );
    };

    assert!(
        message.contains("Streaming request bodies are not supported"),
        "the refusal should say what is unsupported, got {:?}",
        message
    );
}
