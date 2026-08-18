# OpenWorkers Runtime JSC

JavaScriptCore-based JavaScript runtime for serverless workers, built on [rusty_jsc](https://github.com/wasmerio/rusty_jsc).

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

## Testing

```bash
cargo test
```

## License

MIT
