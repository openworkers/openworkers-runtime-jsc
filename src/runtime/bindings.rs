use rusty_jsc::{JSContext, JSObject, JSValue};
use rusty_jsc_macros::callback;

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
