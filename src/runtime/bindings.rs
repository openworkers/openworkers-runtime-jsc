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
    context.evaluate_script("globalThis.console = {}", 1).unwrap();
    let console_obj = global.get_property(context, "console")
        .unwrap()
        .to_object(context)
        .unwrap();

    let mut console_mut = console_obj;
    console_mut.set_property(context, "log", log_fn).unwrap();
}

/// Setup setTimeout binding
pub fn setup_timer(
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

            log::debug!("setTimeout: registered callback {} with delay {}ms", callback_id, delay);

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
