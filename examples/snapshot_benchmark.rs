use openworkers_runtime_jsc::context_group::{ContextFactory, ContextGroup};
use openworkers_runtime_jsc::snapshot::SnapshotBuilder;
use std::time::Instant;

fn main() {
    println!("=== JSC Context Group / Snapshot Benchmark ===\n");

    // The polyfill code to pre-load
    let polyfill = r#"
        globalThis.URL = class URL {
            constructor(url, base) {
                this.href = base ? new URL(base).origin + url : url;
                const match = this.href.match(/^(([^:/?#]+):)?(\/\/([^/?#]*))?([^?#]*)(\?([^#]*))?(#(.*))?/);
                this.protocol = match[2] ? match[2] + ':' : '';
                this.host = match[4] || '';
                this.pathname = match[5] || '/';
                this.search = match[6] || '';
                this.hash = match[8] || '';
                const hostMatch = this.host.match(/^([^:]+)(:(.+))?$/);
                this.hostname = hostMatch ? hostMatch[1] : '';
                this.port = hostMatch && hostMatch[3] ? hostMatch[3] : '';
                this.origin = this.protocol + '//' + this.host;
            }
            toString() { return this.href; }
        };

        globalThis.Response = function(body, init) {
            init = init || {};
            return {
                status: init.status || 200,
                statusText: init.statusText || 'OK',
                headers: init.headers || {},
                text: () => Promise.resolve(String(body)),
                json: () => Promise.resolve(JSON.parse(String(body))),
            };
        };

        globalThis.addEventListener = function(type, handler) {
            if (type === 'fetch') {
                globalThis.__triggerFetch = function(request) {
                    const event = {
                        request: request,
                        respondWith: function(response) { this._response = response; }
                    };
                    handler(event);
                    return event._response;
                };
            }
        };
    "#;

    let worker_code = r#"
        addEventListener('fetch', (event) => {
            const url = new URL(event.request.url);
            event.respondWith(new Response('Hello from ' + url.pathname));
        });
    "#;

    let iterations = 100;

    // ============================================
    // Benchmark 1: Separate contexts (no sharing)
    // ============================================
    println!("--- Benchmark 1: Separate Context Groups (no bytecode sharing) ---");

    let start = Instant::now();
    for _ in 0..iterations {
        let group = ContextGroup::new();
        let ctx = group.create_context();
        ctx.evaluate(polyfill).unwrap();
        ctx.evaluate(worker_code).unwrap();
        // Simulate a request
        ctx.evaluate("globalThis.__triggerFetch({ url: 'http://example.com/test' })").unwrap();
    }
    let separate_time = start.elapsed();
    println!("  {} iterations: {:?}", iterations, separate_time);
    println!("  Average: {:?}/iter\n", separate_time / iterations as u32);

    // ============================================
    // Benchmark 2: Shared context group
    // ============================================
    println!("--- Benchmark 2: Shared Context Group (bytecode cached after first) ---");

    let start = Instant::now();
    let group = ContextGroup::new();  // Single shared group
    for _ in 0..iterations {
        let ctx = group.create_context();
        ctx.evaluate(polyfill).unwrap();
        ctx.evaluate(worker_code).unwrap();
        ctx.evaluate("globalThis.__triggerFetch({ url: 'http://example.com/test' })").unwrap();
    }
    let shared_time = start.elapsed();
    println!("  {} iterations: {:?}", iterations, shared_time);
    println!("  Average: {:?}/iter\n", shared_time / iterations as u32);

    // ============================================
    // Benchmark 3: ContextFactory (pre-loaded scripts)
    // ============================================
    println!("--- Benchmark 3: ContextFactory (scripts evaluated per-context) ---");

    let mut factory = ContextFactory::new();
    factory.add_script(polyfill);
    factory.add_script(worker_code);

    let start = Instant::now();
    for _ in 0..iterations {
        let ctx = factory.create_context().unwrap();
        ctx.evaluate("globalThis.__triggerFetch({ url: 'http://example.com/test' })").unwrap();
    }
    let factory_time = start.elapsed();
    println!("  {} iterations: {:?}", iterations, factory_time);
    println!("  Average: {:?}/iter\n", factory_time / iterations as u32);

    // ============================================
    // Benchmark 4: Snapshot with warmup
    // ============================================
    println!("--- Benchmark 4: Snapshot (warmed up, bytecode pre-cached) ---");

    let snapshot = SnapshotBuilder::new()
        .add_script(polyfill)
        .add_script(worker_code)
        .build();  // Warmup happens here

    let start = Instant::now();
    for _ in 0..iterations {
        let ctx = snapshot.create_context().unwrap();
        ctx.evaluate("globalThis.__triggerFetch({ url: 'http://example.com/test' })").unwrap();
    }
    let snapshot_time = start.elapsed();
    println!("  {} iterations: {:?}", iterations, snapshot_time);
    println!("  Average: {:?}/iter\n", snapshot_time / iterations as u32);

    // ============================================
    // Summary
    // ============================================
    println!("=== Summary ===");
    println!("Separate groups: {:?} total", separate_time);
    println!("Shared group:    {:?} total ({:.1}x faster)",
        shared_time,
        separate_time.as_nanos() as f64 / shared_time.as_nanos() as f64);
    println!("ContextFactory:  {:?} total ({:.1}x faster)",
        factory_time,
        separate_time.as_nanos() as f64 / factory_time.as_nanos() as f64);
    println!("Snapshot:        {:?} total ({:.1}x faster)",
        snapshot_time,
        separate_time.as_nanos() as f64 / snapshot_time.as_nanos() as f64);
}
