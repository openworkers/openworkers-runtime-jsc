use bytes::Bytes;
use openworkers_core::{HttpMethod, HttpRequest, RequestBody};
use rusty_jsc::{JSContext, JSObject, JSValue};
use std::collections::HashMap;
use std::str::FromStr;

// ============================================================================
// Headers
// ============================================================================

/// Parse headers from JS options object
pub fn parse_headers_from_js(
    context: &JSContext,
    headers_val: JSValue,
) -> Result<HashMap<String, String>, String> {
    let mut headers = HashMap::new();

    // A Headers instance carries its entries in a Map, not in own properties
    let normalize = context
        .get_global_object()
        .get_property(context, "__normalizeHeaders")
        .and_then(|value| value.to_object(context).ok())
        .ok_or("__normalizeHeaders is missing")?;

    let headers_obj = normalize
        .call_as_function(context, None, &[headers_val])
        .map_err(|_| "Headers must be a HeadersInit")?
        .to_object(context)
        .map_err(|_| "Headers must be an object")?;

    // Get all property names
    let prop_names = headers_obj.get_property_names(context);

    for prop_name in prop_names {
        if let Some(value_val) = headers_obj.get_property(context, prop_name.as_str())
            && let Ok(value_str) = value_val.to_js_string(context)
        {
            headers.insert(prop_name, value_str.to_string());
        }
    }

    Ok(headers)
}

// ============================================================================
// Request
// ============================================================================

/// Read a property, treating `null` and `undefined` as absent
fn defined_property(context: &JSContext, obj: &JSObject, name: &str) -> Option<JSValue> {
    let value = obj.get_property(context, name)?;

    if value.is_undefined(context) || value.is_null(context) {
        return None;
    }

    Some(value)
}

/// Parse fetch options from JavaScript
pub fn parse_fetch_options(
    context: &JSContext,
    url: String,
    options_val: Option<JSValue>,
) -> Result<HttpRequest, String> {
    let mut method = HttpMethod::Get;
    let mut headers = HashMap::new();
    let mut body = RequestBody::None;

    if let Some(options) = options_val {
        let options_obj = options
            .to_object(context)
            .map_err(|_| "Options must be an object")?;

        // Parse method
        if let Some(method_val) = defined_property(context, &options_obj, "method")
            && let Ok(method_str) = method_val.to_js_string(context)
        {
            method = HttpMethod::from_str(&method_str.to_string())
                .map_err(|_| format!("Invalid HTTP method: {}", method_str))?;
        }

        // Parse headers
        if let Some(headers_val) = defined_property(context, &options_obj, "headers") {
            headers = parse_headers_from_js(context, headers_val)?;
        }

        // Parse body
        if let Some(body_val) = defined_property(context, &options_obj, "body")
            && let Ok(body_str) = body_val.to_js_string(context)
        {
            body = RequestBody::Bytes(Bytes::from(body_str.to_string()));
        }
    }

    Ok(HttpRequest {
        method,
        url,
        headers,
        body,
    })
}
