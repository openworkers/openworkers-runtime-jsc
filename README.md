# OpenWorkers Runtime - JavaScriptCore

A JavaScript runtime for OpenWorkers based on [JavaScriptCore](https://developer.apple.com/documentation/javascriptcore) via [rusty_jsc](https://github.com/rustjs/rust-jsc) bindings.

## Features

- ✅ **JavaScriptCore Engine** - Apple's battle-tested JavaScript engine
- ✅ **Native Promises** - Built-in Promise support (resolve, reject, then, catch, all, race)
- ✅ **Timers** - setTimeout, setInterval, clearTimeout, clearInterval
- ✅ **Fetch API** - HTTP requests to external APIs
- ✅ **Event Handlers** - addEventListener('fetch'), addEventListener('scheduled')
- ✅ **Console Logging** - console.log/warn/error
- ✅ **URL API** - URL and URLSearchParams parsing
- ✅ **Microtasks** - queueMicrotask support

## Performance

Run benchmark:
```bash
cargo run --example benchmark --release
```

### Results (Apple Silicon, Release Mode)

```
Worker::new(): avg=620µs* (495µs after warmup), min=495µs, max=41ms
exec():        avg=478µs, min=434µs, max=542µs
Total:         avg=1.09ms, min=935µs, max=44ms
```

*First iteration has ~40ms warmup, subsequent runs stabilize at 495µs

### Runtime Comparison

| Runtime | Engine | Worker::new() | exec() | Total | Language |
|---------|--------|---------------|--------|-------|----------|
| **[V8](https://github.com/openworkers/openworkers-runtime-v8)** | V8 | 2.9ms | **15µs** ⚡ | ~3ms | Rust + C++ |
| **[JSC](https://github.com/openworkers/openworkers-runtime-jsc)** | JavaScriptCore | 495µs* | 434µs | **935µs** 🏆 | Rust + C |
| **[Boa](https://github.com/openworkers/openworkers-runtime-boa)** | Boa | 605µs | 441µs | 1.05ms | 100% Rust |
| **[Deno](https://github.com/openworkers/openworkers-runtime)** | V8 + Deno | 4.6ms | 1.07ms | 5.8ms | Rust + C++ |

*JSC has ~40ms warmup on first run, then stabilizes

**JSC has the fastest total time** (935µs) after warmup, making it ideal for low-latency scenarios.

### Worker Benchmark

| Benchmark | V8 | JSC | Boa |
|-----------|---:|----:|----:|
| Worker/new | 781 µs | **998 µs** | 1.04 ms |
| exec_simple_response | 1.05 ms | **1.87 ms** | 1.90 ms |
| exec_json_response | 1.07 ms | **2.14 ms** | 2.11 ms |

### Streaming Performance

| Metric | V8 | JSC | Boa |
|--------|---:|----:|----:|
| Buffered req/s | 71,555 | **18,480** | 4,975 |
| Local stream 100KB | 86-129 MB/s | **60-71 MB/s** | 0.2 MB/s |
| Fetch forward | ✅ zero-copy | ✅ zero-copy | ❌ buffered |

## Installation

```toml
[dependencies]
openworkers-runtime-jsc = { path = "../openworkers-runtime-jsc" }
```

Note: Requires local fork of rusty_jsc at `/Users/max/Documents/forks/rusty_jsc`

## Usage

```rust
use openworkers_runtime_jsc::{Worker, Script, Task, HttpRequest};
use std::collections::HashMap;

#[tokio::main]
async fn main() {
    let code = r#"
        addEventListener('fetch', async (event) => {
            const { pathname } = new URL(event.request.url);

            if (pathname === '/api') {
                const response = await fetch('https://api.example.com/data');
                event.respondWith(response);
            } else {
                event.respondWith(new Response('Hello from JSC!'));
            }
        });
    "#;

    let script = Script::new(code);
    let mut worker = Worker::new(script, None, None).await.unwrap();

    let req = HttpRequest {
        method: "GET".to_string(),
        url: "http://localhost/".to_string(),
        headers: HashMap::new(),
        body: None,
    };

    let (task, rx) = Task::fetch(req);
    worker.exec(task).await.unwrap();

    let response = rx.await.unwrap();
    println!("Status: {}", response.status);
}
```

## Testing

```bash
# Run all tests (42 tests)
cargo test

# Run with output
cargo test -- --nocapture
```

### Test Coverage

- **Console** (4) - Logging with various types
- **Error Handling** (4) - Syntax errors, runtime errors
- **Timers** (7) - setTimeout, setInterval, nested timers
- **Promises** (9) - resolve, reject, chains, Promise.all, Promise.race
- **Fetch** (7) - GET/POST, headers, body, status codes
- **Worker/Task** (5) - Event handlers, request/response
- **URL** (3) - URL parsing, URLSearchParams
- **Integration** (3) - Complex scenarios, Date.now(), Math

**Total: 42 tests** ✅

## Supported JavaScript APIs

### Timers
- `setTimeout(callback, delay)`
- `setInterval(callback, interval)`
- `clearTimeout(id)`
- `clearInterval(id)`

### Fetch API
- `fetch(url, options)` - HTTP requests (GET, POST, PUT, DELETE, PATCH, HEAD)
- Full Request/Response objects
- Headers API (get, has, set, delete)
- Promise-based with async/await

### Promises
- Native JavaScriptCore Promise support
- `Promise.resolve()`, `Promise.reject()`
- `Promise.all()`, `Promise.race()`
- `.then()`, `.catch()`, `.finally()`
- `queueMicrotask()`

### Other APIs
- `console.log/warn/error/info/debug`
- `URL` - Full URL parsing
- `URLSearchParams` - Query string handling
- `Response` - HTTP responses
- `addEventListener` - Event handling
- `Date.now()` - Timestamps
- `Math.*` - Standard math operations

## Architecture

```
src/
├── lib.rs              # Public API
├── worker.rs           # Worker with event handlers
├── task.rs             # Task types (Fetch, Scheduled)
├── compat.rs           # Compatibility layer
└── runtime/
    ├── mod.rs          # Runtime & event loop
    ├── bindings.rs     # JavaScript bindings
    ├── url.rs          # URL API implementation
    └── fetch/          # Fetch API implementation
        ├── mod.rs
        ├── request.rs
        ├── response.rs
        └── headers.rs
```

## Key Advantages

- **Fast after warmup** - Sub-millisecond worker creation
- **Native Promises** - Built into JavaScriptCore
- **Full URL API** - Complete URL and URLSearchParams implementation
- **Native on macOS/iOS** - Zero-overhead on Apple platforms

## Other Runtime Implementations

OpenWorkers supports multiple JavaScript engines:

- **[openworkers-runtime](https://github.com/openworkers/openworkers-runtime)** - Deno-based (V8 + Deno extensions)
- **[openworkers-runtime-jsc](https://github.com/openworkers/openworkers-runtime-jsc)** - This runtime (JavaScriptCore)
- **[openworkers-runtime-boa](https://github.com/openworkers/openworkers-runtime-boa)** - Boa (100% Rust)
- **[openworkers-runtime-v8](https://github.com/openworkers/openworkers-runtime-v8)** - V8 via rusty_v8

## License

MIT License - See LICENSE file.

## Credits

Built on JavaScriptCore via [rusty_jsc](https://github.com/rustjs/rust-jsc).
