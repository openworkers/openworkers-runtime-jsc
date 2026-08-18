mod common;

use common::TestRunner;
use openworkers_runtime_jsc::{
    HttpRequest, HttpResponse, OpFuture, OperationsHandle, OperationsHandler, RequestBody,
    ResponseBody,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Keeps the body of the last outbound request for the assertions
#[derive(Default)]
struct RecordingOps {
    body: Mutex<Vec<u8>>,
}

impl OperationsHandler for RecordingOps {
    fn handle_fetch(&self, request: HttpRequest) -> OpFuture<'_, Result<HttpResponse, String>> {
        if let RequestBody::Bytes(bytes) = request.body {
            *self.body.lock().unwrap() = bytes.to_vec();
        }

        Box::pin(async move {
            Ok(HttpResponse {
                status: 200,
                headers: vec![],
                body: ResponseBody::None,
            })
        })
    }
}

async fn sent_body(script: &str) -> Vec<u8> {
    let ops = Arc::new(RecordingOps::default());
    let handle: OperationsHandle = ops.clone();
    let mut runner = TestRunner::new_with_ops(handle);

    runner.execute(script).expect("Script should execute");
    runner.process_for(Duration::from_millis(300)).await;
    runner.shutdown().await;

    ops.body.lock().unwrap().clone()
}

#[tokio::test]
async fn test_typed_array_body_is_sent_as_bytes() {
    let script = r#"
        fetch('https://example.com/post', {
            method: 'POST',
            body: new Uint8Array([0, 1, 255, 65])
        });
    "#;

    assert_eq!(sent_body(script).await, vec![0, 1, 255, 65]);
}

#[tokio::test]
async fn test_view_body_is_sent_from_its_offset() {
    let script = r#"
        const buffer = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]).buffer;

        fetch('https://example.com/post', {
            method: 'POST',
            body: new Uint8Array(buffer, 4, 3)
        });
    "#;

    assert_eq!(sent_body(script).await, vec![5, 6, 7]);
}

#[tokio::test]
async fn test_array_buffer_body_is_sent_as_bytes() {
    let script = r#"
        fetch('https://example.com/post', {
            method: 'POST',
            body: new Uint8Array([9, 8, 7]).buffer
        });
    "#;

    assert_eq!(sent_body(script).await, vec![9, 8, 7]);
}

/// A stream body used to be decoded as text, which mangles anything not UTF-8
#[tokio::test]
async fn test_stream_body_is_sent_as_bytes() {
    let script = r#"
        const stream = new ReadableStream({
            start(controller) {
                controller.enqueue(new Uint8Array([0, 159, 65]));
                controller.close();
            }
        });

        fetch('https://example.com/post', { method: 'POST', body: stream });
    "#;

    assert_eq!(sent_body(script).await, vec![0, 159, 65]);
}

#[tokio::test]
async fn test_string_body_is_sent_as_utf8() {
    let script = r#"
        fetch('https://example.com/post', { method: 'POST', body: 'h\u00e9llo' });
    "#;

    assert_eq!(sent_body(script).await, "h\u{e9}llo".as_bytes());
}
