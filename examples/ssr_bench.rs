// SSR benchmark: wake a worker, render a SvelteKit page, return HTML, sleep.
//
// To run: cargo run --release --example ssr_bench
//
// SSR_BUNDLE overrides the fixture, SSR_OUT writes the rendered page to a file.

use openworkers_core::Event;
use openworkers_core::HttpMethod;
use openworkers_core::HttpRequest;
use openworkers_core::RequestBody;
use openworkers_core::Script;
use openworkers_runtime_jsc::Worker;
use openworkers_transform::CodeLanguage;
use openworkers_transform::parse_worker_code;
use std::collections::HashMap;
use std::time::Duration;
use std::time::Instant;

const WARM_RENDERS: usize = 20;
const COLD_CYCLES: usize = 10;
const RESIDENT_WORKERS: usize = 8;

/// Every route of the fixture site is prerendered, so this one is what the SSR pipeline serves
const SSR_URL: &str = "http://localhost/ssr-bench";
const PRERENDERED_URL: &str = "http://localhost/";

/// console writes straight to stdout, which would drown the measurements
const PRELUDE: &str = r#"
globalThis.__consoleLines = [];
(function() {
    const fmt = (a) => {
        try {
            if (a instanceof Error) return a.stack || String(a);
            return typeof a === 'object' ? JSON.stringify(a) : String(a);
        } catch (e) {
            return String(a);
        }
    };
    const sink = (level) => (...args) => {
        globalThis.__consoleLines.push(level + ' ' + args.map(fmt).join(' '));
    };
    globalThis.console = {
        log: sink('[LOG]'),
        info: sink('[INFO]'),
        warn: sink('[WARN]'),
        error: sink('[ERROR]'),
        debug: sink('[DEBUG]')
    };
})();
"#;

/// This runtime dispatches to addEventListener, so bridge the module handler onto it
const ADAPTER: &str = r#"
globalThis.__ssrEnv = {
    ASSETS: { fetch: () => new Response(null, { status: 404 }) }
};
addEventListener('fetch', (event) => {
    event.respondWith(
        globalThis.default.fetch(event.request, globalThis.__ssrEnv, { waitUntil() {} })
    );
});
"#;

fn bundle_path() -> String {
    std::env::var("SSR_BUNDLE").unwrap_or_else(|_| {
        format!(
            "{}/examples/fixtures/sveltekit_worker.js",
            env!("CARGO_MANIFEST_DIR")
        )
    })
}

fn rss_kb() -> Option<u64> {
    let pid = std::process::id().to_string();
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .ok()?;

    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

fn report_console(worker: &mut Worker) {
    let value = match worker.evaluate("globalThis.__consoleLines.splice(0).join('\\n')") {
        Ok(value) => value,
        Err(e) => {
            println!("console drain failed: {}", e);
            return;
        }
    };

    let text = match value.to_js_string(worker.context()) {
        Ok(text) => text.to_string(),
        Err(_) => return,
    };

    for line in text.lines().filter(|line| !line.is_empty()) {
        println!("  worker console: {}", line);
    }
}

struct Rendered {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

async fn render(worker: &mut Worker, url: &str) -> Result<Rendered, String> {
    let request = HttpRequest {
        method: HttpMethod::Get,
        url: url.to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    };

    let (task, rx) = Event::fetch(request);

    worker
        .exec(task)
        .await
        .map_err(|reason| format!("{:?}", reason))?;

    let response = rx.await.map_err(|e| e.to_string())?;
    let body = response.body.collect().await.unwrap_or_default();

    Ok(Rendered {
        status: response.status,
        headers: response.headers,
        body: body.to_vec(),
    })
}

async fn spawn_worker(source: &str) -> Result<Worker, String> {
    Worker::new(Script::new(source), None)
        .await
        .map_err(|reason| format!("{:?}", reason))
}

fn min_median(mut samples: Vec<Duration>) -> (Duration, Duration) {
    samples.sort();

    (samples[0], samples[samples.len() / 2])
}

fn ms(duration: Duration) -> String {
    format!("{:.2} ms", duration.as_secs_f64() * 1000.0)
}

fn row(phase: &str, min: Duration, median: Duration) {
    println!("| {:<38} | {:>10} | {:>10} |", phase, ms(min), ms(median));
}

fn fail(message: String) -> ! {
    println!("{}", message);
    std::process::exit(1);
}

/// Lets other runtimes be compared byte for byte on the same fixture
fn sha256(body: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, body)
        .as_ref()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let path = bundle_path();
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {}", path, e));

    let lower_start = Instant::now();
    let lowered = parse_worker_code(&raw, CodeLanguage::JavaScript).expect("bundle should lower");
    let lower_time = lower_start.elapsed();

    let source = format!("{}\n{}\n{}", PRELUDE, lowered, ADAPTER);

    println!("bundle: {} ({} bytes)", path, raw.len());
    println!("lowered to classic script in {}", ms(lower_time));
    println!("script handed to the runtime: {} bytes\n", source.len());

    // An empty worker separates our own global setup from the cost of the bundle
    let bootstrap_start = Instant::now();
    let bootstrap = spawn_worker("").await.expect("empty worker should load");
    let bootstrap_time = bootstrap_start.elapsed();

    drop(bootstrap);

    let rss_before = rss_kb();

    let create_start = Instant::now();
    let mut worker = match spawn_worker(&source).await {
        Ok(worker) => worker,
        Err(e) => fail(format!("worker creation failed: {}", e)),
    };
    let create_time = create_start.elapsed();

    match render(&mut worker, PRERENDERED_URL).await {
        Ok(response) => println!(
            "GET {} -> {} ({} bytes): prerendered route, answered by the ASSETS stub",
            PRERENDERED_URL,
            response.status,
            response.body.len()
        ),
        Err(e) => fail(format!("render of {} failed: {}", PRERENDERED_URL, e)),
    }

    let first_start = Instant::now();
    let first = render(&mut worker, SSR_URL).await;
    let first_time = first_start.elapsed();

    let first = match first {
        Ok(result) => result,
        Err(e) => {
            println!("first render failed: {}", e);
            report_console(&mut worker);
            std::process::exit(1);
        }
    };

    let html = String::from_utf8_lossy(&first.body).to_string();

    println!(
        "\nGET {} -> {} ({} bytes)",
        SSR_URL,
        first.status,
        first.body.len()
    );

    for (name, value) in &first.headers {
        println!("  {}: {}", name, value);
    }

    println!("sha256: {}", sha256(&first.body));
    println!("first 200 chars:\n{}\n", &html[..html.len().min(200)]);
    report_console(&mut worker);

    if let Ok(path) = std::env::var("SSR_OUT") {
        std::fs::write(&path, &first.body).unwrap_or_else(|e| panic!("write {}: {}", path, e));
    }

    if !html.contains("<html") || first.body.len() < 1000 {
        fail("the response is not a server-rendered page".to_string());
    }

    let mut warm = Vec::with_capacity(WARM_RENDERS);

    for i in 0..WARM_RENDERS {
        let start = Instant::now();
        let result = render(&mut worker, SSR_URL).await;
        warm.push(start.elapsed());

        match result {
            Ok(response) if response.body == first.body => {}
            Ok(response) => fail(format!(
                "warm render {} diverged: status {}, {} bytes",
                i,
                response.status,
                response.body.len()
            )),
            Err(e) => fail(format!("warm render {} failed: {}", i, e)),
        }
    }

    let rss_warm = rss_kb();

    drop(worker);

    let mut cold = Vec::with_capacity(COLD_CYCLES);

    for i in 0..COLD_CYCLES {
        let start = Instant::now();
        let mut worker = spawn_worker(&source).await.expect("worker should load");

        if let Err(e) = render(&mut worker, SSR_URL).await {
            fail(format!("cold cycle {} failed: {}", i, e));
        }

        cold.push(start.elapsed());
        drop(worker);
    }

    let rss_before_resident = rss_kb();
    let mut resident = Vec::with_capacity(RESIDENT_WORKERS);

    for _ in 0..RESIDENT_WORKERS {
        let mut worker = spawn_worker(&source).await.expect("worker should load");

        if let Err(e) = render(&mut worker, SSR_URL).await {
            fail(format!("resident render failed: {}", e));
        }

        resident.push(worker);
    }

    let rss_resident = rss_kb();

    drop(resident);

    let (warm_min, warm_median) = min_median(warm);
    let (cold_min, cold_median) = min_median(cold);

    println!("\n| {:<38} | {:>10} | {:>10} |", "phase", "min", "median");
    println!("|{:-<40}|{:-<12}|{:-<12}|", "", "", "");
    row(
        "runtime bootstrap (empty script)",
        bootstrap_time,
        bootstrap_time,
    );
    row(
        "worker creation (parse+compile+init)",
        create_time,
        create_time,
    );
    row("first render", first_time, first_time);
    row(
        &format!("warm render (x{})", WARM_RENDERS),
        warm_min,
        warm_median,
    );
    row(
        &format!("cold cycle (x{})", COLD_CYCLES),
        cold_min,
        cold_median,
    );

    if let (Some(before), Some(warm)) = (rss_before, rss_warm) {
        println!(
            "\nRSS: {} MB at start, {} MB with one worker resident",
            before / 1024,
            warm / 1024
        );
    }

    if let (Some(before), Some(resident)) = (rss_before_resident, rss_resident) {
        println!(
            "RSS per idle worker: {:.1} MB ({} resident)",
            (resident.saturating_sub(before)) as f64 / 1024.0 / RESIDENT_WORKERS as f64,
            RESIDENT_WORKERS
        );
    }
}
