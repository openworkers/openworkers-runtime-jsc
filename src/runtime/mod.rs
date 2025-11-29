mod base64;
pub mod bindings;
pub mod fetch;
mod headers;
mod request;
mod response;
pub mod stream_manager;
mod streams;
mod text_encoding;
mod url;

// Re-export fetch functions for internal use
pub use fetch::{execute_fetch_streaming, parse_fetch_options};

use openworkers_core::{HttpRequest, HttpResponseMeta};
use rusty_jsc::{JSContext, JSObject, JSValue};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// Unique ID for callbacks
pub type CallbackId = u64;

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
    /// Stored callbacks (callback_id -> JSObject function) - shared with bindings
    pub(crate) callbacks: Arc<Mutex<HashMap<CallbackId, JSObject>>>,
    /// Next callback ID - shared with bindings
    #[allow(dead_code)]
    pub(crate) next_callback_id: Arc<Mutex<CallbackId>>,
    /// Track which callbacks are intervals (vs timeouts) - shared with bindings
    pub(crate) intervals: Arc<Mutex<std::collections::HashSet<CallbackId>>>,
    /// Sender for fetch response (set during fetch execution)
    pub(crate) fetch_response_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<String>>>>,
    /// Stream manager for handling streaming responses
    #[allow(dead_code)]
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

        let callbacks: Arc<Mutex<HashMap<CallbackId, JSObject>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let next_callback_id: Arc<Mutex<CallbackId>> = Arc::new(Mutex::new(1));
        let intervals: Arc<Mutex<std::collections::HashSet<CallbackId>>> =
            Arc::new(Mutex::new(std::collections::HashSet::new()));
        let fetch_response_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<String>>>> =
            Arc::new(Mutex::new(None));
        let stream_manager = Arc::new(stream_manager::StreamManager::new());

        let mut context = JSContext::default();

        // Setup console.log
        bindings::setup_console(&mut context);

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

        // Setup fetch API
        bindings::setup_fetch(
            &mut context,
            scheduler_tx.clone(),
            callbacks.clone(),
            next_callback_id.clone(),
        );

        // Setup timer bindings (pass shared state)
        bindings::setup_timer(
            &mut context,
            scheduler_tx.clone(),
            callbacks.clone(),
            next_callback_id.clone(),
            intervals.clone(),
        );

        // Setup stream operations for native streaming
        bindings::setup_stream_ops(
            &mut context,
            scheduler_tx.clone(),
            callbacks.clone(),
            next_callback_id.clone(),
        );

        // Setup response stream operations for streaming all responses
        bindings::setup_response_stream_ops(&mut context, stream_manager.clone());

        let runtime = Self {
            context,
            scheduler_tx,
            callback_rx,
            callbacks,
            next_callback_id,
            intervals,
            fetch_response_tx,
            stream_manager: stream_manager.clone(),
        };

        (runtime, scheduler_rx, callback_tx, stream_manager)
    }

    /// Clear a timer (remove from callbacks and intervals)
    pub fn clear_timer(&mut self, callback_id: CallbackId) {
        let mut cbs = self.callbacks.lock().unwrap();
        cbs.remove(&callback_id);

        let mut intervals = self.intervals.lock().unwrap();
        intervals.remove(&callback_id);

        // Send clear message to event loop
        let _ = self
            .scheduler_tx
            .send(SchedulerMessage::ClearTimer(callback_id));
    }

    /// Process pending callbacks (non-blocking)
    pub fn process_callbacks(&mut self) {
        while let Ok(msg) = self.callback_rx.try_recv() {
            match msg {
                CallbackMessage::ExecuteTimeout(callback_id) => {
                    // Timeouts are one-shot: remove the callback after execution
                    let callback_opt = {
                        let mut cbs = self.callbacks.lock().unwrap();
                        cbs.remove(&callback_id)
                    };

                    if let Some(callback) = callback_opt {
                        log::debug!("Executing timeout callback {}", callback_id);

                        // Call the callback
                        match callback.call_as_function(&self.context, None, &[]) {
                            Ok(_) => log::debug!("Callback {} executed successfully", callback_id),
                            Err(e) => {
                                if let Ok(err_str) = e.to_js_string(&self.context) {
                                    log::error!("Callback {} failed: {}", callback_id, err_str);
                                } else {
                                    log::error!(
                                        "Callback {} failed with unknown error",
                                        callback_id
                                    );
                                }
                            }
                        }
                    }
                }
                CallbackMessage::ExecutePromiseResolve(callback_id, result_str) => {
                    // Execute resolve callback with result
                    let callback_opt = {
                        let mut cbs = self.callbacks.lock().unwrap();
                        cbs.remove(&callback_id)
                    };

                    if let Some(callback) = callback_opt {
                        log::debug!("Executing promise resolve callback {}", callback_id);

                        let result_val = JSValue::string(&self.context, result_str.as_str());
                        match callback.call_as_function(&self.context, None, &[result_val]) {
                            Ok(_) => log::debug!("Promise resolved successfully"),
                            Err(e) => {
                                if let Ok(err_str) = e.to_js_string(&self.context) {
                                    log::error!("Promise resolve failed: {}", err_str);
                                }
                            }
                        }
                    }
                }
                CallbackMessage::ExecutePromiseReject(callback_id, error_msg) => {
                    // Execute reject callback with error
                    let callback_opt = {
                        let mut cbs = self.callbacks.lock().unwrap();
                        cbs.remove(&callback_id)
                    };

                    if let Some(callback) = callback_opt {
                        log::debug!("Executing promise reject callback {}", callback_id);

                        let error_val = JSValue::string(&self.context, error_msg.as_str());
                        match callback.call_as_function(&self.context, None, &[error_val]) {
                            Ok(_) => log::debug!("Promise rejected successfully"),
                            Err(e) => {
                                if let Ok(err_str) = e.to_js_string(&self.context) {
                                    log::error!("Promise reject failed: {}", err_str);
                                }
                            }
                        }
                    }
                }
                CallbackMessage::FetchError(callback_id, error_msg) => {
                    // Execute fetch reject callback
                    let callback_opt = {
                        let mut cbs = self.callbacks.lock().unwrap();
                        cbs.remove(&callback_id)
                    };

                    if let Some(callback) = callback_opt {
                        log::debug!("Rejecting fetch promise {}: {}", callback_id, error_msg);

                        let error_val = JSValue::string(&self.context, error_msg.as_str());
                        match callback.call_as_function(&self.context, None, &[error_val]) {
                            Ok(_) => log::debug!("Fetch promise rejected successfully"),
                            Err(e) => {
                                if let Ok(err_str) = e.to_js_string(&self.context) {
                                    log::error!("Fetch reject callback failed: {}", err_str);
                                }
                            }
                        }
                    }
                }
                CallbackMessage::ExecuteInterval(callback_id) => {
                    // Intervals keep the callback for repeated execution
                    let callback_opt = {
                        let cbs = self.callbacks.lock().unwrap();
                        cbs.get(&callback_id).cloned()
                    };

                    if let Some(callback) = callback_opt {
                        // Check if interval is still active
                        let is_active = {
                            let intervals = self.intervals.lock().unwrap();
                            intervals.contains(&callback_id)
                        };

                        if !is_active {
                            log::debug!("Interval {} was cleared, skipping execution", callback_id);
                            continue;
                        }

                        log::debug!("Executing interval callback {}", callback_id);

                        // Call the callback
                        match callback.call_as_function(&self.context, None, &[]) {
                            Ok(_) => log::debug!("Interval {} executed successfully", callback_id),
                            Err(e) => {
                                if let Ok(err_str) = e.to_js_string(&self.context) {
                                    log::error!("Interval {} failed: {}", callback_id, err_str);
                                } else {
                                    log::error!(
                                        "Interval {} failed with unknown error",
                                        callback_id
                                    );
                                }
                            }
                        }
                    }
                }
                CallbackMessage::FetchStreamingSuccess(callback_id, meta, stream_id) => {
                    // Execute fetch resolve callback with a full Response object
                    let callback_opt = {
                        let mut cbs = self.callbacks.lock().unwrap();
                        cbs.remove(&callback_id)
                    };

                    if let Some(callback) = callback_opt {
                        log::debug!(
                            "Resolving fetch streaming promise {} with stream {}",
                            callback_id,
                            stream_id
                        );

                        // Create a Response with streaming body using __createNativeStream
                        let headers_json =
                            serde_json::to_string(&meta.headers).unwrap_or("{}".to_string());
                        let response_script = format!(
                            r#"(function() {{
                                const stream = __createNativeStream({});
                                const response = new Response(stream, {{
                                    status: {},
                                    statusText: "{}",
                                    headers: {}
                                }});
                                // Mark as streaming response
                                response._isStreaming = true;
                                return response;
                            }})()"#,
                            stream_id, meta.status, meta.status_text, headers_json
                        );

                        match self.context.evaluate_script(&response_script, 1) {
                            Ok(response_obj) => {
                                match callback.call_as_function(
                                    &self.context,
                                    None,
                                    &[response_obj],
                                ) {
                                    Ok(_) => log::debug!("Fetch streaming resolved successfully"),
                                    Err(e) => {
                                        if let Ok(err_str) = e.to_js_string(&self.context) {
                                            log::error!(
                                                "Fetch streaming callback failed: {}",
                                                err_str
                                            );
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                if let Ok(err_str) = e.to_js_string(&self.context) {
                                    log::error!("Failed to create streaming Response: {}", err_str);
                                }
                            }
                        }
                    }
                }
                CallbackMessage::StreamChunk(callback_id, chunk) => {
                    // Execute stream read callback with chunk result
                    let callback_opt = {
                        let mut cbs = self.callbacks.lock().unwrap();
                        cbs.remove(&callback_id)
                    };

                    if let Some(callback) = callback_opt {
                        log::debug!("Executing stream chunk callback {}", callback_id);

                        // Create result object based on chunk type
                        let result_script = match chunk {
                            stream_manager::StreamChunk::Data(bytes) => {
                                // Convert bytes to Uint8Array
                                let bytes_array: Vec<u8> = bytes.to_vec();
                                let bytes_str = format!("{:?}", bytes_array);
                                format!(
                                    r#"({{
                                        done: false,
                                        value: new Uint8Array({})
                                    }})"#,
                                    bytes_str
                                )
                            }
                            stream_manager::StreamChunk::Done => {
                                r#"({ done: true, value: undefined })"#.to_string()
                            }
                            stream_manager::StreamChunk::Error(err) => {
                                format!(r#"({{ error: "{}" }})"#, err.replace('"', "\\\""))
                            }
                        };

                        match self.context.evaluate_script(&result_script, 1) {
                            Ok(result_obj) => {
                                match callback.call_as_function(&self.context, None, &[result_obj])
                                {
                                    Ok(_) => log::debug!("Stream chunk callback executed"),
                                    Err(e) => {
                                        if let Ok(err_str) = e.to_js_string(&self.context) {
                                            log::error!(
                                                "Stream chunk callback failed: {}",
                                                err_str
                                            );
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                if let Ok(err_str) = e.to_js_string(&self.context) {
                                    log::error!("Failed to create stream result: {}", err_str);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Evaluate a JavaScript script
    pub fn evaluate(&mut self, script: &str) -> Result<JSValue, JSValue> {
        self.context.evaluate_script(script, 1)
    }
}

/// Background event loop that handles scheduled tasks
pub async fn run_event_loop(
    mut scheduler_rx: mpsc::UnboundedReceiver<SchedulerMessage>,
    callback_tx: mpsc::UnboundedSender<CallbackMessage>,
    stream_manager: Arc<stream_manager::StreamManager>,
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
                tokio::spawn(async move {
                    match fetch::execute_fetch_streaming(request, manager).await {
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

impl Drop for Runtime {
    fn drop(&mut self) {
        // Send shutdown message
        let _ = self.scheduler_tx.send(SchedulerMessage::Shutdown);
    }
}
