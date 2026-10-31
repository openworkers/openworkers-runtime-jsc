mod cpu_limit;
pub mod runtime;
mod worker;

// Core API
pub use runtime::stream_manager::{StreamChunk, StreamManager};
pub use runtime::{Runtime, run_event_loop};
pub use worker::Worker;

// The JSC options, such as the polling traps that the CPU limit needs with
// the JIT; set them before other threads start
pub use openworkers_jsc::Options as JscOptions;
pub use openworkers_jsc::init as init_jsc;

/// Binding types this runtime serves, for an embedder that refuses a worker
/// declaring one it cannot
pub const SUPPORTED_BINDINGS: &[openworkers_core::BindingType] = &[
    #[cfg(feature = "invoke")]
    openworkers_core::BindingType::Invoke,
];

// Re-export common types from openworkers-core
pub use openworkers_core::{
    DefaultOps, Event, EventType, FetchInit, HttpMethod, HttpRequest, HttpResponse,
    HttpResponseMeta, LogEvent, LogLevel, OpFuture, Operation, OperationResult, OperationsHandle,
    OperationsHandler, RequestBody, ResponseBody, ResponseSender, RuntimeLimits, Script, TaskInit,
    TaskResult, TaskSource, TerminationReason, Worker as WorkerTrait,
};
