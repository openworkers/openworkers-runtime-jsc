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

### Runtime Comparison (v0.5.0)

| Runtime | Engine | Worker::new() | exec_simple | exec_json | Tests |
|---------|--------|---------------|-------------|-----------|-------|
| **[QuickJS](https://github.com/openworkers/openworkers-runtime-quickjs)** | QuickJS | 738µs | **12.4µs** ⚡ | **13.7µs** | 16/17 |
| **[V8](https://github.com/openworkers/openworkers-runtime-v8)** | V8 | 790µs | 32.3µs | 34.3µs | **17/17** |
| **[JSC](https://github.com/openworkers/openworkers-runtime-jsc)** | JavaScriptCore | 1.07ms | 30.3µs | 28.3µs | 15/17 |
| **[Deno](https://github.com/openworkers/openworkers-runtime-deno)** | V8 + Deno | 2.56ms | 46.8µs | 38.7µs | **17/17** |
| **[Boa](https://github.com/openworkers/openworkers-runtime-boa)** | Boa | 738µs | 12.4µs | 13.7µs | 13/17 |

**JSC has excellent exec performance** (~28-30µs) with native macOS/iOS integration.

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
