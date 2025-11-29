use crate::runtime::stream_manager::{StreamChunk, StreamId, StreamManager};
use bytes::Bytes;
use futures_util::StreamExt;
use openworkers_core::{HttpMethod, HttpRequest, HttpResponseMeta, RequestBody};
use rusty_jsc::{JSContext, JSValue};
use std::collections::HashMap;
use std::sync::Arc;

// ============================================================================
// Headers
// ============================================================================

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

// ============================================================================
// Request
// ============================================================================

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
        if let Some(method_val) = options_obj.get_property(context, "method") {
            if !method_val.is_undefined(context) && !method_val.is_null(context) {
                if let Ok(method_str) = method_val.to_js_string(context) {
                    method = HttpMethod::from_str(&method_str.to_string())
                        .ok_or_else(|| format!("Invalid HTTP method: {}", method_str))?;
                }
            }
        }

        // Parse headers
        if let Some(headers_val) = options_obj.get_property(context, "headers") {
            if !headers_val.is_undefined(context) && !headers_val.is_null(context) {
                headers = parse_headers_from_js(context, headers_val)?;
            }
        }

        // Parse body
        if let Some(body_val) = options_obj.get_property(context, "body") {
            if !body_val.is_null(context) && !body_val.is_undefined(context) {
                if let Ok(body_str) = body_val.to_js_string(context) {
                    body = RequestBody::Bytes(Bytes::from(body_str.to_string()));
                }
            }
        }
    }

    Ok(HttpRequest {
        method,
        url,
        headers,
        body,
    })
}

/// Execute HTTP request with streaming response
/// Returns metadata and stream ID immediately, body is streamed through StreamManager
pub async fn execute_fetch_streaming(
    request: HttpRequest,
    stream_manager: Arc<StreamManager>,
) -> Result<(HttpResponseMeta, StreamId), String> {
    let client = reqwest::Client::new();

    // Build the request
    let mut req_builder = match request.method {
        HttpMethod::Get => client.get(&request.url),
        HttpMethod::Post => client.post(&request.url),
        HttpMethod::Put => client.put(&request.url),
        HttpMethod::Delete => client.delete(&request.url),
        HttpMethod::Patch => client.patch(&request.url),
        HttpMethod::Head => client.head(&request.url),
        HttpMethod::Options => {
            return Err("OPTIONS method not yet supported".to_string());
        }
    };

    // Add headers
    for (key, value) in &request.headers {
        req_builder = req_builder.header(key, value);
    }

    // Add body if present
    match request.body {
        RequestBody::Bytes(ref bytes) => {
            req_builder = req_builder.body(bytes.clone());
        }
        RequestBody::None => {}
    }

    // Execute request
    let response = req_builder
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    // Extract response metadata
    let status = response.status().as_u16();
    let status_text = response
        .status()
        .canonical_reason()
        .unwrap_or("")
        .to_string();

    // Extract headers
    let mut headers = std::collections::HashMap::new();
    for (key, value) in response.headers() {
        if let Ok(value_str) = value.to_str() {
            headers.insert(key.to_string(), value_str.to_string());
        }
    }

    // Create stream for body
    let stream_id = stream_manager.create_stream(request.url.clone());

    // Spawn task to stream body chunks to StreamManager
    let manager = stream_manager.clone();
    tokio::spawn(async move {
        let mut byte_stream = response.bytes_stream();

        while let Some(chunk_result) = byte_stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    if let Err(e) = manager
                        .write_chunk(stream_id, StreamChunk::Data(chunk))
                        .await
                    {
                        log::error!("Failed to write stream chunk: {}", e);
                        let _ = manager.write_chunk(stream_id, StreamChunk::Error(e)).await;
                        return;
                    }
                }
                Err(e) => {
                    log::error!("Stream read error: {}", e);
                    let _ = manager
                        .write_chunk(stream_id, StreamChunk::Error(e.to_string()))
                        .await;
                    return;
                }
            }
        }

        // Stream completed successfully
        if let Err(e) = manager.write_chunk(stream_id, StreamChunk::Done).await {
            log::error!("Failed to write stream done: {}", e);
        }
    });

    Ok((
        HttpResponseMeta {
            status,
            status_text,
            headers,
        },
        stream_id,
    ))
}
