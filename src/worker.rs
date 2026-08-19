use crate::runtime::{Runtime, run_event_loop, stream_manager::StreamChunk};
use openworkers_core::{
    Event, HttpResponse, OperationsHandle, RequestBody, ResponseBody, RuntimeLimits, Script,
    TaskInit, TaskResult, TaskSource, TerminationReason,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Result slot of the running event, keyed by an epoch the handler echoes back,
/// or else the late result of a timed out event answers the next request
#[derive(Default)]
struct Completion {
    epoch: u64,
    tx: Option<tokio::sync::oneshot::Sender<String>>,
}

impl Completion {
    fn arm(&mut self) -> (u64, tokio::sync::oneshot::Receiver<String>) {
        let (tx, rx) = tokio::sync::oneshot::channel();

        self.epoch += 1;
        self.tx = Some(tx);

        (self.epoch, rx)
    }

    fn complete(&mut self, epoch: u64, result: String) {
        if epoch != self.epoch {
            log::warn!("dropping completion of event {}, event is gone", epoch);
            return;
        }

        if let Some(tx) = self.tx.take() {
            let _ = tx.send(result);
        }
    }
}

/// Worker that executes JavaScript with event handlers
pub struct Worker {
    pub(crate) runtime: Runtime,
    completion: Rc<RefCell<Completion>>,
    event_loop_handle: tokio::task::JoinHandle<()>,
    aborted: Arc<AtomicBool>,
    limits: RuntimeLimits,
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
    /// Create a new worker with an OperationsHandler
    ///
    /// All operations (fetch, log, etc.) go through the runner's OperationsHandler.
    pub async fn new_with_ops(
        script: Script,
        limits: Option<RuntimeLimits>,
        ops: OperationsHandle,
    ) -> Result<Self, TerminationReason> {
        let (mut runtime, scheduler_rx, callback_tx, stream_manager) = Runtime::new();
        let completion = Rc::new(RefCell::new(Completion::default()));

        // Setup addEventListener binding
        setup_event_listener(&mut runtime.context, completion.clone());

        // Setup environment variables
        setup_env(&mut runtime.context, &script.env);

        // Setup console
        crate::runtime::bindings::setup_console(&mut runtime.context, Some(ops.clone()));

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
            run_event_loop(scheduler_rx, callback_tx, stream_manager, ops).await;
        });

        Ok(Self {
            runtime,
            completion,
            event_loop_handle,
            aborted: Arc::new(AtomicBool::new(false)),
            limits: limits.unwrap_or_default(),
        })
    }

    /// Create a new worker with default operations (standalone mode)
    ///
    /// Uses DefaultOps which returns errors for all operations.
    /// For production, use `new_with_ops` with a proper OperationsHandler.
    pub async fn new(
        script: Script,
        limits: Option<RuntimeLimits>,
    ) -> Result<Self, TerminationReason> {
        let ops: OperationsHandle = Arc::new(openworkers_core::DefaultOps);
        Self::new_with_ops(script, limits, ops).await
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
        let request_obj = self.make_request_object(&fetch_init.req).map_err(|_| {
            TerminationReason::Exception("Failed to create Request object".to_string())
        })?;

        // The handler reports the response metadata via the native __completeEvent
        let (epoch, done_rx) = self.completion.borrow_mut().arm();

        let trigger_script = r#"
            (function(request, epoch) {
                if (typeof globalThis.__triggerFetch === 'function') {
                    globalThis.__triggerFetch(request, epoch);
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

        let epoch = rusty_jsc::JSValue::number(&self.runtime.context, epoch as f64);
        let trigger_result =
            trigger_fn.call_as_function(&self.runtime.context, None, &[request_obj, epoch]);

        if let Err(e) = trigger_result {
            let error_msg = if let Ok(err_str) = e.to_js_string(&self.runtime.context) {
                let full_error = err_str.to_string();
                log::error!("Fetch handler exception: {}", full_error);

                // Try to get stack trace if available
                if let Ok(err_obj) = e.to_object(&self.runtime.context)
                    && let Some(stack_val) = err_obj.get_property(&self.runtime.context, "stack")
                    && let Ok(stack_str) = stack_val.to_js_string(&self.runtime.context)
                {
                    log::error!("Stack trace:\n{}", stack_str);
                }

                format!("Fetch handler exception: {}", full_error)
            } else {
                "Fetch handler error (unknown)".to_string()
            };
            return Err(TerminationReason::Exception(error_msg));
        }

        let json_str = self.wait_for_completion(done_rx).await?;

        // Parse the extracted metadata
        #[derive(serde::Deserialize)]
        struct ExtractedResponse {
            status: u16,
            headers: Vec<(String, String)>,
            #[serde(rename = "responseStreamId")]
            response_stream_id: Option<u64>,
        }

        let extracted: ExtractedResponse = serde_json::from_str(&json_str).map_err(|e| {
            TerminationReason::Exception(format!("Failed to parse extracted response: {}", e))
        })?;

        // Every response body is streamed, so no stream means no body
        let stream = extracted
            .response_stream_id
            .and_then(|stream_id| self.runtime.stream_manager.take_receiver(stream_id));

        let body = match stream {
            None => ResponseBody::None,
            Some(mut rx) => {
                // Create bounded channel for HttpBody
                const RESPONSE_STREAM_BUFFER_SIZE: usize = 16;
                let (tx, response_rx) = tokio::sync::mpsc::channel(RESPONSE_STREAM_BUFFER_SIZE);

                // Spawn task to forward from StreamChunk to Result<Bytes, String>
                tokio::spawn(async move {
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
            }
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

        // Splice only JSON, or else a quote or a backslash in the task id breaks out
        let task_id_json = serde_json::to_string(&task_init.task_id).expect("task id is a string");

        let event_script = format!(
            r#"({{
                taskId: {},
                attempt: {},
                payload: {},
                scheduledTime: {}
            }})"#,
            task_id_json, task_init.attempt, payload_json, scheduled_time_js
        );

        let event_obj = self
            .runtime
            .context
            .evaluate_script(&event_script, 1)
            .map_err(|_| TerminationReason::Exception("Failed to create event".to_string()))?;

        // The handler reports the task result via the native __completeEvent
        let (epoch, done_rx) = self.completion.borrow_mut().arm();

        // Try __taskHandler first, then fallback to __triggerScheduled for backward compat
        let trigger_script = r#"
            (function(event, epoch) {
                if (typeof globalThis.__taskHandler === 'function') {
                    return globalThis.__taskHandler(event, epoch);
                } else if (typeof globalThis.__triggerScheduled === 'function') {
                    return globalThis.__triggerScheduled(event, epoch);
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

        let epoch = rusty_jsc::JSValue::number(&self.runtime.context, epoch as f64);

        if let Err(e) =
            trigger_fn.call_as_function(&self.runtime.context, None, &[event_obj, epoch])
        {
            let error_msg = if let Ok(err_str) = e.to_js_string(&self.runtime.context) {
                let full_error = err_str.to_string();
                log::error!("Task handler exception: {}", full_error);

                // Try to get stack trace
                if let Ok(err_obj) = e.to_object(&self.runtime.context)
                    && let Some(stack_val) = err_obj.get_property(&self.runtime.context, "stack")
                    && let Ok(stack_str) = stack_val.to_js_string(&self.runtime.context)
                {
                    log::error!("Stack trace:\n{}", stack_str);
                }

                format!("Task handler exception: {}", full_error)
            } else {
                "Task handler error (unknown)".to_string()
            };
            return Err(TerminationReason::Exception(error_msg));
        }

        let json_str = self.wait_for_completion(done_rx).await?;

        #[derive(serde::Deserialize)]
        struct ExtractedResult {
            success: bool,
            data: Option<serde_json::Value>,
            error: Option<String>,
        }

        let task_result = match serde_json::from_str::<ExtractedResult>(&json_str) {
            Ok(extracted) => TaskResult {
                success: extracted.success,
                data: extracted.data,
                error: extracted.error,
            },
            Err(_) => TaskResult::success(),
        };

        let _ = task_init.res_tx.send(task_result);

        Ok(())
    }

    fn make_request_object(
        &mut self,
        req: &openworkers_core::HttpRequest,
    ) -> Result<rusty_jsc::JSValue, rusty_jsc::JSValue> {
        let build_request_script = r#"
            (function(method, url, headers, body) {
                return new Request(url, {
                    method: method,
                    headers: JSON.parse(headers),
                    body: body.length > 0 ? body : null
                });
            })
        "#;

        let build_request = self
            .runtime
            .context
            .evaluate_script(build_request_script, 1)?
            .to_object(&self.runtime.context)?;

        let headers_json = serde_json::to_string(&req.headers).expect("headers are strings");

        let context = &self.runtime.context;
        let method = rusty_jsc::JSValue::string(context, req.method.as_str());
        let url = rusty_jsc::JSValue::string(context, req.url.as_str());
        let headers = rusty_jsc::JSValue::string(context, headers_json.as_str());

        let body: &[u8] = match &req.body {
            RequestBody::Bytes(bytes) => bytes,
            RequestBody::Stream(_) | RequestBody::None => &[],
        };

        let body = crate::runtime::new_uint8_array(&mut self.runtime.context, body)?;

        build_request.call_as_function(&self.runtime.context, None, &[method, url, headers, body])
    }

    /// Drive JS callbacks until the event completes, honoring the wall-clock limit
    async fn wait_for_completion(
        &mut self,
        mut done_rx: tokio::sync::oneshot::Receiver<String>,
    ) -> Result<String, TerminationReason> {
        let deadline = match self.limits.max_wall_clock_time_ms {
            0 => None,
            ms => Some(tokio::time::Instant::now() + tokio::time::Duration::from_millis(ms)),
        };

        loop {
            let timeout = async {
                match deadline {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            };

            tokio::select! {
                result = &mut done_rx => {
                    return result.map_err(|_| {
                        TerminationReason::Other("Completion channel closed".to_string())
                    });
                }
                msg = self.runtime.callback_rx.recv() => {
                    match msg {
                        Some(msg) => self.runtime.handle_callback(msg),
                        None => {
                            return Err(TerminationReason::Other(
                                "Event loop stopped".to_string(),
                            ));
                        }
                    }
                }
                _ = timeout => {
                    return Err(TerminationReason::WallClockTimeout);
                }
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Abort event loop
        self.event_loop_handle.abort();
    }
}

/// Setup addEventListener binding
fn setup_event_listener(context: &mut rusty_jsc::JSContext, completion: Rc<RefCell<Completion>>) {
    let complete_event_callback = rusty_jsc::callback_closure!(
        context,
        move |ctx: rusty_jsc::JSContext,
              _function: rusty_jsc::JSObject,
              _this: rusty_jsc::JSObject,
              args: &[rusty_jsc::JSValue]| {
            if args.len() < 2 {
                return Ok(rusty_jsc::JSValue::undefined(&ctx));
            }

            let epoch = args[0].to_number(&ctx).unwrap_or_default() as u64;

            if let Ok(result_json) = args[1].to_js_string(&ctx) {
                completion
                    .borrow_mut()
                    .complete(epoch, result_json.to_string());
            }

            Ok(rusty_jsc::JSValue::undefined(&ctx))
        }
    );

    context
        .get_global_object()
        .set_property(context, "__completeEvent", complete_event_callback.into())
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

        globalThis.__extractResponseMeta = function(resp) {
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

            const responseStreamId = resp._responseStreamId;

            return {
                status: resp.status || 200,
                headers: headers,
                responseStreamId: responseStreamId !== undefined ? responseStreamId : null
            };
        };

        globalThis.__finishFetch = function(response, epoch) {
            return __streamResponseBody(response).then(resp => {
                __completeEvent(epoch, JSON.stringify(__extractResponseMeta(resp)));
            });
        };

        globalThis.addEventListener = function(type, handler) {
            if (type === 'fetch') {
                globalThis.__fetchHandler = handler;
                globalThis.__triggerFetch = function(request, epoch) {
                    const event = {
                        request: request,
                        respondWith: function(responseOrPromise) {
                            Promise.resolve(responseOrPromise)
                                .then(response => __finishFetch(response, epoch))
                                .catch(error => {
                                    console.error('[respondWith] Promise rejected:', error);
                                    __finishFetch(new Response(null, { status: 500 }), epoch);
                                });
                        }
                    };

                    // Call handler synchronously
                    try {
                        handler(event);
                    } catch (error) {
                        console.error('[addEventListener] Error in fetch handler:', error);
                        __finishFetch(new Response(null, { status: 500 }), epoch);
                    }
                };
            } else if (type === 'scheduled') {
                globalThis.__triggerScheduled = async function(event, epoch) {
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
                        __completeEvent(epoch, JSON.stringify({ success: true }));
                    }
                };
            } else if (type === 'task') {
                globalThis.__taskHandler = async function(event, epoch) {
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
                        __completeEvent(epoch, JSON.stringify(globalThis.__taskResult));
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
    let env_json = match env {
        Some(env_map) => serde_json::to_string(env_map).expect("env is a map of strings"),
        None => "{}".to_string(),
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
