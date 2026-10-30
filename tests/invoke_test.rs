//! env.<NAME>.invoke(command, args) reaches the host handler of the invoke
//! binding, and its answer settles the promise.

#![cfg(feature = "invoke")]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use openworkers_core::{
    BindingInfo, Event, HttpMethod, HttpRequest, InvokeResult, OpFuture, OperationsHandle,
    OperationsHandler, RequestBody, Script,
};
use openworkers_runtime_jsc::Worker;
use serde_json::{Value, json};

type Call = (String, String, Value);

/// Answers two functions and records every call it gets.
#[derive(Default)]
struct Host {
    calls: Mutex<Vec<Call>>,
}

impl OperationsHandler for Host {
    fn handle_binding_invoke(
        &self,
        binding: &str,
        name: &str,
        args: Value,
    ) -> OpFuture<'_, InvokeResult> {
        self.calls
            .lock()
            .unwrap()
            .push((binding.to_string(), name.to_string(), args.clone()));

        let answer = match name {
            "echo" => InvokeResult::Value(json!({ "binding": binding, "args": args })),
            "fail" => InvokeResult::Error("the disk is full".to_string()),
            other => InvokeResult::Error(format!("no function named '{other}'")),
        };

        Box::pin(async move { answer })
    }
}

/// Runs `body`, the inside of an async function that returns a string, in a
/// fetch handler with one invoke binding named RUST and a variable RUST that
/// the binding must win over.
async fn run(body: &str) -> (String, Vec<Call>) {
    let code = format!(
        "addEventListener('fetch', (event) => {{
            event.respondWith((async () => {{ {body} }})().then((text) => new Response(String(text))));
        }});"
    );

    let host = Arc::new(Host::default());
    let ops: OperationsHandle = host.clone();
    let script = Script {
        code: code.into(),
        env: Some(HashMap::from([(
            "RUST".to_string(),
            "a variable the binding replaces".to_string(),
        )])),
        bindings: vec![BindingInfo::invoke("RUST")],
    };

    let mut worker = Worker::new_with_ops(script, None, ops)
        .await
        .expect("the worker initialises");
    let (task, rx) = Event::fetch(HttpRequest {
        method: HttpMethod::Get,
        url: "http://localhost/".to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    });

    worker.exec(task).await.expect("the task executes");

    let response = rx.await.expect("a response");
    let body = response
        .body
        .collect()
        .await
        .expect("the body does not error")
        .unwrap_or_default();
    let calls = host.calls.lock().unwrap().clone();

    (String::from_utf8_lossy(&body).into_owned(), calls)
}

#[tokio::test]
async fn invoke_returns_what_the_host_answers() {
    let (body, calls) =
        run("return JSON.stringify(await env.RUST.invoke('echo', { a: 2 }));").await;

    assert_eq!(body, r#"{"args":{"a":2},"binding":"RUST"}"#);
    assert_eq!(
        calls,
        vec![("RUST".to_string(), "echo".to_string(), json!({ "a": 2 }))]
    );
}

#[tokio::test]
async fn the_binding_wins_over_a_variable_of_the_same_name() {
    let (body, _) = run("return typeof env.RUST.invoke;").await;

    assert_eq!(body, "function");
}

#[tokio::test]
async fn invoke_keeps_arguments_that_are_falsy() {
    let (body, _) = run("const answers = [];
         for (const value of [0, false, '', null]) {
             answers.push((await env.RUST.invoke('echo', value)).args);
         }
         return JSON.stringify(answers);")
    .await;

    assert_eq!(body, r#"[0,false,"",null]"#);
}

#[tokio::test]
async fn invoke_without_arguments_sends_null() {
    let (body, calls) = run("return JSON.stringify((await env.RUST.invoke('echo')).args);").await;

    assert_eq!(body, "null");
    assert_eq!(calls[0].2, Value::Null);
}

#[tokio::test]
async fn an_error_rejects_with_its_message() {
    let (body, _) =
        run("try { await env.RUST.invoke('fail'); } catch (error) { return error.message; }").await;

    assert_eq!(body, "the disk is full");
}

#[tokio::test]
async fn a_command_that_is_not_a_string_is_a_type_error() {
    let (body, calls) = run(
        "try { await env.RUST.invoke(42); } catch (error) { return error.name + ': ' + error.message; }",
    )
    .await;

    assert_eq!(body, "TypeError: invoke: the command is not a string");
    assert!(calls.is_empty());
}

#[tokio::test]
async fn arguments_that_cannot_be_json_are_a_type_error() {
    let (body, calls) = run("const names = [];
         const circular = {}; circular.self = circular;
         for (const value of [circular, { a: 1n }]) {
             try { await env.RUST.invoke('echo', value); names.push('sent'); }
             catch (error) { names.push(error.name); }
         }
         return names.join(',');")
    .await;

    assert_eq!(body, "TypeError,TypeError");
    assert!(calls.is_empty());
}

#[tokio::test]
async fn two_calls_in_flight_both_settle() {
    let (body, calls) = run(
        "const [a, b] = await Promise.all([env.RUST.invoke('echo', 1), env.RUST.invoke('echo', 2)]);
         return a.args + ',' + b.args;",
    )
    .await;

    assert_eq!(body, "1,2");
    assert_eq!(calls.len(), 2);
}

#[test]
fn the_runtime_says_it_serves_invoke() {
    assert!(
        openworkers_runtime_jsc::SUPPORTED_BINDINGS
            .contains(&openworkers_core::BindingType::Invoke)
    );
}
