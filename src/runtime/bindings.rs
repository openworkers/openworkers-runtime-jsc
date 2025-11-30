use super::{CallbackId, SchedulerMessage, stream_manager::StreamId};
use openworkers_core::{LogEvent, LogLevel, LogSender};
use rusty_jsc::{JSContext, JSObject, JSValue};
use rusty_jsc_macros::callback;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// Shared state for timer callbacks
pub struct TimerState {
    pub scheduler_tx: mpsc::UnboundedSender<SchedulerMessage>,
    pub callbacks: Arc<Mutex<HashMap<CallbackId, JSObject>>>,
    pub next_id: Arc<Mutex<CallbackId>>,
}

/// Setup console bindings (log, info, warn, error, debug) with log_tx support
pub fn setup_console(context: &mut JSContext, log_tx: Option<LogSender>) {
    let log_tx = Arc::new(Mutex::new(log_tx));

    // Create native __console_log function that accepts level and message
    let log_tx_clone = log_tx.clone();
    let console_log_fn = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.len() < 2 {
                return Ok(JSValue::undefined(&ctx));
            }

            let level_num = args[0].to_number(&ctx).map(|n| n as i32).unwrap_or(2);
            let level = match level_num {
                0 => LogLevel::Error,
                1 => LogLevel::Warn,
                _ => LogLevel::Info,
            };

            let msg = args[1]
                .to_js_string(&ctx)
                .map(|s| s.to_string())
                .unwrap_or_default();

            // Send to log_tx if available
            if let Ok(guard) = log_tx_clone.lock() {
                if let Some(ref tx) = *guard {
                    let _ = tx.send(LogEvent {
                        level: level.clone(),
                        message: msg.clone(),
                    });
                }
            }

            // Also print to stdout
            let prefix = match level {
                LogLevel::Error => "[ERROR]",
                LogLevel::Warn => "[WARN]",
                LogLevel::Info | LogLevel::Log => "[LOG]",
                LogLevel::Debug | LogLevel::Trace => "[DEBUG]",
            };
            println!("{} {}", prefix, msg);

            Ok(JSValue::undefined(&ctx))
        }
    );

    // Add __console_log to global
    let mut global = context.get_global_object();
    global
        .set_property(context, "__console_log", console_log_fn.into())
        .unwrap();

    // Create console object via JS that calls __console_log with appropriate level
    let console_script = r#"
        globalThis.console = {
            log: function(...args) {
                const msg = args.map(a => typeof a === 'object' ? JSON.stringify(a) : String(a)).join(' ');
                __console_log(2, msg);
            },
            info: function(...args) {
                const msg = args.map(a => typeof a === 'object' ? JSON.stringify(a) : String(a)).join(' ');
                __console_log(2, msg);
            },
            warn: function(...args) {
                const msg = args.map(a => typeof a === 'object' ? JSON.stringify(a) : String(a)).join(' ');
                __console_log(1, msg);
            },
            error: function(...args) {
                const msg = args.map(a => typeof a === 'object' ? JSON.stringify(a) : String(a)).join(' ');
                __console_log(0, msg);
            },
            debug: function(...args) {
                const msg = args.map(a => typeof a === 'object' ? JSON.stringify(a) : String(a)).join(' ');
                __console_log(2, msg);
            }
        };
    "#;

    context.evaluate_script(console_script, 1).unwrap();
}

#[callback]
fn queue_microtask_fn(
    mut ctx: JSContext,
    _function: JSObject,
    _this: JSObject,
    args: &[JSValue],
) -> Result<JSValue, JSValue> {
    if args.is_empty() {
        return Err(JSValue::string(&ctx, "queueMicrotask requires a function"));
    }

    let callback = match args[0].to_object(&ctx) {
        Ok(obj) => obj,
        Err(_) => return Err(JSValue::string(&ctx, "Argument must be a function")),
    };

    // Use Promise.resolve().then() to queue as microtask
    // This is the standard web platform approach
    let script = r#"
        (function(callback) {
            Promise.resolve().then(callback);
        })
    "#;

    match ctx.evaluate_script(script, 1) {
        Ok(wrapper) => {
            if let Ok(wrapper_fn) = wrapper.to_object(&ctx) {
                let _ = wrapper_fn.call_as_function(&ctx, None, &[callback.into()]);
            }
        }
        Err(_) => {}
    }

    Ok(JSValue::undefined(&ctx))
}

/// Setup queueMicrotask binding
pub fn setup_microtask(context: &mut JSContext) {
    let microtask_fn = JSValue::callback(context, Some(queue_microtask_fn));

    let mut global = context.get_global_object();
    global
        .set_property(context, "queueMicrotask", microtask_fn)
        .unwrap();
}

/// Setup fetch API
pub fn setup_fetch(
    context: &mut JSContext,
    scheduler_tx: mpsc::UnboundedSender<SchedulerMessage>,
    callbacks: Arc<Mutex<HashMap<CallbackId, JSObject>>>,
    next_id: Arc<Mutex<CallbackId>>,
) {
    let scheduler_tx_clone = scheduler_tx;
    let callbacks_clone = callbacks;
    let next_id_clone = next_id;

    // Create fetch function
    let fetch_fn = rusty_jsc::callback_closure!(
        context,
        move |mut ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.is_empty() {
                return Err(JSValue::string(&ctx, "fetch requires a URL"));
            }

            // Get URL
            let url = match args[0].to_js_string(&ctx) {
                Ok(url_str) => url_str.to_string(),
                Err(_) => return Err(JSValue::string(&ctx, "URL must be a string")),
            };

            // Parse fetch options (method, headers, body)
            let options_val = if args.len() > 1 {
                Some(args[1].clone())
            } else {
                None
            };

            let request = match super::fetch::parse_fetch_options(&ctx, url, options_val) {
                Ok(req) => req,
                Err(e) => return Err(JSValue::string(&ctx, e.as_str())),
            };

            // Create a Promise and store resolve/reject callbacks
            let promise_script = r#"
                new Promise((resolve, reject) => {
                    globalThis.__fetchResolve = resolve;
                    globalThis.__fetchReject = reject;
                })
            "#;

            let promise = match ctx.evaluate_script(promise_script, 1) {
                Ok(p) => p,
                Err(_) => return Err(JSValue::string(&ctx, "Failed to create Promise")),
            };

            // Get resolve and reject callbacks
            let global = ctx.get_global_object();

            let resolve_callback = global
                .get_property(&ctx, "__fetchResolve")
                .and_then(|v| v.to_object(&ctx).ok())
                .ok_or_else(|| JSValue::string(&ctx, "Failed to get resolve callback"))?;

            let _reject_callback = global
                .get_property(&ctx, "__fetchReject")
                .and_then(|v| v.to_object(&ctx).ok())
                .ok_or_else(|| JSValue::string(&ctx, "Failed to get reject callback"))?;

            // Generate callback ID for resolve
            let callback_id = {
                let mut next = next_id_clone.lock().unwrap();
                let id = *next;
                *next += 1;
                id
            };

            // Store resolve callback (we'll call it with Response or Error)
            {
                let mut cbs = callbacks_clone.lock().unwrap();
                cbs.insert(callback_id, resolve_callback);
                // For reject, we could store it separately, but for now we'll use the same callback
            }

            log::debug!(
                "fetch: scheduled streaming {} {} (promise_id: {})",
                request.method.as_str(),
                request.url,
                callback_id
            );

            // Schedule the fetch with streaming
            let _ = scheduler_tx_clone.send(SchedulerMessage::FetchStreaming(callback_id, request));

            // Return the Promise
            Ok(promise)
        }
    );

    // Add native fetch to global object (as __nativeFetch)
    let mut global = context.get_global_object();
    global
        .set_property(context, "__nativeFetch", fetch_fn.into())
        .unwrap();

    // Create JS wrapper that handles ReadableStream bodies
    let wrapper_code = r#"
        globalThis.fetch = async function(url, options = {}) {
            // If body is a ReadableStream, consume it first
            if (options && options.body instanceof ReadableStream) {
                console.warn('[fetch] ReadableStream body detected - buffering entire stream before sending');
                const reader = options.body.getReader();
                const chunks = [];

                while (true) {
                    const { done, value } = await reader.read();
                    if (done) break;
                    if (value) chunks.push(value);
                }

                // Combine chunks into a single Uint8Array
                if (chunks.length > 0) {
                    const totalLength = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
                    const combined = new Uint8Array(totalLength);
                    let offset = 0;
                    for (const chunk of chunks) {
                        combined.set(chunk, offset);
                        offset += chunk.length;
                    }
                    // Convert to string for the native fetch
                    options = {
                        ...options,
                        body: new TextDecoder().decode(combined)
                    };
                } else {
                    options = { ...options, body: undefined };
                }
            }

            return __nativeFetch(url, options);
        };
    "#;

    context
        .evaluate_script(wrapper_code, 1)
        .expect("Failed to setup fetch wrapper");
}

/// Setup timer bindings (setTimeout, setInterval, clearTimeout, clearInterval)
pub fn setup_timer(
    context: &mut JSContext,
    scheduler_tx: mpsc::UnboundedSender<SchedulerMessage>,
    callbacks: Arc<Mutex<HashMap<CallbackId, JSObject>>>,
    next_id: Arc<Mutex<CallbackId>>,
    intervals: Arc<Mutex<std::collections::HashSet<CallbackId>>>,
) {
    // Setup setTimeout
    setup_set_timeout(
        context,
        scheduler_tx.clone(),
        callbacks.clone(),
        next_id.clone(),
    );

    // Setup setInterval
    setup_set_interval(
        context,
        scheduler_tx.clone(),
        callbacks.clone(),
        next_id.clone(),
        intervals,
    );

    // Setup clearTimeout and clearInterval (same implementation)
    setup_clear_timer(context, scheduler_tx.clone());
}

/// Setup setTimeout binding
fn setup_set_timeout(
    context: &mut JSContext,
    scheduler_tx: mpsc::UnboundedSender<SchedulerMessage>,
    callbacks: Arc<Mutex<HashMap<CallbackId, JSObject>>>,
    next_id: Arc<Mutex<CallbackId>>,
) {
    let callbacks_clone = callbacks;
    let next_id_clone = next_id;
    let scheduler_tx_clone = scheduler_tx;

    // Create setTimeout function using callback_closure to capture Rust state
    let set_timeout = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.len() < 2 {
                return Err(JSValue::string(&ctx, "setTimeout requires 2 arguments"));
            }

            // Get the callback function
            let callback = match args[0].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Err(JSValue::string(&ctx, "First argument must be a function")),
            };

            // Get the delay
            let delay = match args[1].to_number(&ctx) {
                Ok(d) => d as u64,
                Err(_) => return Err(JSValue::string(&ctx, "Second argument must be a number")),
            };

            // Generate callback ID
            let callback_id = {
                let mut next = next_id_clone.lock().unwrap();
                let id = *next;
                *next += 1;
                id
            };

            // Store the callback
            {
                let mut cbs = callbacks_clone.lock().unwrap();
                cbs.insert(callback_id, callback);
            }

            // Schedule the timeout
            let _ = scheduler_tx_clone.send(SchedulerMessage::ScheduleTimeout(callback_id, delay));

            log::debug!(
                "setTimeout: registered callback {} with delay {}ms",
                callback_id,
                delay
            );

            // Return the timeout ID
            Ok(JSValue::number(&ctx, callback_id as f64))
        }
    );

    // Add setTimeout to global object
    let mut global = context.get_global_object();
    global
        .set_property(context, "setTimeout", set_timeout.into())
        .unwrap();
}

/// Setup setInterval binding
fn setup_set_interval(
    context: &mut JSContext,
    scheduler_tx: mpsc::UnboundedSender<SchedulerMessage>,
    callbacks: Arc<Mutex<HashMap<CallbackId, JSObject>>>,
    next_id: Arc<Mutex<CallbackId>>,
    intervals: Arc<Mutex<std::collections::HashSet<CallbackId>>>,
) {
    let callbacks_clone = callbacks;
    let next_id_clone = next_id;
    let scheduler_tx_clone = scheduler_tx;
    let intervals_clone = intervals;

    // Create setInterval function
    let set_interval = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.len() < 2 {
                return Err(JSValue::string(&ctx, "setInterval requires 2 arguments"));
            }

            // Get the callback function
            let callback = match args[0].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Err(JSValue::string(&ctx, "First argument must be a function")),
            };

            // Get the interval
            let interval = match args[1].to_number(&ctx) {
                Ok(d) => d as u64,
                Err(_) => return Err(JSValue::string(&ctx, "Second argument must be a number")),
            };

            // Generate callback ID
            let callback_id = {
                let mut next = next_id_clone.lock().unwrap();
                let id = *next;
                *next += 1;
                id
            };

            // Store the callback
            {
                let mut cbs = callbacks_clone.lock().unwrap();
                cbs.insert(callback_id, callback);
            }

            // Mark as interval
            {
                let mut intervals = intervals_clone.lock().unwrap();
                intervals.insert(callback_id);
            }

            // Schedule the interval
            let _ =
                scheduler_tx_clone.send(SchedulerMessage::ScheduleInterval(callback_id, interval));

            log::debug!(
                "setInterval: registered callback {} with interval {}ms",
                callback_id,
                interval
            );

            // Return the interval ID
            Ok(JSValue::number(&ctx, callback_id as f64))
        }
    );

    // Add setInterval to global object
    let mut global = context.get_global_object();
    global
        .set_property(context, "setInterval", set_interval.into())
        .unwrap();
}

/// Setup clearTimeout and clearInterval bindings (same implementation for both)
fn setup_clear_timer(
    context: &mut JSContext,
    scheduler_tx: mpsc::UnboundedSender<SchedulerMessage>,
) {
    let scheduler_tx_clone = scheduler_tx.clone();

    // Create clearTimeout function
    let clear_timeout = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.is_empty() {
                return Ok(JSValue::undefined(&ctx));
            }

            // Get the timer ID
            let timer_id = match args[0].to_number(&ctx) {
                Ok(id) => id as u64,
                Err(_) => return Ok(JSValue::undefined(&ctx)),
            };

            // Send clear message
            let _ = scheduler_tx_clone.send(SchedulerMessage::ClearTimer(timer_id));

            log::debug!("clearTimeout: cleared timer {}", timer_id);

            Ok(JSValue::undefined(&ctx))
        }
    );

    let scheduler_tx_clone2 = scheduler_tx;

    // Create clearInterval function (same implementation)
    let clear_interval = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.is_empty() {
                return Ok(JSValue::undefined(&ctx));
            }

            // Get the timer ID
            let timer_id = match args[0].to_number(&ctx) {
                Ok(id) => id as u64,
                Err(_) => return Ok(JSValue::undefined(&ctx)),
            };

            // Send clear message
            let _ = scheduler_tx_clone2.send(SchedulerMessage::ClearTimer(timer_id));

            log::debug!("clearInterval: cleared timer {}", timer_id);

            Ok(JSValue::undefined(&ctx))
        }
    );

    // Add to global object
    let mut global = context.get_global_object();
    global
        .set_property(context, "clearTimeout", clear_timeout.into())
        .unwrap();
    global
        .set_property(context, "clearInterval", clear_interval.into())
        .unwrap();
}

/// Setup stream operations for native streaming (__nativeStreamRead, __nativeStreamCancel)
pub fn setup_stream_ops(
    context: &mut JSContext,
    scheduler_tx: mpsc::UnboundedSender<SchedulerMessage>,
    callbacks: Arc<Mutex<HashMap<CallbackId, JSObject>>>,
    next_id: Arc<Mutex<CallbackId>>,
) {
    // Create __nativeStreamRead(stream_id, resolve_callback)
    // This is called from JS to request the next chunk from a stream
    let scheduler_tx_clone = scheduler_tx.clone();
    let callbacks_clone = callbacks.clone();
    let next_id_clone = next_id.clone();

    let stream_read = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.len() < 2 {
                return Err(JSValue::string(
                    &ctx,
                    "__nativeStreamRead requires stream_id and callback",
                ));
            }

            // Get stream ID
            let stream_id = match args[0].to_number(&ctx) {
                Ok(id) => id as StreamId,
                Err(_) => return Err(JSValue::string(&ctx, "stream_id must be a number")),
            };

            // Get callback function
            let callback = match args[1].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Err(JSValue::string(&ctx, "callback must be a function")),
            };

            // Generate callback ID
            let callback_id = {
                let mut next = next_id_clone.lock().unwrap();
                let id = *next;
                *next += 1;
                id
            };

            // Store callback
            {
                let mut cbs = callbacks_clone.lock().unwrap();
                cbs.insert(callback_id, callback);
            }

            // Send StreamRead message to scheduler
            let _ = scheduler_tx_clone.send(SchedulerMessage::StreamRead(callback_id, stream_id));

            log::debug!(
                "__nativeStreamRead: reading stream {} (callback {})",
                stream_id,
                callback_id
            );

            Ok(JSValue::undefined(&ctx))
        }
    );

    // Create __nativeStreamCancel(stream_id) - sends cancel message to scheduler
    let scheduler_tx_clone2 = scheduler_tx;

    let stream_cancel = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.is_empty() {
                return Err(JSValue::string(
                    &ctx,
                    "__nativeStreamCancel requires stream_id",
                ));
            }

            // Get stream ID
            let stream_id = match args[0].to_number(&ctx) {
                Ok(id) => id as StreamId,
                Err(_) => return Err(JSValue::string(&ctx, "stream_id must be a number")),
            };

            // Send StreamCancel message
            let _ = scheduler_tx_clone2.send(SchedulerMessage::StreamCancel(stream_id));

            log::debug!("__nativeStreamCancel: cancelled stream {}", stream_id);

            Ok(JSValue::undefined(&ctx))
        }
    );

    // Add to global object
    let mut global = context.get_global_object();
    global
        .set_property(context, "__nativeStreamRead", stream_read.into())
        .unwrap();
    global
        .set_property(context, "__nativeStreamCancel", stream_cancel.into())
        .unwrap();

    // Create JS helper __createNativeStream(streamId) that creates a ReadableStream
    // pulling from native Rust code
    // The stream is marked with _nativeStreamId so we can detect it later for forwarding
    let create_native_stream_script = r#"
        globalThis.__createNativeStream = function(streamId) {
            const stream = new ReadableStream({
                pull(controller) {
                    return new Promise((resolve) => {
                        __nativeStreamRead(streamId, (result) => {
                            if (result.error) {
                                controller.error(new Error(result.error));
                            } else if (result.done) {
                                controller.close();
                            } else {
                                controller.enqueue(result.value);
                            }
                            resolve();
                        });
                    });
                },
                cancel() {
                    __nativeStreamCancel(streamId);
                }
            });
            // Mark this stream as a native stream so we can forward it directly
            stream._nativeStreamId = streamId;
            return stream;
        };
    "#;

    context
        .evaluate_script(create_native_stream_script, 1)
        .expect("Failed to setup __createNativeStream");
}

/// Setup response stream operations for streaming all responses
/// __responseStreamCreate() - creates a stream for response body, returns stream ID
/// __responseStreamWrite(stream_id, Uint8Array) - writes bytes to the stream
/// __responseStreamEnd(stream_id) - signals end of stream
pub fn setup_response_stream_ops(
    context: &mut JSContext,
    stream_manager: Arc<super::stream_manager::StreamManager>,
) {
    // __responseStreamCreate() -> stream_id
    let manager_clone = stream_manager.clone();
    let create_stream = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, _args: &[JSValue]| {
            let stream_id = manager_clone.create_stream("response".to_string());
            log::debug!("__responseStreamCreate: created stream {}", stream_id);
            Ok(JSValue::number(&ctx, stream_id as f64))
        }
    );

    // __responseStreamWrite(stream_id, Uint8Array) -> boolean
    let manager_clone = stream_manager.clone();
    let write_stream = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.len() < 2 {
                return Err(JSValue::string(
                    &ctx,
                    "__responseStreamWrite requires stream_id and data",
                ));
            }

            let stream_id = match args[0].to_number(&ctx) {
                Ok(id) => id as StreamId,
                Err(_) => return Err(JSValue::string(&ctx, "stream_id must be a number")),
            };

            // Get the Uint8Array data
            let data_obj = match args[1].to_object(&ctx) {
                Ok(obj) => obj,
                Err(_) => return Err(JSValue::string(&ctx, "data must be a Uint8Array")),
            };

            // Read bytes from the TypedArray
            let bytes = unsafe {
                match data_obj.get_typed_array_buffer(&ctx) {
                    Ok(slice) => bytes::Bytes::copy_from_slice(slice),
                    Err(_) => return Err(JSValue::string(&ctx, "Failed to read TypedArray")),
                }
            };

            // Try to write the chunk (non-blocking)
            match manager_clone
                .try_write_chunk(stream_id, super::stream_manager::StreamChunk::Data(bytes))
            {
                Ok(()) => Ok(JSValue::boolean(&ctx, true)),
                Err(e) => {
                    log::warn!("__responseStreamWrite error: {}", e);
                    Ok(JSValue::boolean(&ctx, false))
                }
            }
        }
    );

    // __responseStreamEnd(stream_id)
    let manager_clone = stream_manager;
    let end_stream = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.is_empty() {
                return Err(JSValue::string(
                    &ctx,
                    "__responseStreamEnd requires stream_id",
                ));
            }

            let stream_id = match args[0].to_number(&ctx) {
                Ok(id) => id as StreamId,
                Err(_) => return Err(JSValue::string(&ctx, "stream_id must be a number")),
            };

            // Send Done signal
            let _ =
                manager_clone.try_write_chunk(stream_id, super::stream_manager::StreamChunk::Done);

            log::debug!("__responseStreamEnd: ended stream {}", stream_id);
            Ok(JSValue::undefined(&ctx))
        }
    );

    // Add to global object
    let mut global = context.get_global_object();
    global
        .set_property(context, "__responseStreamCreate", create_stream.into())
        .unwrap();
    global
        .set_property(context, "__responseStreamWrite", write_stream.into())
        .unwrap();
    global
        .set_property(context, "__responseStreamEnd", end_stream.into())
        .unwrap();
}
