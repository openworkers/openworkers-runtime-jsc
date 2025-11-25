use crate::runtime::{Runtime, run_event_loop};
use crate::task::{HttpResponse, Task};
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

        // Create a oneshot channel for the response
        let (response_tx, response_rx) = tokio::sync::oneshot::channel::<String>();

        // Store the sender in runtime so JS can use it
        {
            let mut tx_lock = self.runtime.fetch_response_tx.lock().unwrap();
            *tx_lock = Some(response_tx);
        }

        // Call the fetch event trigger (set by addEventListener)
        let trigger_script = r#"
            (async function(request) {
                if (typeof globalThis.__triggerFetch === 'function') {
                    const response = await globalThis.__triggerFetch(request);
                    // Extract response data
                    if (response && response.text) {
                        const bodyText = await response.text();

                        // Extract headers
                        const headers = [];
                        if (response.headers) {
                            // Headers class is iterable
                            if (response.headers instanceof Headers) {
                                for (const [key, value] of response.headers) {
                                    headers.push([key, String(value)]);
                                }
                            } else if (typeof response.headers === 'object') {
                                for (const [key, value] of Object.entries(response.headers)) {
                                    headers.push([key, String(value)]);
                                }
                            }
                        }

                        const responseData = {
                            status: response.status || 200,
                            statusText: response.statusText || 'OK',
                            headers: headers,
                            body: bodyText
                        };
                        // Send to Rust via native function
                        if (typeof globalThis.__sendFetchResponse === 'function') {
                            globalThis.__sendFetchResponse(JSON.stringify(responseData));
                        }
                        return responseData;
                    } else {
                        const responseData = {
                            status: 200,
                            statusText: 'OK',
                            headers: [],
                            body: String(response)
                        };
                        if (typeof globalThis.__sendFetchResponse === 'function') {
                            globalThis.__sendFetchResponse(JSON.stringify(responseData));
                        }
                        return responseData;
                    }
                }
                throw new Error("No fetch handler registered");
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

        // Wait for response with adaptive polling
        // Fast polling for sync responses, timeout after ~2s for unresponsive handlers
        let response_json = tokio::select! {
            result = response_rx => {
                result.map_err(|_| "Response channel closed - handler may not have called respondWith")?
            }
            _ = async {
                // Adaptive polling: fast checks first, then slower
                // Total timeout: 10 x 1µs + 100 x 1ms + 190 x 10ms = ~2s
                for iteration in 0..300 {
                    self.runtime.process_callbacks();

                    // Adaptive sleep: fast for first checks, slower later
                    let sleep_duration = if iteration < 10 {
                        // First 10 iterations: minimal sleep (for immediate sync responses)
                        tokio::time::Duration::from_micros(1)
                    } else if iteration < 110 {
                        // Next 100 iterations: 1ms sleep (for fast async < 100ms)
                        tokio::time::Duration::from_millis(1)
                    } else {
                        // After 110ms: 10ms sleep (for slow operations)
                        tokio::time::Duration::from_millis(10)
                    };

                    tokio::time::sleep(sleep_duration).await;
                }
            } => {
                return Err("Response timeout: no response after 2s".to_string());
            }
        };

        // Parse the JSON response
        #[derive(serde::Deserialize)]
        struct ResponseData {
            status: u16,
            headers: Vec<(String, String)>,
            body: String,
        }

        let response_data: ResponseData = serde_json::from_str(&response_json)
            .map_err(|e| format!("Failed to parse response JSON: {}", e))?;

        // Send response back
        let _ = fetch_init.res_tx.send(HttpResponse {
            status: response_data.status,
            headers: response_data.headers.clone(),
            body: crate::task::ResponseBody::Bytes(Bytes::from(response_data.body.clone())),
        });

        Ok(HttpResponse {
            status: response_data.status,
            headers: response_data.headers,
            body: crate::task::ResponseBody::Bytes(Bytes::from(response_data.body)),
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
                globalThis.__triggerFetch = async function(request) {
                    // Create event with Promise-based respondWith
                    let resolveResponse;
                    const responsePromise = new Promise(resolve => {
                        resolveResponse = resolve;
                    });

                    const event = {
                        request: request,
                        _responded: false,
                        respondWith: function(response) {
                            if (this._responded) {
                                throw new Error('respondWith already called');
                            }
                            this._responded = true;
                            // Handle both Promise and direct Response
                            Promise.resolve(response).then(resolveResponse);
                        }
                    };

                    // Call handler - may be sync or async
                    const handlerResult = handler(event);

                    // If handler returns a promise, wait for it
                    if (handlerResult && typeof handlerResult.then === 'function') {
                        await handlerResult;
                    }

                    // If respondWith wasn't called yet, wait a tick for microtasks
                    if (!event._responded) {
                        await Promise.resolve();
                    }

                    // If still no response after microtasks, wait a bit more
                    if (!event._responded) {
                        await new Promise(r => setTimeout(r, 0));
                    }

                    // Final check
                    if (!event._responded) {
                        return new Response("No response");
                    }

                    return responsePromise;
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
