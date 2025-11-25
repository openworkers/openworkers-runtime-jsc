use crate::runtime::{Runtime, run_event_loop, stream_manager::StreamChunk};
use crate::task::{HttpResponse, ResponseBody, Task};
use bytes::Bytes;

/// Worker that executes JavaScript with event handlers
pub struct Worker {
    pub(crate) runtime: Runtime,
    event_loop_handle: tokio::task::JoinHandle<()>,
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
    /// Create a new worker with full options (openworkers-runtime compatible)
    pub async fn new(
        script: crate::compat::Script,
        _log_tx: Option<std::sync::mpsc::Sender<crate::compat::LogEvent>>,
        _limits: Option<crate::compat::RuntimeLimits>,
    ) -> Result<Self, String> {
        let (mut runtime, scheduler_rx, callback_tx, stream_manager) = Runtime::new();

        // Setup addEventListener binding
        setup_event_listener(&mut runtime.context, runtime.fetch_response_tx.clone());

        // TODO: Apply environment variables from script.env
        // TODO: Apply runtime limits
        // TODO: Wire up log_tx for console output

        // Load and evaluate the worker script
        runtime.evaluate(&script.code).map_err(|e| {
            if let Ok(err_str) = e.to_js_string(&runtime.context) {
                format!("Script evaluation failed: {}", err_str)
            } else {
                "Script evaluation failed".to_string()
            }
        })?;

        // Start event loop in background
        let event_loop_handle = tokio::spawn(async move {
            run_event_loop(scheduler_rx, callback_tx, stream_manager).await;
        });

        Ok(Self {
            runtime,
            event_loop_handle,
        })
    }

    /// Execute a task and return termination reason (openworkers-runtime compatible)
    pub async fn exec(
        &mut self,
        mut task: Task,
    ) -> Result<crate::compat::TerminationReason, String> {
        match task {
            Task::Fetch(ref mut init) => {
                let fetch_init = init.take().ok_or("FetchInit already consumed")?;

                // Trigger fetch event in JS
                match self.trigger_fetch_event(fetch_init).await {
                    Ok(_) => Ok(crate::compat::TerminationReason::Success),
                    Err(_) => Ok(crate::compat::TerminationReason::Exception),
                }
            }
            Task::Scheduled(ref mut init) => {
                let scheduled_init = init.take().ok_or("ScheduledInit already consumed")?;

                // Trigger scheduled event in JS
                match self.trigger_scheduled_event(scheduled_init).await {
                    Ok(_) => Ok(crate::compat::TerminationReason::Success),
                    Err(_) => Ok(crate::compat::TerminationReason::Exception),
                }
            }
        }
    }

    /// Execute a task and return the HTTP response directly
    pub async fn exec_http(&mut self, mut task: Task) -> Result<HttpResponse, String> {
        match task {
            Task::Fetch(ref mut init) => {
                let fetch_init = init.take().ok_or("FetchInit already consumed")?;
                self.trigger_fetch_event(fetch_init).await
            }
            Task::Scheduled(ref mut init) => {
                let scheduled_init = init.take().ok_or("ScheduledInit already consumed")?;
                self.trigger_scheduled_event(scheduled_init).await?;

                // Return empty response for scheduled events
                Ok(HttpResponse {
                    status: 200,
                    headers: vec![],
                    body: crate::task::ResponseBody::None,
                })
            }
        }
    }

    async fn trigger_fetch_event(
        &mut self,
        fetch_init: crate::task::FetchInit,
    ) -> Result<HttpResponse, String> {
        let req = &fetch_init.req;

        // Build headers object for JS
        let headers_json = serde_json::to_string(&req.headers).unwrap_or("{}".to_string());

        // Create Request object
        let body_str = req
            .body
            .as_ref()
            .and_then(|b| String::from_utf8(b.to_vec()).ok())
            .unwrap_or_default();

        let request_script = format!(
            r#"({{
                method: "{}",
                url: "{}",
                headers: {},
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
            .map_err(|_| "Failed to create Request object")?;

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
            .map_err(|_| "Failed to get trigger function")?
            .to_object(&self.runtime.context)
            .map_err(|_| "Trigger is not a function")?;

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
            return Err(error_msg);
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
                return Err("Response timeout: no response after 5s".to_string());
            }
        }

        // Extract response metadata from __lastResponse
        // Also call _getRawBody() and store result in __lastResponse._bodyBytes for direct access
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

                // Check for native stream
                const nativeStreamId = resp._nativeStreamId;
                if (nativeStreamId !== null && nativeStreamId !== undefined) {
                    // Streaming response - no body extraction
                    return JSON.stringify({
                        status: resp.status || 200,
                        headers: headers,
                        nativeStreamId: nativeStreamId,
                        hasBody: false
                    });
                }

                // Buffered response - extract body using _getRawBody()
                // Store directly on the response object for Rust access
                if (resp._getRawBody) {
                    resp._bodyBytes = resp._getRawBody();
                } else {
                    resp._bodyBytes = new Uint8Array(0);
                }

                return JSON.stringify({
                    status: resp.status || 200,
                    headers: headers,
                    nativeStreamId: null,
                    hasBody: resp._bodyBytes && resp._bodyBytes.length > 0
                });
            })()
        "#;

        let extract_result = self
            .runtime
            .context
            .evaluate_script(extract_script, 1)
            .map_err(|_| "Failed to extract response data")?;

        let json_str = extract_result
            .to_js_string(&self.runtime.context)
            .map_err(|_| "Failed to get response JSON")?
            .to_string();

        // Parse the extracted metadata
        #[derive(serde::Deserialize)]
        struct ExtractedResponse {
            status: u16,
            headers: Vec<(String, String)>,
            #[serde(rename = "nativeStreamId")]
            native_stream_id: Option<u64>,
            #[serde(rename = "hasBody")]
            has_body: bool,
        }

        let extracted: ExtractedResponse = serde_json::from_str(&json_str)
            .map_err(|e| format!("Failed to parse extracted response: {}", e))?;

        // Determine body type: streaming or buffered
        let body = if let Some(stream_id) = extracted.native_stream_id {
            // Native stream forward - create bounded channel for backpressure
            const RESPONSE_STREAM_BUFFER_SIZE: usize = 16;

            let (tx, rx) = tokio::sync::mpsc::channel(RESPONSE_STREAM_BUFFER_SIZE);
            let stream_manager = self.runtime.stream_manager.clone();

            // Spawn task to read from stream and forward to channel
            tokio::spawn(async move {
                loop {
                    match stream_manager.read_chunk(stream_id).await {
                        Ok(chunk) => match chunk {
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
                        },
                        Err(e) => {
                            let _ = tx.send(Err(e)).await;
                            break;
                        }
                    }
                }
            });

            ResponseBody::Stream(rx)
        } else {
            // Buffered body - read directly from __lastResponse._bodyBytes via TypedArray API
            let body_bytes = if extracted.has_body {
                // Get __lastResponse from global
                let global = self.runtime.context.get_global_object();
                if let Some(resp_val) = global.get_property(&self.runtime.context, "__lastResponse") {
                    if let Ok(resp_obj) = resp_val.to_object(&self.runtime.context) {
                        // Get _bodyBytes property
                        if let Some(body_val) = resp_obj.get_property(&self.runtime.context, "_bodyBytes") {
                            if let Ok(body_obj) = body_val.to_object(&self.runtime.context) {
                                // Use get_typed_array_buffer to read bytes directly
                                // Safety: we read synchronously and copy the data immediately
                                unsafe {
                                    if let Ok(slice) = body_obj.get_typed_array_buffer(&self.runtime.context) {
                                        Bytes::copy_from_slice(slice)
                                    } else {
                                        Bytes::new()
                                    }
                                }
                            } else {
                                Bytes::new()
                            }
                        } else {
                            Bytes::new()
                        }
                    } else {
                        Bytes::new()
                    }
                } else {
                    Bytes::new()
                }
            } else {
                Bytes::new()
            };
            ResponseBody::Bytes(body_bytes.clone())
        };

        // Send response back via channel
        let _ = fetch_init.res_tx.send(HttpResponse {
            status: extracted.status,
            headers: extracted.headers.clone(),
            body,
        });

        // Return response for exec_http
        let return_body = if extracted.native_stream_id.is_some() {
            ResponseBody::None
        } else if extracted.has_body {
            // Re-read from __lastResponse._bodyBytes
            let global = self.runtime.context.get_global_object();
            if let Some(resp_val) = global.get_property(&self.runtime.context, "__lastResponse") {
                if let Ok(resp_obj) = resp_val.to_object(&self.runtime.context) {
                    if let Some(body_val) = resp_obj.get_property(&self.runtime.context, "_bodyBytes") {
                        if let Ok(body_obj) = body_val.to_object(&self.runtime.context) {
                            unsafe {
                                if let Ok(slice) = body_obj.get_typed_array_buffer(&self.runtime.context) {
                                    ResponseBody::Bytes(Bytes::copy_from_slice(slice))
                                } else {
                                    ResponseBody::Bytes(Bytes::new())
                                }
                            }
                        } else {
                            ResponseBody::Bytes(Bytes::new())
                        }
                    } else {
                        ResponseBody::Bytes(Bytes::new())
                    }
                } else {
                    ResponseBody::Bytes(Bytes::new())
                }
            } else {
                ResponseBody::Bytes(Bytes::new())
            }
        } else {
            ResponseBody::Bytes(Bytes::new())
        };

        Ok(HttpResponse {
            status: extracted.status,
            headers: extracted.headers,
            body: return_body,
        })
    }

    async fn trigger_scheduled_event(
        &mut self,
        scheduled_init: crate::task::ScheduledInit,
    ) -> Result<(), String> {
        // Create event object
        let event_script = format!(
            r#"({{
                scheduledTime: {}
            }})"#,
            scheduled_init.time
        );

        let event_obj = self
            .runtime
            .context
            .evaluate_script(&event_script, 1)
            .map_err(|_| "Failed to create event")?;

        // Call trigger
        let trigger_script = r#"
            (function(event) {
                if (typeof globalThis.__triggerScheduled === 'function') {
                    return globalThis.__triggerScheduled(event);
                }
                throw new Error("No scheduled handler registered");
            })
        "#;

        let trigger_fn = self
            .runtime
            .context
            .evaluate_script(trigger_script, 1)
            .map_err(|_| "Failed to get trigger")?
            .to_object(&self.runtime.context)
            .map_err(|_| "Trigger not a function")?;

        if let Err(e) = trigger_fn.call_as_function(&self.runtime.context, None, &[event_obj]) {
            let error_msg = if let Ok(err_str) = e.to_js_string(&self.runtime.context) {
                let full_error = err_str.to_string();
                log::error!("Scheduled handler exception: {}", full_error);

                // Try to get stack trace
                if let Ok(err_obj) = e.to_object(&self.runtime.context) {
                    if let Some(stack_val) = err_obj.get_property(&self.runtime.context, "stack") {
                        if let Ok(stack_str) = stack_val.to_js_string(&self.runtime.context) {
                            log::error!("Stack trace:\n{}", stack_str);
                        }
                    }
                }

                format!("Scheduled handler exception: {}", full_error)
            } else {
                "Scheduled handler error (unknown)".to_string()
            };
            return Err(error_msg);
        }

        // Process callbacks with adaptive polling
        for iteration in 0..100 {
            self.runtime.process_callbacks();

            // Adaptive sleep
            let sleep_duration = if iteration < 10 {
                tokio::time::Duration::from_micros(1)
            } else if iteration < 50 {
                tokio::time::Duration::from_millis(1)
            } else {
                tokio::time::Duration::from_millis(10)
            };

            tokio::time::sleep(sleep_duration).await;
        }

        // Send completion
        let _ = scheduled_init.res_tx.send(());

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
                                // It's a Promise, wait for it to resolve
                                responseOrPromise
                                    .then(response => {
                                        globalThis.__lastResponse = response;
                                    })
                                    .catch(error => {
                                        console.error('[respondWith] Promise rejected:', error);
                                        globalThis.__lastResponse = new Response(
                                            'Promise rejected: ' + (error.message || error),
                                            { status: 500 }
                                        );
                                    });
                            } else {
                                // Direct Response object
                                globalThis.__lastResponse = responseOrPromise;
                            }
                        }
                    };

                    // Call handler synchronously
                    try {
                        handler(event);
                    } catch (error) {
                        console.error('[addEventListener] Error in fetch handler:', error);
                        globalThis.__lastResponse = new Response(
                            'Handler exception: ' + (error.message || error),
                            { status: 500 }
                        );
                    }
                };
            } else if (type === 'scheduled') {
                globalThis.__triggerScheduled = async function(event) {
                    const promises = [];
                    event.waitUntil = function(promise) {
                        promises.push(promise);
                    };

                    // Call handler
                    await handler(event);

                    // Wait for all promises
                    if (promises.length > 0) {
                        await Promise.all(promises);
                    }
                };
            }
        };
    "#;

    context
        .evaluate_script(add_event_listener_script, 1)
        .unwrap();
}
