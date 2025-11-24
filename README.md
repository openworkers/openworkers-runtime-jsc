# OpenWorkers Runtime - JSCore

An alternative implementation of the OpenWorkers runtime using JavaScriptCore instead of Deno/V8.

## Goal

This project explores using JavaScriptCore (via `rusty_jsc`) as a JavaScript runtime for OpenWorkers, providing a lighter alternative to the current Deno-based solution.

## Architecture

### Main Components

- **Runtime**: Manages the JavaScriptCore context lifecycle and Tokio event loop
- **Bindings**: Implements native JavaScript APIs (console.log, setTimeout, fetch, etc.)
- **Event Loop**: Integrates Tokio to handle asynchronous operations

### Project Structure

```
src/
├── lib.rs              # Library entry point
├── main.rs             # Usage example
└── runtime/
    ├── mod.rs          # Main runtime
    └── bindings.rs     # JavaScript bindings
```

## Current Features

- [x] Basic JavaScriptCore context
- [x] console.log
- [x] Event loop architecture with Tokio
- [x] setTimeout (with proper async execution)
- [x] setInterval (repeating timers)
- [x] clearTimeout/clearInterval (cancel timers)
- [ ] fetch API
- [ ] Event handlers (fetch, scheduled)

## Usage

### Basic Example

```rust
use openworkers_runtime_jscore::{run_event_loop, Runtime};

#[tokio::main]
async fn main() {
    let (mut runtime, scheduler_rx, callback_tx) = Runtime::new();

    // Spawn background event loop
    let event_loop = tokio::spawn(async move {
        run_event_loop(scheduler_rx, callback_tx).await;
    });

    let script = r#"
        console.log("Hello from JavaScriptCore!");

        // setTimeout example
        setTimeout(() => {
            console.log("This runs after 100ms!");
        }, 100);

        // setInterval example
        let count = 0;
        const intervalId = setInterval(() => {
            count++;
            console.log("Interval tick", count);
            if (count >= 3) {
                clearInterval(intervalId);
            }
        }, 200);
    "#;

    runtime.evaluate(script).unwrap();

    // Process callbacks
    for _ in 0..10 {
        runtime.process_callbacks();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
```

## Building

```bash
cargo build
cargo run
```

## Comparison with openworkers-runtime (Deno)

| Aspect | Deno/V8 | JSCore |
|--------|---------|---------|
| Binary size | ~50MB | ~5MB (estimated) |
| Snapshots | Yes | No (for now) |
| Extensions | deno_* | Custom |
| Performance | Very fast | Fast |
| Maturity | Production | Experimental |

## Next Steps

1. Implement setTimeout with Tokio timers
2. Create a callback system to handle async results
3. Implement fetch with reqwest
4. Add event handlers (fetch, scheduled) for OpenWorkers compatibility
5. Performance benchmarks vs Deno runtime

## Technical Notes

### JSCore Limitations

- `JSContext` is not `Send`, so must stay on the main thread
- No native snapshot support (unlike V8)
- Need a callback architecture for async operations

### Advantages

- Lighter runtime
- Fewer dependencies
- Native integration on macOS/iOS
- Stable C API

## Dependencies

- `rusty_jsc`: Rust bindings for JavaScriptCore
- `tokio`: Async runtime
- `futures`: Async primitives

## License

Same license as OpenWorkers
