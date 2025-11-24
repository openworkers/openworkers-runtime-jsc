//! Snapshot support for JSCore runtime
//!
//! Unlike V8, JavaScriptCore doesn't expose a bytecode snapshot API.
//! However, we can achieve similar benefits using JSC's Context Groups:
//!
//! - Contexts in the same group share internally cached bytecode
//! - The first evaluation compiles the code, subsequent evaluations reuse it
//! - This provides fast instantiation of pre-configured contexts
//!
//! # Example
//!
//! ```rust,ignore
//! use openworkers_runtime_jsc::snapshot::{Snapshot, SnapshotBuilder};
//!
//! // Create a snapshot with pre-loaded scripts
//! let snapshot = SnapshotBuilder::new()
//!     .add_script("const API_VERSION = '1.0';")
//!     .add_script("function helper(x) { return x * 2; }")
//!     .build();
//!
//! // Create contexts from the snapshot (fast, reuses cached bytecode)
//! let ctx1 = snapshot.create_context().unwrap();
//! let ctx2 = snapshot.create_context().unwrap();
//!
//! // Both contexts have the pre-loaded scripts available
//! assert_eq!(ctx1.evaluate("helper(21)").unwrap(), "42");
//! ```

use crate::context_group::{ContextFactory, ContextGroup, GroupedContext};

/// A snapshot containing pre-compiled script templates.
///
/// Contexts created from this snapshot share a context group,
/// allowing JSC to reuse internally cached bytecode.
pub struct Snapshot {
    factory: ContextFactory,
}

impl Snapshot {
    /// Create an empty snapshot.
    pub fn empty() -> Self {
        Self {
            factory: ContextFactory::new(),
        }
    }

    /// Create a context from this snapshot.
    ///
    /// The returned context has all snapshot scripts pre-evaluated.
    /// Because contexts share a group, bytecode is cached and reused.
    pub fn create_context(&self) -> Result<GroupedContext, String> {
        self.factory.create_context()
    }

    /// Get the underlying context group.
    pub fn group(&self) -> &ContextGroup {
        self.factory.group()
    }
}

/// Builder for creating snapshots with pre-loaded scripts.
pub struct SnapshotBuilder {
    factory: ContextFactory,
}

impl SnapshotBuilder {
    /// Create a new snapshot builder.
    pub fn new() -> Self {
        Self {
            factory: ContextFactory::new(),
        }
    }

    /// Add a script to be evaluated in each context created from this snapshot.
    ///
    /// Scripts are evaluated in the order they are added.
    pub fn add_script(mut self, source: impl Into<String>) -> Self {
        self.factory.add_script(source);
        self
    }

    /// Build the snapshot.
    ///
    /// This "warms up" the bytecode cache by creating and immediately
    /// dropping a context, causing JSC to compile all scripts.
    pub fn build(self) -> Snapshot {
        // Warm up the bytecode cache by creating one context
        // This ensures the first real context creation is fast
        if let Ok(_warmup_ctx) = self.factory.create_context() {
            // Context is dropped here, but bytecode remains cached in the group
        }

        Snapshot {
            factory: self.factory,
        }
    }
}

impl Default for SnapshotBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Legacy snapshot output (for compatibility with existing API).
pub struct SnapshotOutput {
    pub output: Vec<u8>,
}

/// Create a runtime snapshot (legacy compatibility function).
///
/// Note: This returns an empty snapshot for compatibility.
/// Use `SnapshotBuilder` for the new context group-based approach.
pub fn create_runtime_snapshot() -> Result<SnapshotOutput, String> {
    // For backwards compatibility, return empty output
    // New code should use SnapshotBuilder instead
    Ok(SnapshotOutput { output: Vec::new() })
}

/// Create a snapshot with the standard runtime setup (URL API, etc.)
pub fn create_standard_snapshot() -> Snapshot {
    SnapshotBuilder::new()
        .add_script(include_str!("runtime/polyfills/url.js"))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_snapshot() {
        let snapshot = Snapshot::empty();
        let ctx = snapshot.create_context().unwrap();

        let result = ctx.evaluate("1 + 1").unwrap();
        assert_eq!(result, "2");
    }

    #[test]
    fn test_snapshot_builder() {
        let snapshot = SnapshotBuilder::new()
            .add_script("const GREETING = 'Hello';")
            .add_script("function greet(name) { return GREETING + ', ' + name + '!'; }")
            .build();

        let ctx = snapshot.create_context().unwrap();
        let result = ctx.evaluate("greet('World')").unwrap();
        assert_eq!(result, "Hello, World!");
    }

    #[test]
    fn test_multiple_contexts_from_snapshot() {
        let snapshot = SnapshotBuilder::new()
            .add_script("var counter = 0;")
            .add_script("function increment() { return ++counter; }")
            .build();

        // Each context gets its own copy of the variables
        let ctx1 = snapshot.create_context().unwrap();
        let ctx2 = snapshot.create_context().unwrap();

        assert_eq!(ctx1.evaluate("increment()").unwrap(), "1");
        assert_eq!(ctx1.evaluate("increment()").unwrap(), "2");

        // ctx2 has its own counter starting at 0
        assert_eq!(ctx2.evaluate("increment()").unwrap(), "1");
    }

    #[test]
    fn test_snapshot_with_complex_code() {
        let snapshot = SnapshotBuilder::new()
            .add_script(
                r#"
                class EventEmitter {
                    constructor() {
                        this.events = {};
                    }
                    on(event, callback) {
                        if (!this.events[event]) {
                            this.events[event] = [];
                        }
                        this.events[event].push(callback);
                    }
                    emit(event, data) {
                        if (this.events[event]) {
                            this.events[event].forEach(cb => cb(data));
                        }
                    }
                }
            "#,
            )
            .build();

        let ctx = snapshot.create_context().unwrap();

        let result = ctx
            .evaluate(
                r#"
            const emitter = new EventEmitter();
            let received = null;
            emitter.on('test', (data) => { received = data; });
            emitter.emit('test', 'hello');
            received;
        "#,
            )
            .unwrap();

        assert_eq!(result, "hello");
    }
}
