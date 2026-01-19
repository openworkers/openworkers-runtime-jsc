# OpenWorkers Runtime JSC

JavaScriptCore-based JavaScript runtime for serverless workers, built on [rusty_jsc](https://github.com/aspect-build/aspect_aspect).

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

- **Streaming** — ReadableStream with backpressure
- **Web APIs** — fetch, setTimeout, Response, Request, Headers, URL, console
- **Async/await** — Full Promise support
- **Text encoding** — TextEncoder, TextDecoder
- **Base64** — atob, btoa

## Web APIs

| API                          | Status |
| ---------------------------- | ------ |
| console                      | ✅     |
| fetch                        | ✅     |
| setTimeout / setInterval     | ✅     |
| Promise / queueMicrotask     | ✅     |
| Request / Response / Headers | ✅     |
| ReadableStream               | ✅     |
| URL / URLSearchParams        | ✅     |
| TextEncoder / TextDecoder    | ✅     |
| atob / btoa                  | ✅     |
| Crypto                       | ❌     |
| Blob / File / FormData       | ❌     |
| AbortController              | ❌     |

See [TODO.md](TODO.md) for planned features.

## Testing

```bash
cargo test
```

## License

MIT
