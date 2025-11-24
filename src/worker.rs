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
        let (mut runtime, scheduler_rx, callback_tx) = Runtime::new();

        // Setup addEventListener binding
        setup_event_listener(&mut runtime.context);

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
            run_event_loop(scheduler_rx, callback_tx).await;
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
                    body: None,
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
        let trigger_script = r#"
            (async function(request) {
                if (typeof globalThis.__triggerFetch === 'function') {
                    const response = await globalThis.__triggerFetch(request);
                    // Store response data for extraction
                    if (response && response.text) {
                        const bodyText = await response.text();
                        globalThis.__lastResponse = {
                            status: response.status || 200,
                            statusText: response.statusText || 'OK',
                            body: bodyText
                        };
                    } else {
                        globalThis.__lastResponse = {
                            status: 200,
                            statusText: 'OK',
                            body: String(response)
                        };
                    }
                    return globalThis.__lastResponse;
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

        // Process callbacks to resolve promises
        for _ in 0..100 {
            self.runtime.process_callbacks();
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        // Extract response from globalThis.__lastResponse
        let get_response = r#"globalThis.__lastResponse"#;
        let response_val = self
            .runtime
            .context
            .evaluate_script(get_response, 1)
            .map_err(|_| "Failed to get response")?;

        let response = if let Ok(resp_obj) = response_val.to_object(&self.runtime.context) {
            let status = resp_obj
                .get_property(&self.runtime.context, "status")
                .and_then(|v| v.to_number(&self.runtime.context).ok())
                .unwrap_or(200.0) as u16;

            let body_str = resp_obj
                .get_property(&self.runtime.context, "body")
                .and_then(|v| v.to_js_string(&self.runtime.context).ok())
                .map(|s| s.to_string())
                .unwrap_or_default();

            HttpResponse {
                status,
                headers: vec![],
                body: Some(Bytes::from(body_str)),
            }
        } else {
            HttpResponse {
                status: 500,
                headers: vec![],
                body: Some(Bytes::from("Failed to extract response")),
            }
        };

        // Send response back
        let _ = fetch_init.res_tx.send(response.clone());

        Ok(response)
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

        // Process callbacks
        for _ in 0..10 {
            self.runtime.process_callbacks();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
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
fn setup_event_listener(context: &mut rusty_jsc::JSContext) {
    let add_event_listener_script = r#"
        globalThis.addEventListener = function(type, handler) {
            if (type === 'fetch') {
                globalThis.__triggerFetch = function(request) {
                    const event = {
                        request: request,
                        respondWith: function(responsePromise) {
                            this._response = responsePromise;
                        }
                    };
                    handler(event);
                    return event._response || new Response("No response");
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

        globalThis.Response = function(body, init) {
            init = init || {};
            return {
                status: init.status || 200,
                statusText: init.statusText || 'OK',
                headers: init.headers || {},
                text: () => Promise.resolve(String(body)),
                json: () => Promise.resolve(JSON.parse(String(body))),
            };
        };
    "#;

    context
        .evaluate_script(add_event_listener_script, 1)
        .unwrap();
}
