// cargo run --release --example conformance
//
// Runs the SvelteKit conformance suite and diffs against the recorded oracle.
// CONFORMANCE_DIR points at the fixture, CONFORMANCE_DUMP writes the bodies we
// produced next to their oracle counterparts.

use openworkers_core::Event;
use openworkers_core::HttpMethod;
use openworkers_core::HttpRequest;
use openworkers_core::HttpResponse;
use openworkers_core::LogLevel;
use openworkers_core::OpFuture;
use openworkers_core::OperationsHandler;
use openworkers_core::RequestBody;
use openworkers_core::ResponseBody;
use openworkers_core::Script;
use openworkers_runtime_jsc::Worker;
use openworkers_transform::CodeLanguage;
use openworkers_transform::parse_worker_code;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Deserialize)]
struct Spec {
    bundle: String,
    base_url: String,
    scenarios: Vec<Scenario>,
}

#[derive(Deserialize)]
struct Scenario {
    name: String,
    request: RequestSpec,
}

#[derive(Deserialize)]
struct RequestSpec {
    method: String,
    path: String,
    headers: Vec<String>,
    body: Option<String>,
}

#[derive(Deserialize)]
struct Oracle {
    bundle: Artifact,
    lowered: Lowered,
    scenarios: Vec<Recorded>,
}

#[derive(Deserialize)]
struct Artifact {
    bytes: usize,
    sha256: String,
}

#[derive(Deserialize)]
struct Lowered {
    bytes: usize,
    sha256: String,
}

#[derive(Deserialize)]
struct Recorded {
    name: String,
    response: RecordedResponse,
}

#[derive(Deserialize)]
struct RecordedResponse {
    status: u16,
    headers: Vec<String>,
    body_file: String,
    body_sha256: String,
    warm_identical: bool,
}

/// Every binding call 404s: the fixture runs without static assets
struct Ops;

impl OperationsHandler for Ops {
    fn handle_binding_fetch(
        &self,
        _binding: &str,
        _request: HttpRequest,
    ) -> OpFuture<'_, Result<HttpResponse, String>> {
        Box::pin(async {
            Ok(HttpResponse {
                status: 404,
                headers: Vec::new(),
                body: ResponseBody::None,
            })
        })
    }

    fn handle_log(&self, level: LogLevel, message: String) {
        println!("  [{:?}] {}", level, message);
    }
}

/// This runtime dispatches to addEventListener, so bridge the module handler
/// onto it, with the ASSETS binding the fixture expects
const ADAPTER: &str = r#"
globalThis.__env = {
    ASSETS: { fetch: () => new Response(null, { status: 404 }) }
};
addEventListener('fetch', (event) => {
    event.respondWith(
        globalThis.default.fetch(event.request, globalThis.__env, { waitUntil() {} })
    );
});
"#;

fn sha256(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

struct Answer {
    status: u16,
    headers: Vec<String>,
    body: Vec<u8>,
}

async fn spawn(source: &str) -> Worker {
    Worker::new_with_ops(Script::new(source), None, std::sync::Arc::new(Ops))
        .await
        .expect("worker creation failed")
}

async fn dispatch(worker: &mut Worker, base_url: &str, spec: &RequestSpec) -> Answer {
    let headers: HashMap<String, String> = spec
        .headers
        .iter()
        .map(|h| {
            let (name, value) = h.split_once(": ").expect("header must be `name: value`");
            (name.to_string(), value.to_string())
        })
        .collect();

    let request = HttpRequest {
        method: spec.method.parse::<HttpMethod>().expect("bad method"),
        url: format!("{}{}", base_url, spec.path),
        headers,
        body: match &spec.body {
            Some(b) => RequestBody::Bytes(b.clone().into_bytes().into()),
            None => RequestBody::None,
        },
    };

    let (task, rx) = Event::fetch(request);
    worker.exec(task).await.expect("exec failed");

    let response = rx.await.expect("no response");
    let body = response
        .body
        .collect()
        .await
        .expect("the response body errored")
        .unwrap_or_default();

    Answer {
        status: response.status,
        headers: response
            .headers
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect(),
        body: body.to_vec(),
    }
}

/// Byte offset of the first difference, with the surrounding text of both sides
fn body_divergence(expected: &[u8], actual: &[u8]) -> String {
    let at = expected
        .iter()
        .zip(actual)
        .position(|(a, b)| a != b)
        .unwrap_or(expected.len().min(actual.len()));

    let from = at.saturating_sub(60);
    let window = |bytes: &[u8]| {
        let end = (at + 80).min(bytes.len());
        String::from_utf8_lossy(&bytes[from.min(bytes.len())..end]).to_string()
    };

    format!(
        "     first divergence at byte {} ({} vs {} bytes)\n       oracle: ...{}...\n       ours:   ...{}...",
        at,
        expected.len(),
        actual.len(),
        window(expected).escape_debug(),
        window(actual).escape_debug()
    )
}

fn header_diff(expected: &[String], actual: &[String]) -> String {
    let mut out = String::new();
    let rows = expected.len().max(actual.len());

    for i in 0..rows {
        let want = expected.get(i).map(String::as_str).unwrap_or("-");
        let got = actual.get(i).map(String::as_str).unwrap_or("-");
        let mark = if want == got { ' ' } else { '!' };

        out.push_str(&format!("     {} {:<52} | {}\n", mark, want, got));
    }

    out
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let root = PathBuf::from(std::env::var("CONFORMANCE_DIR").unwrap_or_else(|_| {
        format!(
            "{}/../openworkers-conformance/fixtures/sveltekit-app",
            env!("CARGO_MANIFEST_DIR")
        )
    }));

    let spec: Spec = serde_json::from_slice(
        &std::fs::read(root.join("scenarios.json")).expect("scenarios.json"),
    )
    .expect("bad scenarios.json");
    let oracle: Oracle =
        serde_json::from_slice(&std::fs::read(root.join("oracle.json")).expect("oracle.json"))
            .expect("bad oracle.json");

    let bundle = std::fs::read(root.join(&spec.bundle)).expect("no bundle");
    let lowered = parse_worker_code(&bundle, CodeLanguage::JavaScript).expect("transform failed");

    println!(
        "bundle  {} bytes sha {} ({})",
        bundle.len(),
        &sha256(&bundle)[..16],
        if bundle.len() == oracle.bundle.bytes && sha256(&bundle) == oracle.bundle.sha256 {
            "matches oracle"
        } else {
            "DIFFERS from oracle"
        }
    );
    println!(
        "lowered {} bytes sha {} ({})",
        lowered.len(),
        &sha256(lowered.as_bytes())[..16],
        if lowered.len() == oracle.lowered.bytes
            && sha256(lowered.as_bytes()) == oracle.lowered.sha256
        {
            "matches oracle"
        } else {
            "DIFFERS from oracle"
        }
    );

    let source = format!("{}\n{}", lowered, ADAPTER);
    let dump = std::env::var("CONFORMANCE_DUMP").ok().map(PathBuf::from);

    if let Some(dir) = &dump {
        std::fs::create_dir_all(dir).expect("cannot create dump dir");
    }

    let mut failures = 0;

    for scenario in &spec.scenarios {
        let recorded = oracle
            .scenarios
            .iter()
            .find(|s| s.name == scenario.name)
            .map(|s| &s.response)
            .expect("scenario missing from oracle");

        let mut worker = spawn(&source).await;
        let cold = dispatch(&mut worker, &spec.base_url, &scenario.request).await;
        let warm = dispatch(&mut worker, &spec.base_url, &scenario.request).await;
        drop(worker);

        if let Some(dir) = &dump {
            let name = PathBuf::from(&recorded.body_file);
            let name = name.file_name().expect("body file has no name");
            std::fs::write(dir.join(name), &cold.body).expect("cannot write body");
        }

        let want_body = std::fs::read(root.join(&recorded.body_file)).unwrap_or_default();
        let status_ok = cold.status == recorded.status;
        let headers_ok = cold.headers == recorded.headers;
        let body_ok = sha256(&cold.body) == recorded.body_sha256;
        let warm_identical =
            warm.status == cold.status && warm.headers == cold.headers && warm.body == cold.body;
        let warm_ok = warm_identical == recorded.warm_identical;

        let verdict = match (status_ok, headers_ok, body_ok, warm_ok) {
            (true, true, true, true) => "PASS",
            (true, false, true, true) => "PARTIAL",
            _ => "FAIL",
        };

        if verdict != "PASS" {
            failures += 1;
        }

        println!(
            "\n{:<22} {}  status {} ({}), headers {}, body {}, warm {}",
            scenario.name,
            verdict,
            cold.status,
            recorded.status,
            if headers_ok { "ok" } else { "DIFF" },
            if body_ok { "ok" } else { "DIFF" },
            if warm_ok { "ok" } else { "DIFF" },
        );

        if !headers_ok {
            println!("   header diff (oracle | ours):");
            print!("{}", header_diff(&recorded.headers, &cold.headers));
        }

        if !body_ok {
            println!("   body diff:");
            println!("{}", body_divergence(&want_body, &cold.body));
        }
    }

    println!(
        "\n{}/{} scenarios match the oracle",
        spec.scenarios.len() - failures,
        spec.scenarios.len()
    );
}
