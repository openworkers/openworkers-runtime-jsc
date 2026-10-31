# OpenWorkers Runtime JSC

JavaScriptCore-based JavaScript runtime for serverless workers, built on
[openworkers-jsc](https://github.com/openworkers/openworkers-jsc).

## Quick Start

```rust
use openworkers_runtime_jsc::{Worker, Script, Event};

let script = Script::new(r#"
    addEventListener('fetch', event => {
        event.respondWith(new Response('Hello!'));
    });
"#);

let mut worker = Worker::new(script, None).await?;

let (task, rx) = Event::fetch(request);
worker.exec(task).await?;
let response = rx.await?;
```

## Features

- **Streaming** - ReadableStream with backpressure
- **Web APIs** - fetch, setTimeout, Response, Request, Headers, URL, console
- **Async/await** - Full Promise support
- **Text encoding** - TextEncoder, TextDecoder
- **Base64** - atob, btoa
- **Crypto** - getRandomValues, randomUUID, SubtleCrypto
- **Invoke binding** - with the `invoke` feature, `env.<NAME>.invoke(command, args)`
  calls `OperationsHandler::handle_binding_invoke` with JSON arguments; the
  other binding types are not served

## Web APIs

| API                          | Status                             |
| ---------------------------- | ---------------------------------- |
| console                      | yes                                |
| fetch                        | yes                                |
| setTimeout / setInterval     | yes                                |
| Promise / queueMicrotask     | yes                                |
| Request / Response / Headers | yes                                |
| ReadableStream               | yes                                |
| URL / URLSearchParams        | yes                                |
| TextEncoder / TextDecoder    | yes                                |
| atob / btoa                  | yes                                |
| crypto.getRandomValues       | yes                                |
| crypto.randomUUID            | yes                                |
| crypto.subtle.digest         | SHA-1, SHA-256, SHA-384, SHA-512   |
| crypto.subtle.sign / verify  | HMAC, ECDSA P-256, RSA PKCS#1 v1.5 |
| crypto.subtle.importKey      | raw, pkcs8, spki                   |
| crypto.subtle.generateKey    | ECDSA P-256                        |
| Blob / File / FormData       | no                                 |
| AbortController              | no                                 |

See [TODO.md](TODO.md) for planned features.

## JavaScriptCore builds

| Feature | JavaScriptCore |
| ------- | -------------- |
| `system` (default) | JavaScriptCore.framework of macOS |
| `static` | the pinned static JSC of oven-sh/WebKit, macOS arm64 only (wins over `system`) |

- With the JIT, the CPU limit needs polling traps: call
  `openworkers_jsc::init` with `polling_traps` before other threads start,
  or set `JSC_usePollingTraps=true`. The system framework runs the JIT only
  in a binary with the `com.apple.security.cs.allow-jit` entitlement; the
  static build always runs it. With the JIT and without polling traps, the
  watchdog cuts some loops late or never: an empty loop and a loop of
  `Date.now()` calls on the system framework, a loop of `Date.now()` calls
  on the static build. The check at the first worker runs these loops,
  and refuses every worker when a cut comes late.
- `static`: the WTF timers (GC, sweeper, FinalizationRegistry callbacks) go
  to a host per thread, which the worker fires from its event loop under its
  CPU budget.

## Testing

```bash
cargo test --features invoke
JSC_usePollingTraps=true cargo test --features invoke,static
```

## License

MIT
