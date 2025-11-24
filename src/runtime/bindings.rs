use super::{CallbackId, SchedulerMessage};
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

#[callback]
fn console_log(
    ctx: JSContext,
    _function: JSObject,
    _this: JSObject,
    args: &[JSValue],
) -> Result<JSValue, JSValue> {
    let messages: Vec<String> = args
        .iter()
        .map(|arg| {
            arg.to_js_string(&ctx)
                .map(|s| s.to_string())
                .unwrap_or_else(|_| "[value]".to_string())
        })
        .collect();

    println!("[JS] {}", messages.join(" "));
    Ok(JSValue::undefined(&ctx))
}

/// Setup console.log binding
pub fn setup_console(context: &mut JSContext) {
    let global = context.get_global_object();

    // Create console.log function
    let log_fn = JSValue::callback(context, Some(console_log));

    // Create console object via JS and add log method
    context
        .evaluate_script("globalThis.console = {}", 1)
        .unwrap();
    let console_obj = global
        .get_property(context, "console")
        .unwrap()
        .to_object(context)
        .unwrap();

    let mut console_mut = console_obj;
    console_mut.set_property(context, "log", log_fn).unwrap();
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
