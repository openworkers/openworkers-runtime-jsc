use bytes::Bytes;
use openworkers_core::{HttpMethod, HttpRequest, RequestBody};
use openworkers_jsc::Context;
use openworkers_jsc::Object;
use openworkers_jsc::Value;
use std::collections::HashMap;
use std::str::FromStr;

/// Parse headers from JS options object
pub fn parse_headers_from_js(
    context: &Context,
    headers_val: Value,
) -> Result<HashMap<String, String>, String> {
    let mut headers = HashMap::new();

    // A Headers instance carries its entries in a Map, not in own properties
    let normalize = context
        .global_object()
        .get_property(context, "__normalizeHeaders")
        .ok()
        .and_then(|value| value.to_object(context).ok())
        .ok_or("__normalizeHeaders is missing")?;

    let headers_obj = normalize
        .call(context, None, &[headers_val])
        .map_err(|_| "Headers must be a HeadersInit")?
        .to_object(context)
        .map_err(|_| "Headers must be an object")?;

    let prop_names = headers_obj.property_names(context);

    for prop_name in prop_names {
        if let Ok(value_val) = headers_obj.get_property(context, prop_name.as_str())
            && let Ok(value_str) = value_val.to_string(context)
        {
            headers.insert(prop_name, value_str.to_string());
        }
    }

    Ok(headers)
}

/// Read a property, treating `null` and `undefined` as absent
fn defined_property<'c>(context: &Context, obj: &Object<'c>, name: &str) -> Option<Value<'c>> {
    let value = obj.get_property(context, name).ok()?;

    if value.is_undefined(context) || value.is_null(context) {
        return None;
    }

    Some(value)
}

/// Parse fetch options from JavaScript
pub fn parse_fetch_options(
    context: &Context,
    url: String,
    options_val: Option<Value>,
) -> Result<HttpRequest, String> {
    let mut method = HttpMethod::Get;
    let mut headers = HashMap::new();
    let mut body = RequestBody::None;

    if let Some(options) = options_val {
        let options_obj = options
            .to_object(context)
            .map_err(|_| "Options must be an object")?;

        if let Some(method_val) = defined_property(context, &options_obj, "method")
            && let Ok(method_str) = method_val.to_string(context)
        {
            method = HttpMethod::from_str(&method_str.to_string())
                .map_err(|_| format!("Invalid HTTP method: {}", method_str))?;
        }

        if let Some(headers_val) = defined_property(context, &options_obj, "headers") {
            headers = parse_headers_from_js(context, headers_val)?;
        }

        if let Some(body_val) = defined_property(context, &options_obj, "body") {
            body = match super::typed_array_bytes(context, &body_val) {
                Some(bytes) => RequestBody::Bytes(bytes),
                None => match body_val.to_string(context) {
                    Ok(text) => RequestBody::Bytes(Bytes::from(text.to_string())),
                    Err(_) => RequestBody::None,
                },
            };
        }
    }

    Ok(HttpRequest {
        method,
        url,
        headers,
        body,
    })
}
