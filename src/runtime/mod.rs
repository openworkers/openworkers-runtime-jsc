mod base64;
pub mod bindings;
mod crypto;
pub mod fetch;
mod form_data;
mod headers;
mod request;
mod response;
pub mod stream_manager;
mod streams;
mod text_encoding;

#[cfg(feature = "static")]
mod timers;

mod url;

// Re-export fetch functions for internal use
pub use fetch::parse_fetch_options;

use crate::cpu_limit::CpuLimit;
use openworkers_core::{HttpRequest, HttpResponseMeta, TerminationReason};
use openworkers_jsc::Context;
use openworkers_jsc::Object;
use openworkers_jsc::TypedArrayType;
use openworkers_jsc::Value;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Unique ID for callbacks
pub type CallbackId = u64;

/// Read a thrown value, which itself may throw
pub(crate) fn error_text(context: &Context, error: &Value<'_>) -> String {
    error
        .to_string(context)
        .unwrap_or_else(|_| "unknown error".to_string())
}

/// Copy out the bytes of an ArrayBuffer or of any typed array view,
/// or None for a value that holds no buffer
pub(crate) fn typed_array_bytes(context: &Context, value: &Value<'_>) -> Option<bytes::Bytes> {
    let Some(kind) = value.typed_array_type(context) else {
        return data_view_bytes(context, value);
    };

    let object = value.to_object(context).ok()?;

    // SAFETY: the bytes are copied before any other call into JSC
    let bytes = unsafe {
        match kind {
            TypedArrayType::ArrayBuffer => object.array_buffer_bytes(context),
            _ => object.typed_array_bytes(context),
        }
    };

    bytes.ok().map(|bytes| bytes::Bytes::copy_from_slice(bytes))
}

/// JSC reports no typed array type for a DataView, so read it through its own
/// properties; anything else lacks them and falls out as None
fn data_view_bytes(context: &Context, value: &Value<'_>) -> Option<bytes::Bytes> {
    let view = value.to_object(context).ok()?;
    let buffer = view
        .get_property(context, "buffer")
        .ok()?
        .to_object(context)
        .ok()?;
    let offset = view
        .get_property(context, "byteOffset")
        .ok()?
        .to_number(context)
        .ok()? as usize;
    let length = view
        .get_property(context, "byteLength")
        .ok()?
        .to_number(context)
        .ok()? as usize;

    // SAFETY: the bytes are copied before any other call into JSC
    let bytes = unsafe { buffer.array_buffer_bytes(context) }.ok()?;
    let end = offset.checked_add(length)?;

    bytes.get(offset..end).map(bytes::Bytes::copy_from_slice)
}

/// Copy bytes into a JS-owned Uint8Array, so JS may outlive `bytes`. It runs
/// no JavaScript, so a Uint8Array that the guest replaced is not called.
pub(crate) fn new_uint8_array<'c>(
    context: &'c Context,
    bytes: &[u8],
) -> Result<Value<'c>, Value<'c>> {
    Object::new_uint8_array(context, bytes).map(Value::from)
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
    /// Call a function of the host: (callback_id, binding, command, arguments as JSON)
    #[cfg(feature = "invoke")]
    Invoke(CallbackId, String, String, String),
    /// Drop every timer and task of the event that ended
    ClearAll,
    /// Shutdown the event loop
    Shutdown,
}

/// Message sent back from the event loop to execute callbacks
pub enum CallbackMessage {
    /// Execute a timeout callback (one-shot)
    ExecuteTimeout(CallbackId),
    /// Execute an interval callback (repeating)
    ExecuteInterval(CallbackId),
    /// Reject a fetch Promise with error
    FetchError(CallbackId, String),
    /// Fetch streaming success: metadata + stream ID
    FetchStreamingSuccess(CallbackId, HttpResponseMeta, stream_manager::StreamId),
    /// Stream chunk ready
    StreamChunk(CallbackId, stream_manager::StreamChunk),
    /// A host function answered: the call settles with `Ok(JSON)` or `Err(message)`
    #[cfg(feature = "invoke")]
    InvokeSettled(CallbackId, Result<String, String>),
}

/// Runtime that manages Context and tokio event loop
pub struct Runtime {
    /// JavaScript context
    pub context: Context,
    /// Channel to send scheduler messages to the event loop
    pub scheduler_tx: mpsc::UnboundedSender<SchedulerMessage>,
    /// Channel to receive callback messages from the event loop
    pub callback_rx: mpsc::UnboundedReceiver<CallbackMessage>,
    /// Stream manager for handling streaming responses
    pub(crate) stream_manager: Arc<stream_manager::StreamManager>,
    /// The CPU budget of each call into JavaScript; none until a worker sets one
    pub(crate) cpu: CpuLimit,
    /// The WTF timers of this thread
    #[cfg(feature = "static")]
    timers: Arc<timers::Timers>,
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

        // Before the first context of the thread
        #[cfg(feature = "static")]
        let timers = timers::host();

        let context = Context::new();

        // Setup the registry every pending callback is stored in
        bindings::setup_callback_registry(&context);

        // Setup queueMicrotask
        bindings::setup_microtask(&context);

        // Setup TextEncoder/TextDecoder
        text_encoding::setup_text_encoding(&context);

        // Setup atob/btoa (depends on TextEncoder/TextDecoder)
        base64::setup_base64(&context);

        // Setup ReadableStream
        streams::setup_readable_stream(&context);

        // Setup Headers (before Response)
        headers::setup_headers(&context);

        // Setup FormData (used by Request/Response body decoding)
        form_data::setup_form_data(&context);

        // Setup Response (uses ReadableStream and Headers)
        response::setup_response(&context);

        // Setup Request (uses ReadableStream, Headers, TextEncoder)
        request::setup_request(&context);

        // Setup URL API
        url::setup_url_api(&context);

        // Setup crypto API
        crypto::setup_crypto(&context);

        // Setup fetch API
        bindings::setup_fetch(&context, scheduler_tx.clone(), next_callback_id.clone());

        // Setup timer bindings (pass shared state)
        bindings::setup_timer(&context, scheduler_tx.clone(), next_callback_id.clone());

        #[cfg(feature = "invoke")]
        bindings::setup_invoke(&context, scheduler_tx.clone(), next_callback_id.clone());

        // Setup stream operations for native streaming
        bindings::setup_stream_ops(&context, scheduler_tx.clone(), next_callback_id);

        // Setup response stream operations for streaming all responses
        bindings::setup_response_stream_ops(&context, stream_manager.clone());

        // Last, so it holds the builtins made above, and before guest code
        bindings::setup_host_builtins(&context);

        let cpu = CpuLimit::new(&context, 0).expect("no limit always works");

        let runtime = Self {
            context,
            scheduler_tx,
            callback_rx,
            stream_manager: stream_manager.clone(),
            cpu,
            #[cfg(feature = "static")]
            timers,
        };

        (runtime, scheduler_rx, callback_tx, stream_manager)
    }

    /// Completes when a WTF timer is due. Never in the system build, whose
    /// timers wait on a CFRunLoop that nothing runs.
    pub(crate) fn timers_due(&self) -> impl Future<Output = ()> + 'static {
        #[cfg(feature = "static")]
        {
            let timers = self.timers.clone();

            async move { timers.due().await }
        }

        #[cfg(not(feature = "static"))]
        std::future::pending()
    }

    /// Fires the WTF timers that are due, under the CPU budget, since a
    /// timer can run guest code (FinalizationRegistry callbacks).
    pub(crate) fn fire_timers(&self) -> Result<(), TerminationReason> {
        #[cfg(feature = "static")]
        while let Some(timer) = self.timers.take_due() {
            // SAFETY: this is the host thread, between calls into JSC; the
            // host got the timer from schedule and no cancel since
            self.cpu.run(|| unsafe { timer.fire() })?;
        }

        Ok(())
    }

    /// Process pending callbacks (non-blocking), until the CPU budget is spent
    pub fn process_callbacks(&mut self) {
        while let Ok(msg) = self.callback_rx.try_recv() {
            if self.handle_callback(msg).is_err() {
                return;
            }
        }
    }

    /// Runs the callback of one message. An error means the CPU budget is
    /// spent: the event must end.
    pub fn handle_callback(&self, msg: CallbackMessage) -> Result<(), TerminationReason> {
        let context = &self.context;

        let (runner, callback_id, made) = match msg {
            CallbackMessage::ExecuteTimeout(callback_id) => {
                ("__runCallback", callback_id, Ok(vec![]))
            }
            CallbackMessage::ExecuteInterval(callback_id) => {
                // Intervals stay registered until they are cleared
                ("__runRepeatingCallback", callback_id, Ok(vec![]))
            }
            CallbackMessage::FetchError(callback_id, message) => {
                let message = Value::string(context, message.as_str());
                let error = self.call("__hostTypeError", &[message])?;

                ("__runCallback", callback_id, error.map(|e| vec![e]))
            }
            CallbackMessage::FetchStreamingSuccess(callback_id, meta, stream_id) => {
                let response = self.make_response(&meta, stream_id)?;

                ("__runCallback", callback_id, response.map(|r| vec![r]))
            }
            #[cfg(feature = "invoke")]
            CallbackMessage::InvokeSettled(callback_id, answer) => {
                let (ok, payload) = match answer {
                    Ok(json) => (true, json),
                    Err(message) => (false, message),
                };
                let ok = Value::boolean(context, ok);
                let payload = Value::string(context, payload.as_str());

                ("__runCallback", callback_id, Ok(vec![ok, payload]))
            }
            CallbackMessage::StreamChunk(callback_id, chunk) => {
                let result = self.make_chunk_result(chunk)?;

                ("__runCallback", callback_id, result.map(|r| vec![r]))
            }
        };

        let args = match made {
            Ok(args) => args,
            Err(e) => {
                // Read outside the log macro, which skips its arguments when
                // logging is off: the read is a call under the budget
                let text = self.value_text(&e)?;

                log::error!("Failed to make the arguments of callback {callback_id}: {text}");

                return Ok(());
            }
        };

        log::debug!("Executing callback {}", callback_id);

        let mut call_args = vec![Value::number(&self.context, callback_id as f64)];
        call_args.extend(args);

        if let Err(e) = self.call(runner, &call_args)? {
            let text = self.value_text(&e)?;

            log::error!("Callback {callback_id} failed: {text}");
        }

        Ok(())
    }

    /// Calls a function that the host set on the global object before guest
    /// code, under the CPU budget. The outer error is a spent budget, the
    /// inner one a thrown value.
    pub(crate) fn call(
        &self,
        name: &str,
        args: &[Value],
    ) -> Result<Result<Value<'_>, Value<'_>>, TerminationReason> {
        self.cpu
            .run(|| bindings::call_global(&self.context, name, args))
    }

    /// The text of a value of the guest, such as a thrown one. Reading it can
    /// run guest code (toString), so it is a call under the CPU budget too.
    pub(crate) fn value_text(&self, error: &Value<'_>) -> Result<String, TerminationReason> {
        self.cpu.run(|| error_text(&self.context, error))
    }

    /// Evaluate a JavaScript script, with no CPU budget
    pub fn evaluate(&self, script: &str) -> Result<Value<'_>, Value<'_>> {
        self.context.evaluate_script(script)
    }

    fn make_response(
        &self,
        meta: &HttpResponseMeta,
        stream_id: stream_manager::StreamId,
    ) -> Result<Result<Value<'_>, Value<'_>>, TerminationReason> {
        let headers_json = serde_json::to_string(&meta.headers).unwrap_or("{}".to_string());
        let context = &self.context;
        let args = [
            Value::number(context, stream_id as f64),
            Value::number(context, meta.status as f64),
            Value::string(context, meta.status_text.as_str()),
            Value::string(context, headers_json.as_str()),
        ];

        self.call("__hostResponse", &args)
    }

    fn make_chunk_result(
        &self,
        chunk: stream_manager::StreamChunk,
    ) -> Result<Result<Value<'_>, Value<'_>>, TerminationReason> {
        let args = match chunk {
            stream_manager::StreamChunk::Data(bytes) => {
                match new_uint8_array(&self.context, &bytes) {
                    Ok(value) => [Value::string(&self.context, "data"), value],
                    Err(e) => return Ok(Err(e)),
                }
            }
            stream_manager::StreamChunk::Done => [
                Value::string(&self.context, "done"),
                Value::undefined(&self.context),
            ],
            stream_manager::StreamChunk::Error(err) => [
                Value::string(&self.context, "error"),
                Value::string(&self.context, err.as_str()),
            ],
        };

        self.call("__hostChunk", &args)
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

    // Fetch, invoke and stream reads: cancelled with the event that made them
    let mut tasks: Vec<JoinHandle<()>> = Vec::new();

    while let Some(msg) = scheduler_rx.recv().await {
        match msg {
            SchedulerMessage::ClearAll => {
                for (_, handle) in running_tasks.drain() {
                    handle.abort();
                }

                for handle in tasks.drain(..) {
                    handle.abort();
                }
            }
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

                // A timeout that fires is never cleared, so its handle would sit
                // here for the whole life of the worker
                running_tasks.retain(|_, handle| !handle.is_finished());
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

                let handle = tokio::spawn(async move {
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

                tasks.retain(|task| !task.is_finished());
                tasks.push(handle);
            }
            #[cfg(feature = "invoke")]
            SchedulerMessage::Invoke(callback_id, binding, command, args) => {
                let callback_tx = callback_tx.clone();
                let ops = ops.clone();

                let handle = tokio::spawn(async move {
                    let answer = match serde_json::from_str(&args) {
                        Ok(args) => match ops.handle_binding_invoke(&binding, &command, args).await
                        {
                            openworkers_core::InvokeResult::Value(value) => Ok(value.to_string()),
                            openworkers_core::InvokeResult::Error(message) => Err(message),
                        },
                        Err(e) => Err(format!("bad invoke arguments: {e}")),
                    };

                    let _ = callback_tx.send(CallbackMessage::InvokeSettled(callback_id, answer));
                });

                tasks.retain(|task| !task.is_finished());
                tasks.push(handle);
            }
            SchedulerMessage::StreamRead(callback_id, stream_id) => {
                log::debug!("Reading stream {} for callback {}", stream_id, callback_id);

                let callback_tx = callback_tx.clone();
                let manager = stream_manager.clone();
                let handle = tokio::spawn(async move {
                    let chunk = match manager.read_chunk(stream_id).await {
                        Ok(chunk) => chunk,
                        Err(e) => stream_manager::StreamChunk::Error(e),
                    };
                    let _ = callback_tx.send(CallbackMessage::StreamChunk(callback_id, chunk));
                });

                tasks.retain(|task| !task.is_finished());
                tasks.push(handle);
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
