use rusty_jsc::{JSContext, JSValue};
use std::collections::HashMap;

/// Create a Headers object in JavaScript from a HashMap
pub fn create_headers_object(
    context: &mut JSContext,
    headers: &HashMap<String, String>,
) -> Result<JSValue, String> {
    // Create headers via JS to avoid type annotation issues
    context
        .evaluate_script("({})", 1)
        .map_err(|_| "Failed to create headers object".to_string())?;

    let headers_obj = context
        .evaluate_script("({})", 1)
        .map_err(|_| "Failed to create headers object".to_string())?
        .to_object(context)
        .map_err(|_| "Failed to convert to object".to_string())?;

    // Add get, has, and forEach methods
    let headers_data = headers.clone();

    // Store headers as a JS object for easy access
    for (key, value) in headers {
        let value_js = JSValue::string(context, value.as_str());
        let mut headers_mut = headers_obj.clone();
        headers_mut
            .set_property(context, key.as_str(), value_js)
            .map_err(|_| "Failed to set header property")?;
    }

    // Add get method
    let headers_data_get = headers_data.clone();
    let get_fn = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.is_empty() {
                return Ok(JSValue::null(&ctx));
            }

            let key = match args[0].to_js_string(&ctx) {
                Ok(s) => s.to_string().to_lowercase(),
                Err(_) => return Ok(JSValue::null(&ctx)),
            };

            // Case-insensitive lookup
            for (k, v) in &headers_data_get {
                if k.to_lowercase() == key {
                    return Ok(JSValue::string(&ctx, v.as_str()));
                }
            }

            Ok(JSValue::null(&ctx))
        }
    );

    // Add has method
    let headers_data_has = headers_data.clone();
    let has_fn = rusty_jsc::callback_closure!(
        context,
        move |ctx: JSContext, _func: JSObject, _this: JSObject, args: &[JSValue]| {
            if args.is_empty() {
                return Ok(JSValue::boolean(&ctx, false));
            }

            let key = match args[0].to_js_string(&ctx) {
                Ok(s) => s.to_string().to_lowercase(),
                Err(_) => return Ok(JSValue::boolean(&ctx, false)),
            };

            // Case-insensitive lookup
            let has = headers_data_has.keys().any(|k| k.to_lowercase() == key);

            Ok(JSValue::boolean(&ctx, has))
        }
    );

    let mut headers_mut = headers_obj.clone();
    headers_mut
        .set_property(context, "get", get_fn.into())
        .map_err(|_| "Failed to set get method")?;
    headers_mut
        .set_property(context, "has", has_fn.into())
        .map_err(|_| "Failed to set has method")?;

    Ok(headers_obj.into())
}

/// Parse headers from JS options object
pub fn parse_headers_from_js(
    context: &JSContext,
    headers_val: JSValue,
) -> Result<HashMap<String, String>, String> {
    let mut headers = HashMap::new();

    let headers_obj = headers_val
        .to_object(context)
        .map_err(|_| "Headers must be an object")?;

    // Get all property names
    let prop_names = headers_obj.get_property_names(context);

    for prop_name in prop_names {
        if let Some(value_val) = headers_obj.get_property(context, prop_name.as_str()) {
            if let Ok(value_str) = value_val.to_js_string(context) {
                headers.insert(prop_name, value_str.to_string());
            }
        }
    }

    Ok(headers)
}
