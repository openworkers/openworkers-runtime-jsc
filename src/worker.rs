use crate::cpu_limit::CpuLimit;
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
    /// Evaluates a script for testing or inspection, under what is left of
    /// the CPU budget, and answers the text of its value. No value of the
    /// guest leaves the worker: reading one can run guest code, which must
    /// be a call under the budget.
    pub fn evaluate(&mut self, script: &str) -> Result<String, String> {
        let runtime = &mut self.runtime;
        let reason = |reason: TerminationReason| format!("{reason:?}");
        let evaluated = runtime
            .cpu
            .run(|| runtime.context.evaluate_script(script, 1))
            .map_err(reason)?;

        match evaluated {
            Ok(value) => runtime.value_text(&value).map_err(reason),
            Err(e) => Err(runtime.value_text(&e).unwrap_or_else(reason)),
        }
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
        let limits = limits.unwrap_or_default();
        let (mut runtime, scheduler_rx, callback_tx, stream_manager) = Runtime::new();
        let completion = Rc::new(RefCell::new(Completion::default()));

        // Before any guest code: the top level of the script is cut too
        runtime.cpu = CpuLimit::new(&runtime.context, limits.max_cpu_time_ms)?;

        // Setup addEventListener binding
        setup_event_listener(&mut runtime.context, completion.clone());

        // Setup environment variables
        setup_env(&mut runtime.context, &script.env, &script.bindings);

        // Setup console
        crate::runtime::bindings::setup_console(&mut runtime.context, Some(ops.clone()));

        // Extract JavaScript code from WorkerCode
        let js_code = script.code.as_js().ok_or_else(|| {
            TerminationReason::Exception("Only JavaScript code is supported".to_string())
        })?;

        // Load and evaluate the worker script, with the floor of a load
        runtime.cpu.refill_for_load();

        let evaluated = runtime
            .cpu
            .run(|| runtime.context.evaluate_script(js_code, 1))?;

        if let Err(e) = evaluated {
            return Err(TerminationReason::Exception(format!(
                "Script evaluation failed: {}",
                runtime.value_text(&e)?
            )));
        }

        // The floor was for the load only
        runtime.cpu.refill();

        // Start event loop in background
        let event_loop_handle = tokio::spawn(async move {
            run_event_loop(scheduler_rx, callback_tx, stream_manager, ops).await;
        });

        Ok(Self {
            runtime,
            completion,
            event_loop_handle,
            aborted: Arc::new(AtomicBool::new(false)),
            limits,
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

    /// Makes the worker ready for its next event: drops the timers, the
    /// pending callbacks and the tasks of the previous one. It runs no
    /// JavaScript that the guest can change, and its one call into JavaScript
    /// is under the CPU limit. An error means the worker must not be used.
    pub fn reset(&mut self) -> Result<(), TerminationReason> {
        if self.runtime.cpu.hit() {
            return Err(TerminationReason::CpuTimeLimit);
        }

        self.stop_tasks();

        // Answers to the previous event find no callback after the clear
        while self.runtime.callback_rx.try_recv().is_ok() {}

        self.runtime.cpu.refill();

        self.runtime
            .call("__clearCallbacks", &[])?
            .map(|_| ())
            .map_err(|_| TerminationReason::Other("the reset of the worker failed".to_string()))
    }

    /// Stops the timers, fetches, invokes and stream reads of the event that
    /// ended, so an idle worker queues no callback. The event loop takes its
    /// messages in order, so the tasks of a later event are not stopped.
    fn stop_tasks(&self) {
        self.runtime
            .scheduler_tx
            .send(crate::runtime::SchedulerMessage::ClearAll)
            .ok();
    }

    /// The callbacks that wait for the next entry into JavaScript.
    #[doc(hidden)]
    pub fn queued_callbacks(&self) -> usize {
        self.runtime.callback_rx.len()
    }

    /// Abort the worker execution
    pub fn abort(&mut self) {
        self.aborted.store(true, Ordering::SeqCst);
        self.event_loop_handle.abort();
    }

    /// Execute an event and return termination reason (openworkers-core compatible)
    ///
    /// A `ResponseBody::Stream` has to be read while this future is polled: the
    /// guest produces its body from here, and stops once `stream_buffer_size`
    /// chunks are waiting for a reader.
    pub async fn exec(&mut self, event: Event) -> Result<(), TerminationReason> {
        let result = self.exec_event(event).await;

        self.stop_tasks();

        // A cut entry leaves an error, or a completion that never comes
        if self.runtime.cpu.hit() {
            return Err(TerminationReason::CpuTimeLimit);
        }

        result
    }

    async fn exec_event(&mut self, mut event: Event) -> Result<(), TerminationReason> {
        // A worker whose entry was cut holds half an event
        if self.runtime.cpu.hit() {
            return Err(TerminationReason::CpuTimeLimit);
        }

        // Check if aborted before starting
        if self.aborted.load(Ordering::SeqCst) {
            return Err(TerminationReason::Aborted);
        }

        self.runtime.cpu.refill();

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
    pub async fn exec_http(&mut self, event: Event) -> Result<HttpResponse, TerminationReason> {
        let result = self.exec_http_event(event).await;

        self.stop_tasks();

        if self.runtime.cpu.hit() {
            return Err(TerminationReason::CpuTimeLimit);
        }

        result
    }

    async fn exec_http_event(
        &mut self,
        mut event: Event,
    ) -> Result<HttpResponse, TerminationReason> {
        if self.runtime.cpu.hit() {
            return Err(TerminationReason::CpuTimeLimit);
        }

        self.runtime.cpu.refill();

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
        // Nothing here feeds a host channel into a guest Request, and a body that
        // silently arrives empty costs the caller its whole wall-clock budget
        if let RequestBody::Stream(_) = fetch_init.req.body {
            return Err(TerminationReason::Other(
                "Streaming request bodies are not supported: collect the body into \
                 RequestBody::Bytes before calling exec"
                    .to_string(),
            ));
        }

        // One budget covers the handler and the response body it leaves behind
        let deadline = self.deadline();

        // The handler reports the response metadata via the native __completeEvent
        let (epoch, done_rx) = self.completion.borrow_mut().arm();

        self.call_fetch_handler(&fetch_init.req, epoch)?;

        let json_str = self.wait_for_completion(done_rx, deadline).await?;

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
        let stream = extracted.response_stream_id.and_then(|stream_id| {
            self.runtime
                .stream_manager
                .take_receiver(stream_id)
                .map(|rx| (stream_id, rx))
        });

        let (body, pending) = match stream {
            None => (ResponseBody::None, None),
            Some((stream_id, rx)) => {
                let (tx, response_rx) =
                    tokio::sync::mpsc::channel(self.limits.stream_buffer_size.max(1));

                (ResponseBody::Stream(response_rx), Some((stream_id, rx, tx)))
            }
        };

        // Send response back via channel
        let _ = fetch_init.res_tx.send(HttpResponse {
            status: extracted.status,
            headers: extracted.headers.clone(),
            body,
        });

        if let Some((stream_id, rx, tx)) = pending {
            let pumped = self.pump_response_body(rx, tx, deadline).await;

            // Whatever ended the stream, the guest has no reader left to write to
            self.runtime.stream_manager.close_stream(stream_id);

            pumped?;
        }

        // Return response for exec_http (body already sent via channel)
        Ok(HttpResponse {
            status: extracted.status,
            headers: extracted.headers,
            body: ResponseBody::None,
        })
    }

    /// Builds the Request and calls the fetch handler: one call into
    /// JavaScript, to a function the guest cannot replace
    fn call_fetch_handler(
        &mut self,
        req: &openworkers_core::HttpRequest,
        epoch: u64,
    ) -> Result<(), TerminationReason> {
        let headers_json = serde_json::to_string(&req.headers).expect("headers are strings");
        let body = match &req.body {
            RequestBody::Bytes(bytes) if !bytes.is_empty() => {
                crate::runtime::new_uint8_array(&mut self.runtime.context, bytes).map_err(|_| {
                    TerminationReason::Exception("Failed to create Request object".to_string())
                })?
            }
            _ => rusty_jsc::JSValue::null(&self.runtime.context),
        };

        let context = &self.runtime.context;
        let args = [
            rusty_jsc::JSValue::string(context, req.method.as_str()),
            rusty_jsc::JSValue::string(context, req.url.as_str()),
            rusty_jsc::JSValue::string(context, headers_json.as_str()),
            body,
            rusty_jsc::JSValue::number(context, epoch as f64),
        ];

        match self.runtime.call("__triggerFetch", &args)? {
            Ok(_) => Ok(()),
            Err(e) => Err(self.handler_error("Fetch", &e)?),
        }
    }

    /// The error of a handler that threw. Each read of the thrown value is a
    /// call under the CPU budget, since it can run guest code.
    fn handler_error(
        &self,
        kind: &str,
        error: &rusty_jsc::JSValue,
    ) -> Result<TerminationReason, TerminationReason> {
        let text = self.runtime.value_text(error)?;

        log::error!("{} handler exception: {}", kind, text);

        let context = &self.runtime.context;
        let stack = self.runtime.cpu.run(|| {
            let stack = error
                .to_object(context)
                .ok()?
                .get_property(context, "stack")?;

            stack.to_js_string(context).ok().map(|s| s.to_string())
        })?;

        if let Some(stack) = stack {
            log::error!("Stack trace:\n{}", stack);
        }

        Ok(TerminationReason::Exception(format!(
            "{} handler exception: {}",
            kind, text
        )))
    }

    /// Forward a response body to the host, driving JS for as long as the guest
    /// still has chunks to produce
    ///
    /// A stream that paces itself has nobody left to finish it once `exec` returns
    /// at the response head, so the host waits out its own deadline for a channel
    /// nobody closes.
    async fn pump_response_body(
        &mut self,
        mut rx: tokio::sync::mpsc::Receiver<StreamChunk>,
        tx: tokio::sync::mpsc::Sender<Result<bytes::Bytes, String>>,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<(), TerminationReason> {
        loop {
            let bytes = tokio::select! {
                biased;

                chunk = rx.recv() => match chunk {
                    Some(StreamChunk::Data(bytes)) => bytes,
                    // Done, or a writer that dropped without saying so
                    Some(StreamChunk::Done) | None => return Ok(()),
                    Some(StreamChunk::Error(e)) => {
                        let _ = tx.send(Err(e)).await;

                        return Ok(());
                    }
                },
                msg = self.runtime.callback_rx.recv() => {
                    match msg {
                        Some(msg) => {
                            self.runtime.handle_callback(msg)?;
                        }
                        None => return Err(TerminationReason::Other(
                            "Event loop stopped".to_string(),
                        )),
                    }

                    continue;
                }
                _ = at_deadline(deadline) => return Err(TerminationReason::WallClockTimeout),
            };

            tokio::select! {
                // The host walking away ends the stream, and the guest finds out
                // when its next write hits a closed channel
                sent = tx.send(Ok(bytes)) => if sent.is_err() {
                    return Ok(());
                },
                _ = at_deadline(deadline) => return Err(TerminationReason::WallClockTimeout),
            }
        }
    }

    async fn trigger_task_event(&mut self, task_init: TaskInit) -> Result<(), TerminationReason> {
        // Extract scheduled time if this is a schedule-triggered task
        let scheduled_time = match &task_init.source {
            Some(TaskSource::Schedule { time, .. }) => Some(*time),
            _ => None,
        };

        // The event goes in as JSON: a quote in the task id cannot break out
        let event_json = serde_json::json!({
            "taskId": task_init.task_id,
            "attempt": task_init.attempt,
            "payload": task_init.payload,
            "scheduledTime": scheduled_time,
        })
        .to_string();

        // The handler reports the task result via the native __completeEvent
        let (epoch, done_rx) = self.completion.borrow_mut().arm();
        let deadline = self.deadline();

        self.call_task_handler(&event_json, epoch)?;

        let json_str = self.wait_for_completion(done_rx, deadline).await?;

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

    /// Builds the event and calls the task handler: one call into
    /// JavaScript, to a function the guest cannot replace
    fn call_task_handler(&mut self, event_json: &str, epoch: u64) -> Result<(), TerminationReason> {
        let context = &self.runtime.context;
        let args = [
            rusty_jsc::JSValue::string(context, event_json),
            rusty_jsc::JSValue::number(context, epoch as f64),
        ];

        match self.runtime.call("__triggerTask", &args)? {
            Ok(_) => Ok(()),
            Err(e) => Err(self.handler_error("Task", &e)?),
        }
    }

    /// The instant the wall-clock budget runs out, or None when there is none
    fn deadline(&self) -> Option<tokio::time::Instant> {
        match self.limits.max_wall_clock_time_ms {
            0 => None,
            ms => Some(tokio::time::Instant::now() + tokio::time::Duration::from_millis(ms)),
        }
    }

    /// Drive JS callbacks until the event completes, honoring the wall-clock limit
    async fn wait_for_completion(
        &mut self,
        mut done_rx: tokio::sync::oneshot::Receiver<String>,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<String, TerminationReason> {
        loop {
            tokio::select! {
                result = &mut done_rx => {
                    return result.map_err(|_| {
                        TerminationReason::Other("Completion channel closed".to_string())
                    });
                }
                msg = self.runtime.callback_rx.recv() => {
                    match msg {
                        Some(msg) => {
                            self.runtime.handle_callback(msg)?;
                        }
                        None => {
                            return Err(TerminationReason::Other(
                                "Event loop stopped".to_string(),
                            ));
                        }
                    }
                }
                _ = at_deadline(deadline) => {
                    return Err(TerminationReason::WallClockTimeout);
                }
            }
        }
    }
}

/// Completes when the wall-clock budget runs out, and never without one
async fn at_deadline(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
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

    // Known limit: the helpers below (__finishFetch, __streamResponseBody,
    // __extractResponseMeta) are writable, and they call builtins the guest
    // can replace (Promise.prototype.then, JSON.stringify, Headers). A guest
    // that replaces one only breaks its own response: the code runs inside
    // its events, under the CPU budget and the wall clock of the request.
    let add_event_listener_script = r#"
        // Hand control back to the event loop, so the host can drain what it has
        globalThis.__yieldToHost = function() {
            return new Promise(resolve => setTimeout(resolve, 0));
        };

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
                            break;
                        }
                        if (value) {
                            // A full buffer is the host being slow, not a lost chunk
                            while (!__responseStreamWrite(streamId, value)) {
                                await __yieldToHost();
                            }
                        }
                    }
                } catch (e) {
                    // A guest that gives up mid-stream must not read as a whole body
                    const message = String((e && e.message) || e);
                    while (!__responseStreamError(streamId, message)) {
                        await __yieldToHost();
                    }
                }

                __responseStreamEnd(streamId);
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

        // The host calls __triggerFetch and __triggerTask from Rust. They are
        // made here, before guest code, hold the builtins they call, and are
        // read-only; addEventListener only fills the slots they read.
        (function() {
            const NativeRequest = Request;
            const parse = JSON.parse;
            const define = Object.defineProperty;
            const slots = { fetch: undefined, scheduled: undefined, task: undefined };

            const fixed = function(name, value) {
                define(globalThis, name, {
                    value: value,
                    writable: false,
                    configurable: false,
                    enumerable: false,
                });
            };

            fixed('__triggerFetch', function(method, url, headers, body, epoch) {
                const trigger = slots.fetch;

                if (trigger === undefined) {
                    throw new Error("No fetch handler registered");
                }

                const request = new NativeRequest(url, {
                    method: method,
                    headers: parse(headers),
                    body: body
                });

                trigger(request, epoch);
            });

            fixed('__triggerTask', function(json, epoch) {
                const trigger = slots.task !== undefined ? slots.task : slots.scheduled;

                if (trigger === undefined) {
                    throw new Error("No task handler registered");
                }

                const event = parse(json);

                if (event.scheduledTime === null) {
                    event.scheduledTime = undefined;
                }

                return trigger(event, epoch);
            });

            fixed('__setTrigger', function(type, trigger) {
                slots[type] = trigger;
            });
        })();

        globalThis.addEventListener = function(type, handler) {
            if (type === 'fetch') {
                __setTrigger('fetch', function(request, epoch) {
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
                });
            } else if (type === 'scheduled') {
                __setTrigger('scheduled', async function(event, epoch) {
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
                });
            } else if (type === 'task') {
                __setTrigger('task', async function(event, epoch) {
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
                });
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
    bindings: &[openworkers_core::BindingInfo],
) {
    let env_json = match env {
        Some(env_map) => serde_json::to_string(env_map).expect("env is a map of strings"),
        None => "{}".to_string(),
    };

    let script = format!(
        r#"Object.defineProperty(globalThis, 'env', {{
            value: Object.assign({}, {{{}}}),
            writable: false,
            enumerable: true,
            configurable: false
        }});"#,
        env_json,
        binding_objects(bindings).join(",")
    );

    context.evaluate_script(&script, 1).unwrap();
}

/// One `env` object per binding this runtime serves, which wins over a variable
/// of the same name
#[cfg(feature = "invoke")]
fn binding_objects(bindings: &[openworkers_core::BindingInfo]) -> Vec<String> {
    bindings
        .iter()
        .filter(|binding| binding.binding_type == openworkers_core::BindingType::Invoke)
        .map(|binding| {
            let name = serde_json::to_string(&binding.name).expect("a string serialises");

            format!("{name}: __makeInvokeBinding({name})")
        })
        .collect()
}

#[cfg(not(feature = "invoke"))]
fn binding_objects(_bindings: &[openworkers_core::BindingInfo]) -> Vec<String> {
    Vec::new()
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
