use super::stream_manager::StreamId;
use super::stream_manager::TryWriteError;
use super::{CallbackId, SchedulerMessage};
use openworkers_core::{LogLevel, OperationsHandle};
use rusty_jsc::{JSContext, JSObject, JSValue};
use rusty_jsc_macros::callback;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Ids are handed out from the JS thread only, so no lock is needed
pub(crate) type CallbackCounter = Rc<Cell<CallbackId>>;

fn next_callback_id(counter: &CallbackCounter) -> CallbackId {
    let id = counter.get();

    counter.set(id + 1);

    id
}

/// JSC collects whatever JS cannot reach, so pending callbacks live in a JS map
/// rather than in a Rust one the collector never sees
const CALLBACK_REGISTRY_JS: &str = r#"
    (function() {
        const callbacks = new Map();

        globalThis.__storeCallback = function(id, callback) {
            callbacks.set(id, callback);
        };

        globalThis.__dropCallback = function(id) {
            callbacks.delete(id);
        };

        globalThis.__runCallback = function(id, ...args) {
            const callback = callbacks.get(id);
            callbacks.delete(id);

            if (callback !== undefined) {
                callback(...args);
            }
        };

        globalThis.__runRepeatingCallback = function(id, ...args) {
            const callback = callbacks.get(id);

            if (callback !== undefined) {
                callback(...args);
            }
        };
    })();
"#;

/// Setup the registry that keeps pending callbacks reachable from JS
pub fn setup_callback_registry(context: &mut JSContext) {
    context
        .evaluate_script(CALLBACK_REGISTRY_JS, 1)
        .expect("Failed to setup the callback registry");
}

/// Call a function held by the global object
pub(crate) fn call_global(
    context: &JSContext,
    name: &str,
    args: &[JSValue],
) -> Result<JSValue, JSValue> {
    let function = context
        .get_global_object()
        .get_property(context, name)
        .ok_or_else(|| JSValue::string(context, format!("{} is missing", name)))?
        .to_object(context)?;

    function.call_as_function(context, None, args)
}

/// Hand a callback to the JS registry, which owns it until it runs or is dropped
fn store_callback(
    context: &JSContext,
    callback_id: CallbackId,
    callback: JSObject,
) -> Result<(), JSValue> {
    let id = JSValue::number(context, callback_id as f64);

    call_global(context, "__storeCallback", &[id, callback.into()])?;

    Ok(())
}

fn drop_callback(context: &JSContext, callback_id: CallbackId) {
    let id = JSValue::number(context, callback_id as f64);

    if let Err(e) = call_global(context, "__dropCallback", &[id]) {
        log::error!(
            "Failed to drop callback {}: {}",
            callback_id,
            super::error_text(context, &e)
        );
    }
}

/// Setup console bindings (log, info, warn, error, debug)
///
/// Without an ops handle the messages go to stdout, which no runner reads.
pub fn setup_console(context: &mut JSContext, ops: Option<OperationsHandle>) {
    // Create native __console_log function that accepts level and message
    let console_log_fn = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.len() < 2 {
                return Ok(JSValue::undefined(&ctx));
            }

            let level_num = args[0].to_number(&ctx).map(|n| n as i32).unwrap_or(2);
            let msg = args[1]
                .to_js_string(&ctx)
                .map(|s| s.to_string())
                .unwrap_or_default();

            let level = match level_num {
                0 => LogLevel::Error,
                1 => LogLevel::Warn,
                3 => LogLevel::Debug,
                _ => LogLevel::Info,
            };

            // Called straight from JS, or else a script that never yields to the
            // event loop would end before its logs were delivered
            match &ops {
                Some(ops) => ops.handle_log(level, msg),
                None => println!("[{}] {}", level, msg),
            }

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
                __console_log(3, msg);
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

    if let Ok(wrapper) = ctx.evaluate_script(script, 1)
        && let Ok(wrapper_fn) = wrapper.to_object(&ctx)
    {
        let _ = wrapper_fn.call_as_function(&ctx, None, &[callback.into()]);
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
    next_id: CallbackCounter,
) {
    let scheduler_tx_clone = scheduler_tx;
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

            // One callback settles the Promise, and a network error arrives as
            // an Error, which has to reject rather than resolve
            let promise_script = r#"
                new Promise((resolve, reject) => {
                    globalThis.__fetchSettle = (value) =>
                        value instanceof Error ? reject(value) : resolve(value);
                })
            "#;

            let promise = match ctx.evaluate_script(promise_script, 1) {
                Ok(p) => p,
                Err(_) => return Err(JSValue::string(&ctx, "Failed to create Promise")),
            };

            let settle_callback = ctx
                .get_global_object()
                .get_property(&ctx, "__fetchSettle")
                .and_then(|v| v.to_object(&ctx).ok())
                .ok_or_else(|| JSValue::string(&ctx, "Failed to get settle callback"))?;

            let callback_id = next_callback_id(&next_id_clone);

            store_callback(&ctx, callback_id, settle_callback)?;

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
        globalThis.fetch = async function(input, init) {
            let options = { ...(init || {}) };
            const isRequest = input instanceof Request;
            const url = isRequest ? input.url : String(input);

            if (isRequest) {
                options.method = options.method || input.method;
                options.headers = options.headers || input.headers;

                if (options.body === undefined && input.body && !input.bodyUsed) {
                    options.body = input.body;
                }
            }

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
                    options = { ...options, body: combined };
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
    next_id: CallbackCounter,
) {
    // Setup setTimeout
    setup_set_timeout(context, scheduler_tx.clone(), next_id.clone());

    // Setup setInterval
    setup_set_interval(context, scheduler_tx.clone(), next_id);

    // Setup clearTimeout and clearInterval (same implementation)
    setup_clear_timer(context, scheduler_tx);
}

/// Setup setTimeout binding
fn setup_set_timeout(
    context: &mut JSContext,
    scheduler_tx: mpsc::UnboundedSender<SchedulerMessage>,
    next_id: CallbackCounter,
) {
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

            let callback_id = next_callback_id(&next_id_clone);

            store_callback(&ctx, callback_id, callback)?;

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
    next_id: CallbackCounter,
) {
    let next_id_clone = next_id;
    let scheduler_tx_clone = scheduler_tx;

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

            let callback_id = next_callback_id(&next_id_clone);

            store_callback(&ctx, callback_id, callback)?;

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

            // Drop the stored callback, or else it leaks until worker teardown
            drop_callback(&ctx, timer_id);

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

            drop_callback(&ctx, timer_id);

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
    next_id: CallbackCounter,
) {
    // Create __nativeStreamRead(stream_id, resolve_callback)
    // This is called from JS to request the next chunk from a stream
    let scheduler_tx_clone = scheduler_tx.clone();
    let next_id_clone = next_id;

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

            let callback_id = next_callback_id(&next_id_clone);

            store_callback(&ctx, callback_id, callback)?;

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
/// __responseStreamWrite(stream_id, Uint8Array) - writes bytes, false when the buffer is full
/// __responseStreamError(stream_id, message) - hands the reader an error, false when full
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

            // Read bytes from the TypedArray
            let bytes = match super::typed_array_bytes(&ctx, &args[1]) {
                Some(bytes) => bytes,
                None => return Err(JSValue::string(&ctx, "data must be a Uint8Array")),
            };

            // A full buffer is the reader being slow, which the caller can wait out;
            // a closed one has nobody left to write to
            match manager_clone
                .try_write_chunk(stream_id, super::stream_manager::StreamChunk::Data(bytes))
            {
                Ok(()) => Ok(JSValue::boolean(&ctx, true)),
                Err(TryWriteError::Full) => Ok(JSValue::boolean(&ctx, false)),
                Err(TryWriteError::Closed) => Err(JSValue::string(
                    &ctx,
                    format!("response stream {} is closed", stream_id),
                )),
            }
        }
    );

    // __responseStreamError(stream_id, message) -> boolean
    let manager_clone = stream_manager.clone();
    let error_stream = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.len() < 2 {
                return Err(JSValue::string(
                    &ctx,
                    "__responseStreamError requires stream_id and message",
                ));
            }

            let stream_id = match args[0].to_number(&ctx) {
                Ok(id) => id as StreamId,
                Err(_) => return Err(JSValue::string(&ctx, "stream_id must be a number")),
            };

            let message = args[1]
                .to_js_string(&ctx)
                .map(|s| s.to_string())
                .unwrap_or_else(|_| "the guest errored its response stream".to_string());

            // A stream nobody reads any more has nobody to tell, so that counts as told
            match manager_clone.try_write_chunk(
                stream_id,
                super::stream_manager::StreamChunk::Error(message),
            ) {
                Ok(()) | Err(TryWriteError::Closed) => Ok(JSValue::boolean(&ctx, true)),
                Err(TryWriteError::Full) => Ok(JSValue::boolean(&ctx, false)),
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

            // Dropping the writer ends the stream whether or not the buffer has room,
            // and a Done chunk would need a free slot to say the same thing
            manager_clone.finish_stream(stream_id);

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
        .set_property(context, "__responseStreamError", error_stream.into())
        .unwrap();
    global
        .set_property(context, "__responseStreamEnd", end_stream.into())
        .unwrap();
}
