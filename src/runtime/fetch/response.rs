use super::FetchResponse;
use rusty_jsc::{JSContext, JSValue};

/// Create a Response object in JavaScript from FetchResponse
pub fn create_response_object(
    context: &mut JSContext,
    response: FetchResponse,
) -> Result<JSValue, String> {
    // Escape the body for safe JavaScript string literal
    let body_escaped = escape_js_string(&response.body);

    // Create headers object
    let headers_obj = super::headers::create_headers_object(context, &response.headers)
        .map_err(|e| format!("Failed to create headers: {}", e))?;

    // Build Response object with methods
    let response_script = format!(
        r#"(function() {{
            const _body = `{}`;
            const _headers = arguments[0];

            return {{
                status: {},
                statusText: "{}",
                ok: {},
                headers: _headers,
                text: function() {{
                    return Promise.resolve(_body);
                }},
                json: function() {{
                    return Promise.resolve(JSON.parse(_body));
                }},
                _bodyUsed: false,
            }};
        }})"#,
        body_escaped,
        response.status,
        response.status_text,
        response.ok()
    );

    // Evaluate to get the constructor function
    let constructor = context
        .evaluate_script(&response_script, 1)
        .map_err(|_| "Failed to create Response constructor")?;

    // Call it with headers as argument
    let constructor_fn = constructor
        .to_object(context)
        .map_err(|_| "Response constructor is not a function")?;

    let response_obj = constructor_fn
        .call_as_function(context, None, &[headers_obj])
        .map_err(|_| "Failed to call Response constructor")?;

    Ok(response_obj)
}

/// Escape a string for safe inclusion in JavaScript template literal
fn escape_js_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace('$', "\\$")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_js_string() {
        assert_eq!(escape_js_string("hello"), "hello");
        assert_eq!(escape_js_string("line1\nline2"), "line1\\nline2");
        assert_eq!(escape_js_string("tab\there"), "tab\\there");
        assert_eq!(escape_js_string("`template`"), "\\`template\\`");
        assert_eq!(escape_js_string("${var}"), "\\${var}");
    }
}
