use super::{FetchRequest, HttpMethod};
use crate::runtime::FetchResponseMeta;
use crate::runtime::stream_manager::{StreamChunk, StreamManager};
use futures_util::StreamExt;
use rusty_jsc::{JSContext, JSValue};
use std::sync::Arc;

/// Parse fetch options from JavaScript
pub fn parse_fetch_options(
    context: &JSContext,
    url: String,
    options_val: Option<JSValue>,
) -> Result<FetchRequest, String> {
    let mut request = FetchRequest {
        url,
        ..Default::default()
    };

    if let Some(options) = options_val {
        let options_obj = options
            .to_object(context)
            .map_err(|_| "Options must be an object")?;

        // Parse method
        if let Some(method_val) = options_obj.get_property(context, "method") {
            if !method_val.is_undefined(context) && !method_val.is_null(context) {
                if let Ok(method_str) = method_val.to_js_string(context) {
                    request.method = HttpMethod::from_str(&method_str.to_string())
                        .ok_or_else(|| format!("Invalid HTTP method: {}", method_str))?;
                }
            }
        }

        // Parse headers
        if let Some(headers_val) = options_obj.get_property(context, "headers") {
            if !headers_val.is_undefined(context) && !headers_val.is_null(context) {
                request.headers = super::headers::parse_headers_from_js(context, headers_val)?;
            }
        }

        // Parse body
        if let Some(body_val) = options_obj.get_property(context, "body") {
            if !body_val.is_null(context) && !body_val.is_undefined(context) {
                if let Ok(body_str) = body_val.to_js_string(context) {
                    request.body = Some(body_str.to_string());
                }
            }
        }
    }

    Ok(request)
}

/// Execute HTTP request using reqwest
pub async fn execute_fetch(request: FetchRequest) -> Result<super::FetchResponse, String> {
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
    for (key, value) in request.headers {
        req_builder = req_builder.header(key, value);
    }

    // Add body if present
    if let Some(body) = request.body {
        req_builder = req_builder.body(body);
    }

    // Execute request
    let response = req_builder
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    // Extract response data
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

    // Get body
    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    Ok(super::FetchResponse {
        status,
        status_text,
        headers,
        body,
    })
}

/// Execute HTTP request with streaming response
/// Returns metadata immediately, body is streamed through StreamManager
pub async fn execute_fetch_streaming(
    request: FetchRequest,
    stream_manager: Arc<StreamManager>,
) -> Result<FetchResponseMeta, String> {
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
    if let Some(body) = &request.body {
        req_builder = req_builder.body(body.clone());
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

    Ok(FetchResponseMeta {
        status,
        status_text,
        headers,
        stream_id,
    })
}
