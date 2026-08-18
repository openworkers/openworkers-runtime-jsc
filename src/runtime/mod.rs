mod base64;
pub mod bindings;
mod crypto;
pub mod fetch;
mod headers;
mod request;
mod response;
pub mod stream_manager;
mod streams;
mod text_encoding;
mod url;

// Re-export fetch functions for internal use
pub use fetch::parse_fetch_options;

use openworkers_core::{HttpRequest, HttpResponseMeta};
use rusty_jsc::{JSContext, JSValue};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Unique ID for callbacks
pub type CallbackId = u64;

/// Read a thrown value, which itself may throw
pub(crate) fn error_text(context: &JSContext, error: &JSValue) -> String {
    match error.to_js_string(context) {
        Ok(text) => text.to_string(),
        Err(_) => "unknown error".to_string(),
    }
}

/// Copy out the bytes of an ArrayBuffer or of any typed array view,
/// or None for a value that holds no buffer
pub(crate) fn typed_array_bytes(context: &JSContext, value: &JSValue) -> Option<bytes::Bytes> {
    use rusty_jsc::private;

    let mut exception: private::JSValueRef = std::ptr::null_mut();

    let kind = unsafe {
        private::JSValueGetTypedArrayType(context.get_ref(), value.get_ref(), &mut exception)
    };

    if kind == private::JSTypedArrayType_kJSTypedArrayTypeNone {
        return None;
    }

    let object = value.to_object(context).ok()?;

    if kind == private::JSTypedArrayType_kJSTypedArrayTypeArrayBuffer {
        return object
            .get_array_buffer(context)
            .ok()
            .map(|buffer| bytes::Bytes::copy_from_slice(buffer));
    }

    let object = rusty_jsc::private::JSObjectRef::from(object.clone());

    // The pointer is the start of the backing buffer, so a view has to be
    // taken from its own offset, or else it reads the wrong bytes
    let bytes = unsafe {
        let start =
            private::JSObjectGetTypedArrayBytesPtr(context.get_ref(), object, &mut exception)
                as *const u8;

        if start.is_null() {
            return None;
        }

        let offset =
            private::JSObjectGetTypedArrayByteOffset(context.get_ref(), object, &mut exception);
        let length =
            private::JSObjectGetTypedArrayByteLength(context.get_ref(), object, &mut exception);

        std::slice::from_raw_parts(start.add(offset as usize), length as usize)
    };

    Some(bytes::Bytes::copy_from_slice(bytes))
}

/// Copy bytes into a JS-owned Uint8Array, so JS may outlive `bytes`
pub(crate) fn new_uint8_array(context: &mut JSContext, bytes: &[u8]) -> Result<JSValue, JSValue> {
    let script = format!("new Uint8Array({})", bytes.len());
    let array = context.evaluate_script(&script, 1)?;

    if bytes.is_empty() {
        return Ok(array);
    }

    let array_obj = array.to_object(context)?;

    // SAFETY: the pointer stays valid as long as no JSC call runs, and none does here
    let buffer = unsafe { array_obj.get_typed_array_buffer(context)? };
    buffer.copy_from_slice(bytes);

    Ok(array)
}

/// Message sent from JS to schedule async operations
pub enum SchedulerMessage {
    /// Schedule a timeout: (callback_id, delay_ms)
    ScheduleTimeout(CallbackId, u64),
    /// Schedule an interval: (callback_id, interval_ms)
    ScheduleInterval(CallbackId, u64),
    /// Clear a timer (timeout or interval): (callback_id)
    ClearTimer(CallbackId),
    /// Fetch with streaming response: (promise_id, request)
    FetchStreaming(CallbackId, HttpRequest),
    /// Read next chunk from stream: (callback_id, stream_id)
    StreamRead(CallbackId, stream_manager::StreamId),
    /// Cancel/close a stream
    StreamCancel(stream_manager::StreamId),
    /// Shutdown the event loop
    Shutdown,
}

/// Message sent back from the event loop to execute callbacks
pub enum CallbackMessage {
    /// Execute a timeout callback (one-shot)
    ExecuteTimeout(CallbackId),
    /// Execute an interval callback (repeating)
    ExecuteInterval(CallbackId),
    /// Execute a Promise resolve callback with string result
    ExecutePromiseResolve(CallbackId, String),
    /// Execute a Promise reject callback with error
    ExecutePromiseReject(CallbackId, String),
    /// Reject a fetch Promise with error
    FetchError(CallbackId, String),
    /// Fetch streaming success: metadata + stream ID
    FetchStreamingSuccess(CallbackId, HttpResponseMeta, stream_manager::StreamId),
    /// Stream chunk ready
    StreamChunk(CallbackId, stream_manager::StreamChunk),
}

/// Runtime that manages JSContext and tokio event loop
pub struct Runtime {
    /// JavaScript context
    pub context: JSContext,
    /// Channel to send scheduler messages to the event loop
    pub scheduler_tx: mpsc::UnboundedSender<SchedulerMessage>,
    /// Channel to receive callback messages from the event loop
    pub callback_rx: mpsc::UnboundedReceiver<CallbackMessage>,
    /// Stream manager for handling streaming responses
    pub(crate) stream_manager: Arc<stream_manager::StreamManager>,
}

impl Runtime {
    pub fn new() -> (
        Self,
        mpsc::UnboundedReceiver<SchedulerMessage>,
        mpsc::UnboundedSender<CallbackMessage>,
        Arc<stream_manager::StreamManager>,
    ) {
        let (scheduler_tx, scheduler_rx) = mpsc::unbounded_channel();
        let (callback_tx, callback_rx) = mpsc::unbounded_channel();

        let next_callback_id: bindings::CallbackCounter = Rc::new(std::cell::Cell::new(1));
        let stream_manager = Arc::new(stream_manager::StreamManager::new());

        let mut context = JSContext::default();

        // Setup the registry every pending callback is stored in
        bindings::setup_callback_registry(&mut context);

        // Setup queueMicrotask
        bindings::setup_microtask(&mut context);

        // Setup TextEncoder/TextDecoder
        text_encoding::setup_text_encoding(&mut context);

        // Setup atob/btoa (depends on TextEncoder/TextDecoder)
        base64::setup_base64(&mut context);

        // Setup ReadableStream
        streams::setup_readable_stream(&mut context);

        // Setup Headers (before Response)
        headers::setup_headers(&mut context);

        // Setup Response (uses ReadableStream and Headers)
        response::setup_response(&mut context);

        // Setup Request (uses ReadableStream, Headers, TextEncoder)
        request::setup_request(&mut context);

        // Setup URL API
        url::setup_url_api(&mut context);

        // Setup crypto API
        crypto::setup_crypto(&mut context);

        // Setup fetch API
        bindings::setup_fetch(&mut context, scheduler_tx.clone(), next_callback_id.clone());

        // Setup timer bindings (pass shared state)
        bindings::setup_timer(&mut context, scheduler_tx.clone(), next_callback_id.clone());

        // Setup stream operations for native streaming
        bindings::setup_stream_ops(&mut context, scheduler_tx.clone(), next_callback_id);

        // Setup response stream operations for streaming all responses
        bindings::setup_response_stream_ops(&mut context, stream_manager.clone());

        let runtime = Self {
            context,
            scheduler_tx,
            callback_rx,
            stream_manager: stream_manager.clone(),
        };

        (runtime, scheduler_rx, callback_tx, stream_manager)
    }

    /// Process pending callbacks (non-blocking)
    pub fn process_callbacks(&mut self) {
        while let Ok(msg) = self.callback_rx.try_recv() {
            self.handle_callback(msg);
        }
    }

    pub fn handle_callback(&mut self, msg: CallbackMessage) {
        match msg {
            CallbackMessage::ExecuteTimeout(callback_id) => self.run_callback(callback_id, &[]),
            CallbackMessage::ExecuteInterval(callback_id) => {
                // Intervals stay registered until they are cleared
                self.call_registry("__runRepeatingCallback", callback_id, &[])
            }
            CallbackMessage::ExecutePromiseResolve(callback_id, result)
            | CallbackMessage::ExecutePromiseReject(callback_id, result)
            | CallbackMessage::FetchError(callback_id, result) => {
                let value = JSValue::string(&self.context, result.as_str());

                self.run_callback(callback_id, &[value]);
            }
            CallbackMessage::FetchStreamingSuccess(callback_id, meta, stream_id) => {
                match self.make_response(&meta, stream_id) {
                    Ok(response) => self.run_callback(callback_id, &[response]),
                    Err(e) => log::error!(
                        "Failed to create streaming Response: {}",
                        error_text(&self.context, &e)
                    ),
                }
            }
            CallbackMessage::StreamChunk(callback_id, chunk) => {
                match self.make_chunk_result(chunk) {
                    Ok(result) => self.run_callback(callback_id, &[result]),
                    Err(e) => log::error!(
                        "Failed to create stream result: {}",
                        error_text(&self.context, &e)
                    ),
                }
            }
        }
    }

    /// Run a callback and drop it; only interval ticks keep theirs
    fn run_callback(&mut self, callback_id: CallbackId, args: &[JSValue]) {
        self.call_registry("__runCallback", callback_id, args)
    }

    fn call_registry(&mut self, runner: &str, callback_id: CallbackId, args: &[JSValue]) {
        log::debug!("Executing callback {}", callback_id);

        let mut call_args = vec![JSValue::number(&self.context, callback_id as f64)];
        call_args.extend_from_slice(args);

        if let Err(e) = bindings::call_global(&self.context, runner, &call_args) {
            log::error!(
                "Callback {} failed: {}",
                callback_id,
                error_text(&self.context, &e)
            );
        }
    }

    /// Evaluate a JavaScript script
    pub fn evaluate(&mut self, script: &str) -> Result<JSValue, JSValue> {
        self.context.evaluate_script(script, 1)
    }

    fn make_response(
        &mut self,
        meta: &HttpResponseMeta,
        stream_id: stream_manager::StreamId,
    ) -> Result<JSValue, JSValue> {
        // The headers and the status text land in JS source, so JSON-encode them
        let headers_json = serde_json::to_string(&meta.headers).unwrap_or("{}".to_string());
        let status_text_json =
            serde_json::to_string(&meta.status_text).unwrap_or_else(|_| "\"\"".to_string());

        let script = format!(
            r#"new Response(__createNativeStream({}), {{
                    status: {},
                    statusText: {},
                    headers: {}
                }})"#,
            stream_id, meta.status, status_text_json, headers_json
        );

        self.context.evaluate_script(&script, 1)
    }

    fn make_chunk_result(
        &mut self,
        chunk: stream_manager::StreamChunk,
    ) -> Result<JSValue, JSValue> {
        let bytes = match chunk {
            stream_manager::StreamChunk::Data(bytes) => bytes,
            stream_manager::StreamChunk::Done => {
                return self
                    .context
                    .evaluate_script("({ done: true, value: undefined })", 1);
            }
            stream_manager::StreamChunk::Error(err) => {
                let err_json = serde_json::to_string(&err).unwrap();

                return self
                    .context
                    .evaluate_script(&format!("({{ error: {} }})", err_json), 1);
            }
        };

        let wrap = self
            .context
            .evaluate_script("(value => ({ done: false, value }))", 1)?
            .to_object(&self.context)?;

        let value = new_uint8_array(&mut self.context, &bytes)?;

        wrap.call_as_function(&self.context, None, &[value])
    }
}

/// Background event loop that handles scheduled tasks
pub async fn run_event_loop(
    mut scheduler_rx: mpsc::UnboundedReceiver<SchedulerMessage>,
    callback_tx: mpsc::UnboundedSender<CallbackMessage>,
    stream_manager: Arc<stream_manager::StreamManager>,
    ops: openworkers_core::OperationsHandle,
) {
    use std::collections::HashMap;
    use tokio::task::JoinHandle;

    log::info!("Event loop started");

    // Track running tasks so we can cancel them
    let mut running_tasks: HashMap<CallbackId, JoinHandle<()>> = HashMap::new();

    while let Some(msg) = scheduler_rx.recv().await {
        match msg {
            SchedulerMessage::ScheduleTimeout(callback_id, delay_ms) => {
                log::debug!(
                    "Scheduling timeout {} with delay {}ms",
                    callback_id,
                    delay_ms
                );

                let callback_tx = callback_tx.clone();
                let handle = tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    let _ = callback_tx.send(CallbackMessage::ExecuteTimeout(callback_id));
                });

                running_tasks.insert(callback_id, handle);
            }
            SchedulerMessage::ScheduleInterval(callback_id, interval_ms) => {
                log::debug!(
                    "Scheduling interval {} with period {}ms",
                    callback_id,
                    interval_ms
                );

                let callback_tx = callback_tx.clone();
                let handle = tokio::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_millis(interval_ms));
                    // Skip the first tick (it fires immediately)
                    interval.tick().await;

                    loop {
                        interval.tick().await;
                        if callback_tx
                            .send(CallbackMessage::ExecuteInterval(callback_id))
                            .is_err()
                        {
                            // Channel closed, stop the interval
                            break;
                        }
                    }
                });

                running_tasks.insert(callback_id, handle);
            }
            SchedulerMessage::FetchStreaming(promise_id, request) => {
                log::debug!(
                    "Fetching streaming {} {}",
                    request.method.as_str(),
                    request.url
                );

                let callback_tx = callback_tx.clone();
                let manager = stream_manager.clone();
                let ops = ops.clone();

                tokio::spawn(async move {
                    match execute_fetch_via_ops(request, manager, ops).await {
                        Ok((meta, stream_id)) => {
                            let _ = callback_tx.send(CallbackMessage::FetchStreamingSuccess(
                                promise_id, meta, stream_id,
                            ));
                        }
                        Err(e) => {
                            let _ = callback_tx.send(CallbackMessage::FetchError(promise_id, e));
                        }
                    }
                });
            }
            SchedulerMessage::StreamRead(callback_id, stream_id) => {
                log::debug!("Reading stream {} for callback {}", stream_id, callback_id);

                let callback_tx = callback_tx.clone();
                let manager = stream_manager.clone();
                tokio::spawn(async move {
                    let chunk = match manager.read_chunk(stream_id).await {
                        Ok(chunk) => chunk,
                        Err(e) => stream_manager::StreamChunk::Error(e),
                    };
                    let _ = callback_tx.send(CallbackMessage::StreamChunk(callback_id, chunk));
                });
            }
            SchedulerMessage::StreamCancel(stream_id) => {
                log::debug!("Cancelling stream {}", stream_id);
                stream_manager.close_stream(stream_id);
            }
            SchedulerMessage::ClearTimer(callback_id) => {
                log::debug!("Clearing timer {}", callback_id);

                if let Some(handle) = running_tasks.remove(&callback_id) {
                    handle.abort();
                }
            }
            SchedulerMessage::Shutdown => {
                log::info!("Shutting down event loop");

                // Abort all running tasks
                for (_, handle) in running_tasks.drain() {
                    handle.abort();
                }

                break;
            }
        }
    }
}

/// Execute fetch via OperationsHandler
async fn execute_fetch_via_ops(
    request: openworkers_core::HttpRequest,
    stream_manager: Arc<stream_manager::StreamManager>,
    ops: openworkers_core::OperationsHandle,
) -> Result<(openworkers_core::HttpResponseMeta, stream_manager::StreamId), String> {
    use openworkers_core::{Operation, OperationResult, ResponseBody};

    let result = ops.handle(Operation::Fetch(request)).await;

    let response = match result {
        OperationResult::Http(r) => r?,
        _ => return Err("Unexpected result type for fetch".into()),
    };

    let meta = openworkers_core::HttpResponseMeta {
        status: response.status,
        status_text: status_text(response.status),
        headers: response.headers,
    };

    let stream_id = stream_manager.create_stream("ops_fetch".to_string());

    match response.body {
        ResponseBody::None => {
            let _ = stream_manager
                .write_chunk(stream_id, stream_manager::StreamChunk::Done)
                .await;
        }
        ResponseBody::Bytes(bytes) => {
            let _ = stream_manager
                .write_chunk(stream_id, stream_manager::StreamChunk::Data(bytes))
                .await;
            let _ = stream_manager
                .write_chunk(stream_id, stream_manager::StreamChunk::Done)
                .await;
        }
        ResponseBody::Stream(mut rx) => {
            let manager = stream_manager.clone();

            tokio::spawn(async move {
                while let Some(result) = rx.recv().await {
                    match result {
                        Ok(bytes) => {
                            if manager
                                .write_chunk(stream_id, stream_manager::StreamChunk::Data(bytes))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(e) => {
                            let _ = manager
                                .write_chunk(stream_id, stream_manager::StreamChunk::Error(e))
                                .await;
                            break;
                        }
                    }
                }

                let _ = manager
                    .write_chunk(stream_id, stream_manager::StreamChunk::Done)
                    .await;
            });
        }
    }

    Ok((meta, stream_id))
}

/// Get HTTP status text
fn status_text(status: u16) -> String {
    match status {
        200 => "OK".to_string(),
        201 => "Created".to_string(),
        204 => "No Content".to_string(),
        301 => "Moved Permanently".to_string(),
        302 => "Found".to_string(),
        304 => "Not Modified".to_string(),
        400 => "Bad Request".to_string(),
        401 => "Unauthorized".to_string(),
        403 => "Forbidden".to_string(),
        404 => "Not Found".to_string(),
        500 => "Internal Server Error".to_string(),
        502 => "Bad Gateway".to_string(),
        503 => "Service Unavailable".to_string(),
        _ => "Unknown".to_string(),
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // Send shutdown message
        let _ = self.scheduler_tx.send(SchedulerMessage::Shutdown);
    }
}
