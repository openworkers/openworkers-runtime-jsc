use openworkers_core::{Event, HttpMethod, HttpRequest, RequestBody, Script};
use openworkers_runtime_jsc::Worker;
use std::collections::HashMap;

async fn post(script: &str, content_type: &str, body: &str) -> String {
    let mut worker = Worker::new(Script::new(script), None)
        .await
        .expect("Worker should initialize");

    let request = HttpRequest {
        method: HttpMethod::Post,
        url: "https://example.com/".to_string(),
        headers: HashMap::from([("content-type".to_string(), content_type.to_string())]),
        body: RequestBody::Bytes(body.to_string().into_bytes().into()),
    };

    let (task, rx) = Event::fetch(request);
    worker.exec(task).await.expect("Task should execute");

    let response = rx.await.expect("Should receive response");
    let body = response.body.collect().await.expect("Should have body");

    String::from_utf8_lossy(&body).to_string()
}

/// Answers with whatever the handler resolves to, or the error it threw
const ECHO: &str = r#"
    addEventListener('fetch', (event) => {
        event.respondWith((async () => {
            try {
                return new Response(await handle(event.request));
            } catch (e) {
                return new Response(`${e.constructor.name}: ${e.message}`);
            }
        })());
    });
"#;

#[tokio::test]
async fn test_form_data_decodes_urlencoded_body() {
    let script = format!(
        r#"
        async function handle(request) {{
            const form = await request.formData();

            return JSON.stringify([form.get('name'), form.get('note')]);
        }}
        {ECHO}
    "#
    );

    assert_eq!(
        post(
            &script,
            "application/x-www-form-urlencoded",
            "name=Jo+Ann&note=a%20%26%20b"
        )
        .await,
        r#"["Jo Ann","a & b"]"#
    );
}

#[tokio::test]
async fn test_form_data_keeps_repeated_keys() {
    let script = format!(
        r#"
        async function handle(request) {{
            const form = await request.formData();

            return JSON.stringify([form.getAll('tag'), form.get('tag'), form.has('missing')]);
        }}
        {ECHO}
    "#
    );

    assert_eq!(
        post(&script, "application/x-www-form-urlencoded", "tag=a&tag=b").await,
        r#"[["a","b"],"a",false]"#
    );
}

#[tokio::test]
async fn test_form_data_set_replaces_every_value() {
    let script = format!(
        r#"
        async function handle(request) {{
            const form = await request.formData();
            form.set('tag', 'c');
            form.delete('name');

            return JSON.stringify([...form]);
        }}
        {ECHO}
    "#
    );

    assert_eq!(
        post(
            &script,
            "application/x-www-form-urlencoded",
            "tag=a&name=Jo&tag=b"
        )
        .await,
        r#"[["tag","c"]]"#
    );
}

#[tokio::test]
async fn test_form_data_rejects_a_body_it_cannot_decode() {
    let script = format!(
        r#"
        async function handle(request) {{
            return await request.formData();
        }}
        {ECHO}
    "#
    );

    assert_eq!(
        post(&script, "application/json", "{}").await,
        "TypeError: Cannot decode a application/json body as FormData"
    );
}
