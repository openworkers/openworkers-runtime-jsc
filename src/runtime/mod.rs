pub mod bindings;

use rusty_jsc::{JSContext, JSValue};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// Type for async JS callbacks
pub type JsFuture = Pin<Box<dyn Future<Output = Result<JSValue, String>> + Send>>;

/// Message sent from JS to the event loop
pub enum EventLoopMessage {
    /// Execute an async task
    ExecuteTask(JsFuture),
    /// Shutdown the event loop
    Shutdown,
}

/// Runtime that manages JSContext and tokio event loop
pub struct Runtime {
    /// JavaScript context
    pub context: JSContext,
    /// Channel to send messages to the event loop
    pub event_tx: mpsc::UnboundedSender<EventLoopMessage>,
    /// Channel to receive messages from the event loop
    event_rx: Arc<Mutex<mpsc::UnboundedReceiver<EventLoopMessage>>>,
}

impl Runtime {
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let mut context = JSContext::default();

        // Setup console.log
        bindings::setup_console(&mut context);

        Self {
            context,
            event_tx,
            event_rx: Arc::new(Mutex::new(event_rx)),
        }
    }

    /// Run the event loop until all tasks are completed
    pub async fn run_event_loop(&mut self) {
        let mut rx = self.event_rx.lock().unwrap();

        while let Some(msg) = rx.recv().await {
            match msg {
                EventLoopMessage::ExecuteTask(fut) => {
                    // Execute the future
                    match fut.await {
                        Ok(_result) => {
                            log::debug!("Task completed successfully");
                        }
                        Err(e) => {
                            log::error!("Task failed: {}", e);
                        }
                    }
                }
                EventLoopMessage::Shutdown => {
                    log::info!("Shutting down event loop");
                    break;
                }
            }
        }
    }

    /// Evaluate a JavaScript script
    pub fn evaluate(&mut self, script: &str) -> Result<JSValue, JSValue> {
        self.context.evaluate_script(script, 1)
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // Send shutdown message
        let _ = self.event_tx.send(EventLoopMessage::Shutdown);
    }
}
