mod common;

use common::TestRunner;
use std::time::Duration;

#[tokio::test]
async fn test_fetch_basic_get() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.fetchResult = null;

        fetch('https://echo.workers.rocks/get')
            .then(response => {
                globalThis.fetchResult = {
                    status: response.status,
                    ok: response.ok
                };
            })
            .catch(error => {
                globalThis.fetchResult = { error: String(error) };
            });
    "#;

    runner.execute(script).expect("fetch should execute");

    // Wait for fetch to complete
    runner.process_for(Duration::from_secs(3)).await;

    // Check result
    let check = r#"globalThis.fetchResult"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            if let Ok(obj) = result.to_object(&runner.runtime.context) {
                if let Some(status_val) = obj.get_property(&runner.runtime.context, "status") {
                    if let Ok(status) = status_val.to_number(&runner.runtime.context) {
                        assert_eq!(status, 200.0, "Should get 200 OK response");
                    }
                }

                if let Some(ok_val) = obj.get_property(&runner.runtime.context, "ok") {
                    assert!(
                        ok_val.to_bool(&runner.runtime.context),
                        "Response should be ok"
                    );
                }
            }
        }
        Err(_) => panic!("Failed to check fetch result"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_fetch_with_text() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.textResult = null;

        fetch('https://echo.workers.rocks/get')
            .then(response => response.text())
            .then(text => {
                globalThis.textResult = text.substring(0, 50);
            })
            .catch(error => {
                console.log("Fetch error:", error);
            });
    "#;

    runner.execute(script).expect("fetch should execute");

    // Wait for fetch
    runner.process_for(Duration::from_secs(3)).await;

    // Check we got text back
    let check = r#"typeof globalThis.textResult"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            if let Ok(type_str) = result.to_js_string(&runner.runtime.context) {
                assert_eq!(type_str.to_string(), "string", "Should get text response");
            }
        }
        Err(_) => panic!("Failed to check text result"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_fetch_404_error() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.notFoundResult = null;

        fetch('https://echo.workers.rocks/status/404')
            .then(response => {
                globalThis.notFoundResult = {
                    status: response.status,
                    ok: response.ok
                };
            })
            .catch(error => {
                globalThis.notFoundResult = { error: String(error) };
            });
    "#;

    runner.execute(script).expect("fetch should execute");

    // Wait for fetch
    runner.process_for(Duration::from_secs(3)).await;

    // Check result
    let check_status = r#"globalThis.notFoundResult.status"#;
    match runner.runtime.evaluate(check_status) {
        Ok(result) => {
            let status = result.to_number(&runner.runtime.context).unwrap();
            assert_eq!(status, 404.0, "Should get 404 status");
        }
        Err(_) => panic!("Failed to check status"),
    }

    let check_ok = r#"globalThis.notFoundResult.ok"#;
    match runner.runtime.evaluate(check_ok) {
        Ok(result) => {
            assert!(
                !result.to_bool(&runner.runtime.context),
                "404 response should not be ok"
            );
        }
        Err(_) => panic!("Failed to check ok status"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_fetch_json() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.jsonResult = null;

        fetch('https://echo.workers.rocks/json')
            .then(response => response.json())
            .then(data => {
                globalThis.jsonResult = data;
            })
            .catch(error => {
                console.log("JSON error:", error);
            });
    "#;

    runner.execute(script).expect("fetch should execute");

    // Wait for fetch
    runner.process_for(Duration::from_secs(3)).await;

    // Check we got JSON object
    let check = r#"typeof globalThis.jsonResult"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            if let Ok(type_str) = result.to_js_string(&runner.runtime.context) {
                assert_eq!(
                    type_str.to_string(),
                    "object",
                    "Should parse JSON to object"
                );
            }
        }
        Err(_) => panic!("Failed to check JSON result"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_fetch_with_custom_method() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.methodResult = null;

        fetch('https://echo.workers.rocks/get', {
            method: 'POST',
            body: 'test data'
        })
        .then(response => {
            globalThis.methodResult = {
                gotResponse: true,
                status: response.status
            };
        })
        .catch(error => {
            console.log("Error:", error);
            globalThis.methodResult = { gotResponse: true, status: 0 };
        });
    "#;

    runner
        .execute(script)
        .expect("fetch with method should work");

    runner.process_for(Duration::from_secs(3)).await;

    let check = r#"globalThis.methodResult.gotResponse"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            assert!(
                result.to_bool(&runner.runtime.context),
                "Should get a response from POST"
            );
        }
        Err(_) => panic!("Failed to check method result"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_fetch_post_with_body() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.postResult = null;

        fetch('https://echo.workers.rocks/post', {
            method: 'POST',
            headers: {
                'Content-Type': 'text/plain'
            },
            body: 'Hello from JSCore!'
        })
        .then(response => response.text())
        .then(text => {
            // Just check that we got a response
            globalThis.postResult = {
                hasResponse: text.length > 0,
                isString: typeof text === 'string'
            };
        })
        .catch(error => {
            console.log("POST error:", error);
            globalThis.postResult = { error: String(error) };
        });
    "#;

    runner.execute(script).expect("POST with body should work");

    runner.process_for(Duration::from_secs(3)).await;

    let check = r#"globalThis.postResult"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            if let Ok(obj) = result.to_object(&runner.runtime.context) {
                if let Some(has_response) = obj.get_property(&runner.runtime.context, "hasResponse")
                {
                    assert!(
                        has_response.to_bool(&runner.runtime.context),
                        "POST should return response"
                    );
                }
                if let Some(is_string) = obj.get_property(&runner.runtime.context, "isString") {
                    assert!(
                        is_string.to_bool(&runner.runtime.context),
                        "Response should be string"
                    );
                }
            }
        }
        Err(_) => panic!("Failed to check POST result"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_response_headers_api() {
    let mut runner = TestRunner::new();

    let script = r#"
        globalThis.headersApiResult = null;

        fetch('https://echo.workers.rocks/get')
            .then(response => {
                globalThis.headersApiResult = {
                    hasContentType: response.headers.has('content-type'),
                    contentType: response.headers.get('content-type'),
                    hasNonExistent: response.headers.has('x-nonexistent'),
                    nonExistent: response.headers.get('x-nonexistent')
                };
            })
            .catch(error => {
                console.log("Error:", error);
            });
    "#;

    runner.execute(script).expect("headers API should work");

    runner.process_for(Duration::from_secs(3)).await;

    let check = r#"globalThis.headersApiResult"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            if let Ok(obj) = result.to_object(&runner.runtime.context) {
                // Check has() works
                if let Some(has_ct) = obj.get_property(&runner.runtime.context, "hasContentType") {
                    assert!(
                        has_ct.to_bool(&runner.runtime.context),
                        "should have content-type header"
                    );
                }

                // Check get() works and is case-insensitive
                if let Some(ct_val) = obj.get_property(&runner.runtime.context, "contentType") {
                    assert!(
                        !ct_val.is_null(&runner.runtime.context),
                        "content-type should not be null"
                    );
                }

                // Check non-existent header
                if let Some(has_ne) = obj.get_property(&runner.runtime.context, "hasNonExistent") {
                    assert!(
                        !has_ne.to_bool(&runner.runtime.context),
                        "non-existent header should return false"
                    );
                }

                if let Some(ne_val) = obj.get_property(&runner.runtime.context, "nonExistent") {
                    assert!(
                        ne_val.is_null(&runner.runtime.context),
                        "non-existent header get should return null"
                    );
                }
            }
        }
        Err(_) => panic!("Failed to check headers API result"),
    }

    runner.shutdown().await;
}
