mod common;

use common::TestRunner;

#[tokio::test]
async fn test_url_parsing() {
    let mut runner = TestRunner::new();

    let script = r#"
        const url = new URL('https://example.com/path?foo=bar#hash');

        globalThis.result = {
            href: url.href,
            protocol: url.protocol,
            hostname: url.hostname,
            pathname: url.pathname,
            search: url.search,
            hash: url.hash,
            origin: url.origin
        };
    "#;

    runner.execute(script).expect("URL parsing should work");

    let check = r#"globalThis.result"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            if let Ok(obj) = result.to_object(&runner.runtime.context) {
                if let Some(pathname) = obj.get_property(&runner.runtime.context, "pathname") {
                    if let Ok(pathname_str) = pathname.to_js_string(&runner.runtime.context) {
                        assert_eq!(pathname_str.to_string(), "/path");
                    }
                }

                if let Some(search) = obj.get_property(&runner.runtime.context, "search") {
                    if let Ok(search_str) = search.to_js_string(&runner.runtime.context) {
                        assert_eq!(search_str.to_string(), "?foo=bar");
                    }
                }
            }
        }
        Err(_) => panic!("Failed to check URL result"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_url_search_params() {
    let mut runner = TestRunner::new();

    let script = r#"
        const params = new URLSearchParams('foo=bar&baz=qux&name=value');

        globalThis.result = {
            hasFoo: params.has('foo'),
            foo: params.get('foo'),
            baz: params.get('baz'),
            missing: params.get('missing')
        };
    "#;

    runner.execute(script).expect("URLSearchParams should work");

    let check = r#"globalThis.result"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            if let Ok(obj) = result.to_object(&runner.runtime.context) {
                if let Some(has_foo) = obj.get_property(&runner.runtime.context, "hasFoo") {
                    assert!(has_foo.to_bool(&runner.runtime.context));
                }

                if let Some(foo_val) = obj.get_property(&runner.runtime.context, "foo") {
                    if let Ok(foo_str) = foo_val.to_js_string(&runner.runtime.context) {
                        assert_eq!(foo_str.to_string(), "bar");
                    }
                }

                if let Some(missing) = obj.get_property(&runner.runtime.context, "missing") {
                    assert!(missing.is_null(&runner.runtime.context));
                }
            }
        }
        Err(_) => panic!("Failed to check URLSearchParams result"),
    }

    runner.shutdown().await;
}

#[tokio::test]
async fn test_url_in_worker_context() {
    let mut runner = TestRunner::new();

    let script = r#"
        const url = new URL('https://api.example.com/users/123?filter=active');
        globalThis.pathname = url.pathname;
    "#;

    runner.execute(script).expect("URL should work");

    let check = r#"globalThis.pathname"#;
    match runner.runtime.evaluate(check) {
        Ok(result) => {
            if let Ok(pathname) = result.to_js_string(&runner.runtime.context) {
                assert_eq!(pathname.to_string(), "/users/123");
            }
        }
        Err(_) => panic!("Failed to check pathname"),
    }

    runner.shutdown().await;
}
