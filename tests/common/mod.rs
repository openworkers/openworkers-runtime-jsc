use openworkers_runtime_jsc::{DefaultOps, OperationsHandle, Runtime, run_event_loop};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Helper to run JS code and collect console output
pub struct TestRunner {
    pub runtime: Runtime,
    #[allow(dead_code)]
    pub console_output: Arc<Mutex<Vec<String>>>,
    event_loop_handle: Option<tokio::task::JoinHandle<()>>,
}

impl TestRunner {
    pub fn new() -> Self {
        let ops: OperationsHandle = Arc::new(DefaultOps);
        Self::new_with_ops(ops)
    }

    pub fn new_with_ops(ops: OperationsHandle) -> Self {
        let (runtime, scheduler_rx, callback_tx, stream_manager) = Runtime::new();

        // Spawn event loop
        let event_loop_handle = tokio::spawn(async move {
            run_event_loop(scheduler_rx, callback_tx, stream_manager, ops).await;
        });

        Self {
            runtime,
            console_output: Arc::new(Mutex::new(Vec::new())),
            event_loop_handle: Some(event_loop_handle),
        }
    }

    /// Execute JavaScript and return result
    pub fn execute(&mut self, script: &str) -> Result<(), String> {
        match self.runtime.evaluate(script) {
            Ok(_) => Ok(()),
            Err(e) => {
                if let Ok(err_str) = e.to_js_string(&self.runtime.context) {
                    Err(err_str.to_string())
                } else {
                    Err("Unknown error".to_string())
                }
            }
        }
    }

    /// Process callbacks for a duration
    pub async fn process_for(&mut self, duration: Duration) {
        let iterations = (duration.as_millis() / 10) as usize;
        for _ in 0..iterations {
            self.runtime.process_callbacks();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // Final drain
        self.runtime.process_callbacks();
    }

    /// Wait a bit for timers to fire
    #[allow(dead_code)]
    pub async fn wait(&mut self) {
        self.process_for(Duration::from_millis(100)).await;
    }

    /// Shutdown the runtime
    pub async fn shutdown(mut self) {
        drop(self.runtime);
        if let Some(handle) = self.event_loop_handle.take() {
            let _ = tokio::time::timeout(Duration::from_secs(1), handle).await;
        }
    }
}
