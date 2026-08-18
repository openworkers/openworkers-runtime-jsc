mod common;

use common::TestRunner;

/// Run a script that leaves its outcome in globalThis.result
fn eval_result(runner: &mut TestRunner, script: &str) -> String {
    runner.execute(script).expect("script should run");

    let value = runner
        .runtime
        .evaluate("String(globalThis.result)")
        .expect("result should be readable");

    value
        .to_js_string(&runner.runtime.context)
        .expect("result should be a string")
        .to_string()
}

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
                if let Some(pathname) = obj.get_property(&runner.runtime.context, "pathname")
                    && let Ok(pathname_str) = pathname.to_js_string(&runner.runtime.context)
                {
                    assert_eq!(pathname_str.to_string(), "/path");
                }

                if let Some(search) = obj.get_property(&runner.runtime.context, "search")
                    && let Ok(search_str) = search.to_js_string(&runner.runtime.context)
                {
                    assert_eq!(search_str.to_string(), "?foo=bar");
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

                if let Some(foo_val) = obj.get_property(&runner.runtime.context, "foo")
                    && let Ok(foo_str) = foo_val.to_js_string(&runner.runtime.context)
                {
                    assert_eq!(foo_str.to_string(), "bar");
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

#[tokio::test]
async fn test_url_from_url_instance() {
    let mut runner = TestRunner::new();

    let result = eval_result(
        &mut runner,
        r#"globalThis.result = new URL(new URL('https://example.com/a/b?x=1#f')).href;"#,
    );

    assert_eq!(result, "https://example.com/a/b?x=1#f");

    runner.shutdown().await;
}

#[tokio::test]
async fn test_url_resolves_against_base() {
    let mut runner = TestRunner::new();

    let result = eval_result(
        &mut runner,
        r#"globalThis.result = [
            new URL('../c', 'https://example.com/a/b/d').href,
            new URL('/root', 'https://example.com/a/b').href,
            new URL('https://other.test/x', 'https://example.com/a').href
        ].join(' ');"#,
    );

    assert_eq!(
        result,
        "https://example.com/a/c https://example.com/root https://other.test/x"
    );

    runner.shutdown().await;
}

#[tokio::test]
async fn test_url_setters_update_href() {
    let mut runner = TestRunner::new();

    let result = eval_result(
        &mut runner,
        r#"
            const url = new URL('https://example.com/a?x=1#f');
            url.pathname = '/b/c';
            url.search = '?y=2';
            url.hash = 'top';
            url.port = '8443';
            globalThis.result = url.href + ' | ' + url.host + ' | ' + url.origin;
        "#,
    );

    assert_eq!(
        result,
        "https://example.com:8443/b/c?y=2#top | example.com:8443 | https://example.com:8443"
    );

    runner.shutdown().await;
}

#[tokio::test]
async fn test_url_rejects_invalid_input() {
    let mut runner = TestRunner::new();

    let result = eval_result(
        &mut runner,
        r#"
            try {
                new URL('not a url');
                globalThis.result = 'no throw';
            } catch (e) {
                globalThis.result = e.constructor.name;
            }
        "#,
    );

    assert_eq!(result, "TypeError");

    runner.shutdown().await;
}

#[tokio::test]
async fn test_search_params_write_back_to_url() {
    let mut runner = TestRunner::new();

    let result = eval_result(
        &mut runner,
        r#"
            const url = new URL('https://example.com/?a=1');
            url.searchParams.set('b', '2');
            url.searchParams.append('b', '3');
            url.searchParams.delete('a');
            globalThis.result = url.href + ' | ' + url.search + ' | ' + url.searchParams.size;
        "#,
    );

    assert_eq!(result, "https://example.com/?b=2&b=3 | ?b=2&b=3 | 2");

    runner.shutdown().await;
}

#[tokio::test]
async fn test_search_params_api() {
    let mut runner = TestRunner::new();

    let result = eval_result(
        &mut runner,
        r#"
            const params = new URLSearchParams('b=2&a=1&b=3');
            const all = params.getAll('b').join(',');
            params.sort();
            const encoded = new URLSearchParams({ 'a b': 'c+d~e' }).toString();
            const entries = [...new URLSearchParams([['x', '1'], ['y', '2']])]
                .map(([k, v]) => k + '=' + v)
                .join('&');
            globalThis.result = [all, params.toString(), encoded, entries].join(' | ');
        "#,
    );

    assert_eq!(result, "2,3 | a=1&b=2&b=3 | a+b=c%2Bd%7Ee | x=1&y=2");

    runner.shutdown().await;
}

#[tokio::test]
async fn test_url_search_params_decodes_plus_and_percent() {
    let mut runner = TestRunner::new();

    let result = eval_result(
        &mut runner,
        r#"
            const params = new URLSearchParams('q=a+b%20c&empty&raw=%zz');
            globalThis.result = [
                params.get('q'),
                params.get('empty'),
                params.get('raw'),
                params.has('missing')
            ].join(' | ');
        "#,
    );

    assert_eq!(result, "a b c |  | %zz | false");

    runner.shutdown().await;
}

#[tokio::test]
async fn test_host_setter_keeps_an_ipv6_literal() {
    let mut runner = TestRunner::new();

    let script = r#"
        const url = new URL('http://example.com/path');
        url.host = '[::1]:8080';

        globalThis.result = [url.hostname, url.port, url.href].join(' ');
    "#;

    assert_eq!(
        eval_result(&mut runner, script),
        "[::1] 8080 http://[::1]:8080/path"
    );

    runner.shutdown().await;
}
