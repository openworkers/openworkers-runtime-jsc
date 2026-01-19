use crate::runtime::{Runtime, run_event_loop, stream_manager::StreamChunk};
use openworkers_core::{
    Event, HttpResponse, RequestBody, ResponseBody, RuntimeLimits, Script, TaskInit, TaskResult,
    TaskSource, TerminationReason,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Worker that executes JavaScript with event handlers
pub struct Worker {
    pub(crate) runtime: Runtime,
    event_loop_handle: tokio::task::JoinHandle<()>,
    aborted: Arc<AtomicBool>,
}

impl Worker {
    /// Evaluate a script for testing/inspection
    pub fn evaluate(&mut self, script: &str) -> Result<rusty_jsc::JSValue, String> {
        self.runtime.evaluate(script).map_err(|e| {
            if let Ok(err_str) = e.to_js_string(&self.runtime.context) {
                err_str.to_string()
            } else {
                "Evaluation failed".to_string()
            }
        })
    }

    /// Get context reference for testing
    pub fn context(&self) -> &rusty_jsc::JSContext {
        &self.runtime.context
    }
}

impl Worker {
    /// Create a new worker with full options (openworkers-core compatible)
    pub async fn new(
        script: Script,
        _limits: Option<RuntimeLimits>,
    ) -> Result<Self, TerminationReason> {
        let (mut runtime, scheduler_rx, callback_tx, stream_manager) = Runtime::new();

        // Setup addEventListener binding
        setup_event_listener(&mut runtime.context, runtime.fetch_response_tx.clone());

        // Setup environment variables
        setup_env(&mut runtime.context, &script.env);

        // Setup console
        crate::runtime::bindings::setup_console(&mut runtime.context);

        // TODO: Apply runtime limits

        // Extract JavaScript code from WorkerCode
        let js_code = script.code.as_js().ok_or_else(|| {
            TerminationReason::Exception("Only JavaScript code is supported".to_string())
        })?;

        // Load and evaluate the worker script
        runtime.evaluate(js_code).map_err(|e| {
            if let Ok(err_str) = e.to_js_string(&runtime.context) {
                TerminationReason::Exception(format!("Script evaluation failed: {}", err_str))
            } else {
                TerminationReason::Exception("Script evaluation failed".to_string())
            }
        })?;

        // Start event loop in background
        let event_loop_handle = tokio::spawn(async move {
            run_event_loop(scheduler_rx, callback_tx, stream_manager).await;
        });

        Ok(Self {
            runtime,
            event_loop_handle,
            aborted: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Abort the worker execution
    pub fn abort(&mut self) {
        self.aborted.store(true, Ordering::SeqCst);
        self.event_loop_handle.abort();
    }

    /// Execute an event and return termination reason (openworkers-core compatible)
    pub async fn exec(&mut self, mut event: Event) -> Result<(), TerminationReason> {
        // Check if aborted before starting
        if self.aborted.load(Ordering::SeqCst) {
            return Err(TerminationReason::Aborted);
        }

        match event {
            Event::Fetch(ref mut init) => {
                let fetch_init = init.take().ok_or(TerminationReason::Other(
                    "FetchInit already consumed".to_string(),
                ))?;
                self.trigger_fetch_event(fetch_init).await?;
                Ok(())
            }
            Event::Task(ref mut init) => {
                let task_init = init.take().ok_or(TerminationReason::Other(
                    "TaskInit already consumed".to_string(),
                ))?;
                self.trigger_task_event(task_init).await?;
                Ok(())
            }
        }
    }

    /// Execute an event and return the HTTP response directly
    pub async fn exec_http(&mut self, mut event: Event) -> Result<HttpResponse, TerminationReason> {
        match event {
            Event::Fetch(ref mut init) => {
                let fetch_init = init.take().ok_or(TerminationReason::Other(
                    "FetchInit already consumed".to_string(),
                ))?;
                self.trigger_fetch_event(fetch_init).await
            }
            Event::Task(ref mut init) => {
                let task_init = init.take().ok_or(TerminationReason::Other(
                    "TaskInit already consumed".to_string(),
                ))?;
                self.trigger_task_event(task_init).await?;

                // Return empty response for task events
                Ok(HttpResponse {
                    status: 200,
                    headers: vec![],
                    body: ResponseBody::None,
                })
            }
        }
    }

    async fn trigger_fetch_event(
        &mut self,
        fetch_init: openworkers_core::FetchInit,
    ) -> Result<HttpResponse, TerminationReason> {
        let req = &fetch_init.req;

        // Build headers object for JS
        let headers_json = serde_json::to_string(&req.headers).unwrap_or("{}".to_string());

        // Create Request object
        let body_str = match &req.body {
            RequestBody::Bytes(bytes) => String::from_utf8(bytes.to_vec()).unwrap_or_default(),
            RequestBody::Stream(_) => String::new(), // Stream body not supported for now
            RequestBody::None => String::new(),
        };

        let request_script = format!(
            r#"({{
                method: "{}",
                url: "{}",
                headers: new Headers({}),
                text: () => Promise.resolve("{}"),
                json: () => Promise.resolve(JSON.parse("{}")),
            }})"#,
            req.method,
            req.url,
            headers_json,
            body_str.replace('"', "\\\""),
            body_str.replace('"', "\\\""),
        );

        let request_obj = self
            .runtime
            .context
            .evaluate_script(&request_script, 1)
            .map_err(|_| {
                TerminationReason::Exception("Failed to create Request object".to_string())
            })?;

        // Call the fetch event trigger (set by addEventListener)
        // The Response is stored in __lastResponse by the event handler
        let trigger_script = r#"
            (function(request) {
                if (typeof globalThis.__triggerFetch === 'function') {
                    globalThis.__triggerFetch(request);
                } else {
                    throw new Error("No fetch handler registered");
                }
            })
        "#;

        let trigger_fn = self
            .runtime
            .context
            .evaluate_script(trigger_script, 1)
            .map_err(|_| {
                TerminationReason::Exception("Failed to get trigger function".to_string())
            })?
            .to_object(&self.runtime.context)
            .map_err(|_| TerminationReason::Exception("Trigger is not a function".to_string()))?;

        let trigger_result =
            trigger_fn.call_as_function(&self.runtime.context, None, &[request_obj]);

        if let Err(e) = trigger_result {
            let error_msg = if let Ok(err_str) = e.to_js_string(&self.runtime.context) {
                let full_error = err_str.to_string();
                log::error!("Fetch handler exception: {}", full_error);

                // Try to get stack trace if available
                if let Ok(err_obj) = e.to_object(&self.runtime.context) {
                    if let Some(stack_val) = err_obj.get_property(&self.runtime.context, "stack") {
                        if let Ok(stack_str) = stack_val.to_js_string(&self.runtime.context) {
                            log::error!("Stack trace:\n{}", stack_str);
                        }
                    }
                }

                format!("Fetch handler exception: {}", full_error)
            } else {
                "Fetch handler error (unknown)".to_string()
            };
            return Err(TerminationReason::Exception(error_msg));
        }

        // Wait for __lastResponse to be set with adaptive polling
        // Fast polling for sync responses, timeout after ~5s for async handlers
        for iteration in 0..500 {
            self.runtime.process_callbacks();

            // Check if __lastResponse is set
            let check_script = r#"
                (function() {
                    const resp = globalThis.__lastResponse;
                    if (resp && typeof resp === 'object' && resp.status !== undefined) {
                        return true;
                    }
                    return false;
                })()
            "#;

            if let Ok(result) = self.runtime.context.evaluate_script(check_script, 1) {
                if result.to_bool(&self.runtime.context) {
                    break;
                }
            }

            // Adaptive sleep: fast for first checks, slower later
            let sleep_duration = if iteration < 10 {
                tokio::time::Duration::from_micros(1)
            } else if iteration < 110 {
                tokio::time::Duration::from_millis(1)
            } else {
                tokio::time::Duration::from_millis(10)
            };

            tokio::time::sleep(sleep_duration).await;

            if iteration == 499 {
                return Err(TerminationReason::WallClockTimeout);
            }
        }

        // Extract response metadata from __lastResponse
        // All responses with body are now streamed via _responseStreamId
        let extract_script = r#"
            (function() {
                const resp = globalThis.__lastResponse;
                if (!resp) {
                    return JSON.stringify({ error: 'No response' });
                }

                // Extract headers
                const headers = [];
                if (resp.headers) {
                    if (resp.headers instanceof Headers) {
                        for (const [key, value] of resp.headers) {
                            headers.push([key, String(value)]);
                        }
                    } else if (typeof resp.headers === 'object') {
                        for (const [key, value] of Object.entries(resp.headers)) {
                            headers.push([key, String(value)]);
                        }
                    }
                }

                // Check for response stream ID (all responses with body have this now)
                const responseStreamId = resp._responseStreamId;

                return JSON.stringify({
                    status: resp.status || 200,
                    headers: headers,
                    responseStreamId: responseStreamId !== undefined ? responseStreamId : null,
                    hasBody: resp.body !== null
                });
            })()
        "#;

        let extract_result = self
            .runtime
            .context
            .evaluate_script(extract_script, 1)
            .map_err(|_| {
                TerminationReason::Exception("Failed to extract response data".to_string())
            })?;

        let json_str = extract_result
            .to_js_string(&self.runtime.context)
            .map_err(|_| TerminationReason::Exception("Failed to get response JSON".to_string()))?
            .to_string();

        // Parse the extracted metadata
        #[derive(serde::Deserialize)]
        struct ExtractedResponse {
            status: u16,
            headers: Vec<(String, String)>,
            #[serde(rename = "responseStreamId")]
            response_stream_id: Option<u64>,
            #[serde(rename = "hasBody")]
            has_body: bool,
        }

        let extracted: ExtractedResponse = serde_json::from_str(&json_str).map_err(|e| {
            TerminationReason::Exception(format!("Failed to parse extracted response: {}", e))
        })?;

        // All responses with body are now streamed
        let body = if let Some(stream_id) = extracted.response_stream_id {
            // Take the receiver from stream manager
            if let Some(rx) = self.runtime.stream_manager.take_receiver(stream_id) {
                // Create bounded channel for HttpBody
                const RESPONSE_STREAM_BUFFER_SIZE: usize = 16;
                let (tx, response_rx) = tokio::sync::mpsc::channel(RESPONSE_STREAM_BUFFER_SIZE);

                // Spawn task to forward from StreamChunk to Result<Bytes, String>
                tokio::spawn(async move {
                    let mut rx = rx;
                    while let Some(chunk) = rx.recv().await {
                        match chunk {
                            StreamChunk::Data(bytes) => {
                                if tx.send(Ok(bytes)).await.is_err() {
                                    break;
                                }
                            }
                            StreamChunk::Done => {
                                break;
                            }
                            StreamChunk::Error(e) => {
                                let _ = tx.send(Err(e)).await;
                                break;
                            }
                        }
                    }
                });

                ResponseBody::Stream(response_rx)
            } else {
                // Stream not found, return empty
                ResponseBody::None
            }
        } else if extracted.has_body {
            // Has body but no stream ID - shouldn't happen, but handle it
            ResponseBody::None
        } else {
            // No body
            ResponseBody::None
        };

        // Send response back via channel
        let _ = fetch_init.res_tx.send(HttpResponse {
            status: extracted.status,
            headers: extracted.headers.clone(),
            body,
        });

        // Return response for exec_http (body already sent via channel)
        Ok(HttpResponse {
            status: extracted.status,
            headers: extracted.headers,
            body: ResponseBody::None,
        })
    }

    async fn trigger_task_event(&mut self, task_init: TaskInit) -> Result<(), TerminationReason> {
        // Extract scheduled time if this is a schedule-triggered task
        let scheduled_time = match &task_init.source {
            Some(TaskSource::Schedule { time }) => Some(*time),
            _ => None,
        };

        // Build event object with task info
        let payload_json = task_init
            .payload
            .as_ref()
            .map(|p| serde_json::to_string(p).unwrap_or_default())
            .unwrap_or_else(|| "null".to_string());

        let scheduled_time_js = scheduled_time
            .map(|t| t.to_string())
            .unwrap_or_else(|| "undefined".to_string());

        let event_script = format!(
            r#"({{
                taskId: "{}",
                attempt: {},
                payload: {},
                scheduledTime: {}
            }})"#,
            task_init.task_id.replace('"', "\\\""),
            task_init.attempt,
            payload_json,
            scheduled_time_js
        );

        let event_obj = self
            .runtime
            .context
            .evaluate_script(&event_script, 1)
            .map_err(|_| TerminationReason::Exception("Failed to create event".to_string()))?;

        // Try __taskHandler first, then fallback to __triggerScheduled for backward compat
        let trigger_script = r#"
            (function(event) {
                if (typeof globalThis.__taskHandler === 'function') {
                    return globalThis.__taskHandler(event);
                } else if (typeof globalThis.__triggerScheduled === 'function') {
                    return globalThis.__triggerScheduled(event);
                }
                throw new Error("No task handler registered");
            })
        "#;

        let trigger_fn = self
            .runtime
            .context
            .evaluate_script(trigger_script, 1)
            .map_err(|_| TerminationReason::Exception("Failed to get trigger".to_string()))?
            .to_object(&self.runtime.context)
            .map_err(|_| TerminationReason::Exception("Trigger not a function".to_string()))?;

        if let Err(e) = trigger_fn.call_as_function(&self.runtime.context, None, &[event_obj]) {
            let error_msg = if let Ok(err_str) = e.to_js_string(&self.runtime.context) {
                let full_error = err_str.to_string();
                log::error!("Task handler exception: {}", full_error);

                // Try to get stack trace
                if let Ok(err_obj) = e.to_object(&self.runtime.context) {
                    if let Some(stack_val) = err_obj.get_property(&self.runtime.context, "stack") {
                        if let Ok(stack_str) = stack_val.to_js_string(&self.runtime.context) {
                            log::error!("Stack trace:\n{}", stack_str);
                        }
                    }
                }

                format!("Task handler exception: {}", full_error)
            } else {
                "Task handler error (unknown)".to_string()
            };
            return Err(TerminationReason::Exception(error_msg));
        }

        // Process callbacks with adaptive polling and check for __taskResult
        for iteration in 0..500 {
            self.runtime.process_callbacks();

            // Check if __requestComplete is set (handler finished including waitUntil)
            let check_script = r#"
                (function() {
                    return globalThis.__requestComplete === true;
                })()
            "#;

            if let Ok(result) = self.runtime.context.evaluate_script(check_script, 1) {
                if result.to_bool(&self.runtime.context) {
                    break;
                }
            }

            // Adaptive sleep
            let sleep_duration = if iteration < 10 {
                tokio::time::Duration::from_micros(1)
            } else if iteration < 110 {
                tokio::time::Duration::from_millis(1)
            } else {
                tokio::time::Duration::from_millis(10)
            };

            tokio::time::sleep(sleep_duration).await;

            if iteration == 499 {
                return Err(TerminationReason::WallClockTimeout);
            }
        }

        // Extract __taskResult from JS
        let extract_script = r#"
            (function() {
                const result = globalThis.__taskResult;
                if (!result || typeof result !== 'object') {
                    return JSON.stringify({ success: true });
                }
                return JSON.stringify({
                    success: result.success !== false,
                    data: result.data,
                    error: result.error
                });
            })()
        "#;

        let task_result =
            if let Ok(result_val) = self.runtime.context.evaluate_script(extract_script, 1) {
                if let Ok(result_str) = result_val.to_js_string(&self.runtime.context) {
                    let json_str = result_str.to_string();

                    #[derive(serde::Deserialize)]
                    struct ExtractedResult {
                        success: bool,
                        data: Option<serde_json::Value>,
                        error: Option<String>,
                    }

                    if let Ok(extracted) = serde_json::from_str::<ExtractedResult>(&json_str) {
                        TaskResult {
                            success: extracted.success,
                            data: extracted.data,
                            error: extracted.error,
                        }
                    } else {
                        TaskResult::success()
                    }
                } else {
                    TaskResult::success()
                }
            } else {
                TaskResult::success()
            };

        // Send result
        let _ = task_init.res_tx.send(task_result);

        Ok(())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Abort event loop
        self.event_loop_handle.abort();
    }
}

/// Setup addEventListener binding
fn setup_event_listener(
    context: &mut rusty_jsc::JSContext,
    fetch_response_tx: std::sync::Arc<
        std::sync::Mutex<Option<tokio::sync::oneshot::Sender<String>>>,
    >,
) {
    // Setup native __sendFetchResponse function
    let fetch_tx_clone = fetch_response_tx.clone();
    let send_response_callback = rusty_jsc::callback_closure!(
        context,
        move |ctx: rusty_jsc::JSContext,
              _function: rusty_jsc::JSObject,
              _this: rusty_jsc::JSObject,
              args: &[rusty_jsc::JSValue]| {
            if args.is_empty() {
                return Ok(rusty_jsc::JSValue::undefined(&ctx));
            }

            if let Ok(response_json) = args[0].to_js_string(&ctx) {
                let response_str = response_json.to_string();

                // Send the response through the channel
                if let Some(tx) = fetch_tx_clone.lock().unwrap().take() {
                    let _ = tx.send(response_str);
                }
            }

            Ok(rusty_jsc::JSValue::undefined(&ctx))
        }
    );

    context
        .get_global_object()
        .set_property(
            context,
            "__sendFetchResponse",
            send_response_callback.into(),
        )
        .unwrap();

    let add_event_listener_script = r#"
        // Stream all response bodies to Rust
        globalThis.__streamResponseBody = async function(response) {
            if (!response || !response.body) {
                // No body to stream
                return response;
            }

            // If it already has a native stream ID (fetch forward), use that
            if (response.body._nativeStreamId !== undefined) {
                response._responseStreamId = response.body._nativeStreamId;
                return response;
            }

            // Create output stream
            const streamId = __responseStreamCreate();
            response._responseStreamId = streamId;

            // Start streaming asynchronously
            (async () => {
                try {
                    const reader = response.body.getReader();
                    while (true) {
                        const { done, value } = await reader.read();
                        if (done) {
                            __responseStreamEnd(streamId);
                            break;
                        }
                        if (value) {
                            __responseStreamWrite(streamId, value);
                        }
                    }
                } catch (e) {
                    console.error('[__streamResponseBody] Error:', e);
                    __responseStreamEnd(streamId);
                }
            })();

            return response;
        };

        globalThis.addEventListener = function(type, handler) {
            if (type === 'fetch') {
                globalThis.__fetchHandler = handler;
                globalThis.__triggerFetch = function(request) {
                    // Reset last response
                    globalThis.__lastResponse = null;

                    const event = {
                        request: request,
                        respondWith: function(responseOrPromise) {
                            // Handle both direct Response and Promise<Response>
                            if (responseOrPromise && typeof responseOrPromise.then === 'function') {
                                // It's a Promise, wait for it to resolve then stream
                                responseOrPromise
                                    .then(response => __streamResponseBody(response))
                                    .then(response => {
                                        globalThis.__lastResponse = response;
                                    })
                                    .catch(error => {
                                        console.error('[respondWith] Promise rejected:', error);
                                        globalThis.__lastResponse = new Response(null, { status: 500 });
                                    });
                            } else {
                                // Direct Response object - stream it
                                __streamResponseBody(responseOrPromise)
                                    .then(response => {
                                        globalThis.__lastResponse = response;
                                    });
                            }
                        }
                    };

                    // Call handler synchronously
                    try {
                        handler(event);
                    } catch (error) {
                        console.error('[addEventListener] Error in fetch handler:', error);
                        globalThis.__lastResponse = new Response(null, { status: 500 });
                    }
                };
            } else if (type === 'scheduled') {
                globalThis.__triggerScheduled = async function(event) {
                    globalThis.__requestComplete = false;
                    const promises = [];

                    event.waitUntil = function(promise) {
                        promises.push(Promise.resolve(promise));
                    };

                    try {
                        // Call handler
                        await handler(event);

                        // Wait for all promises
                        if (promises.length > 0) {
                            await Promise.all(promises);
                        }
                    } finally {
                        globalThis.__requestComplete = true;
                    }
                };
            } else if (type === 'task') {
                globalThis.__taskHandler = async function(event) {
                    globalThis.__requestComplete = false;
                    const waitUntilPromises = [];

                    // Default result (success with no data)
                    globalThis.__taskResult = { success: true };

                    event.waitUntil = function(promise) {
                        waitUntilPromises.push(Promise.resolve(promise));
                    };

                    event.respondWith = function(result) {
                        if (result && typeof result === 'object') {
                            globalThis.__taskResult = {
                                success: result.success !== false,
                                data: result.data,
                                error: result.error
                            };
                        } else {
                            globalThis.__taskResult = { success: true, data: result };
                        }
                    };

                    try {
                        const result = await handler(event);

                        // If handler returns a value and respondWith wasn't called, use it
                        if (result !== undefined && globalThis.__taskResult.data === undefined) {
                            if (result && typeof result === 'object' && 'success' in result) {
                                globalThis.__taskResult = {
                                    success: result.success !== false,
                                    data: result.data,
                                    error: result.error
                                };
                            } else {
                                globalThis.__taskResult = { success: true, data: result };
                            }
                        }

                        // Wait for all waitUntil promises to complete
                        if (waitUntilPromises.length > 0) {
                            await Promise.all(waitUntilPromises);
                        }
                    } catch (error) {
                        globalThis.__taskResult = {
                            success: false,
                            error: error.message || String(error)
                        };
                    } finally {
                        globalThis.__requestComplete = true;
                    }
                };
            }
        };
    "#;

    context
        .evaluate_script(add_event_listener_script, 1)
        .unwrap();
}

/// Setup environment variables as globalThis.env
fn setup_env(
    context: &mut rusty_jsc::JSContext,
    env: &Option<std::collections::HashMap<String, String>>,
) {
    let env_json = if let Some(env_map) = env {
        let pairs: Vec<String> = env_map
            .iter()
            .map(|(k, v)| {
                format!(
                    "\"{}\": \"{}\"",
                    k.replace('\\', "\\\\").replace('"', "\\\""),
                    v.replace('\\', "\\\\").replace('"', "\\\"")
                )
            })
            .collect();
        format!("{{{}}}", pairs.join(", "))
    } else {
        "{}".to_string()
    };

    let script = format!(
        r#"Object.defineProperty(globalThis, 'env', {{
            value: {},
            writable: false,
            enumerable: true,
            configurable: false
        }});"#,
        env_json
    );

    context.evaluate_script(&script, 1).unwrap();
}

impl openworkers_core::Worker for Worker {
    async fn new(script: Script, limits: Option<RuntimeLimits>) -> Result<Self, TerminationReason> {
        Worker::new(script, limits).await
    }

    async fn exec(&mut self, event: Event) -> Result<(), TerminationReason> {
        Worker::exec(self, event).await
    }

    fn abort(&mut self) {
        Worker::abort(self)
    }
}
