pub mod bindings;

use rusty_jsc::{JSContext, JSObject, JSValue};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// Unique ID for callbacks
pub type CallbackId = u64;

/// Message sent from JS to schedule async operations
pub enum SchedulerMessage {
    /// Schedule a timeout: (callback_id, delay_ms)
    ScheduleTimeout(CallbackId, u64),
    /// Schedule an interval: (callback_id, interval_ms)
    ScheduleInterval(CallbackId, u64),
    /// Clear a timer (timeout or interval): (callback_id)
    ClearTimer(CallbackId),
    /// Shutdown the event loop
    Shutdown,
}

/// Message sent back from the event loop to execute callbacks
pub enum CallbackMessage {
    /// Execute a timeout callback (one-shot)
    ExecuteTimeout(CallbackId),
    /// Execute an interval callback (repeating)
    ExecuteInterval(CallbackId),
}

/// Runtime that manages JSContext and tokio event loop
pub struct Runtime {
    /// JavaScript context
    pub context: JSContext,
    /// Channel to send scheduler messages to the event loop
    pub scheduler_tx: mpsc::UnboundedSender<SchedulerMessage>,
    /// Channel to receive callback messages from the event loop
    pub callback_rx: mpsc::UnboundedReceiver<CallbackMessage>,
    /// Stored callbacks (callback_id -> JSObject function) - shared with bindings
    pub(crate) callbacks: Arc<Mutex<HashMap<CallbackId, JSObject>>>,
    /// Next callback ID - shared with bindings
    #[allow(dead_code)]
    pub(crate) next_callback_id: Arc<Mutex<CallbackId>>,
    /// Track which callbacks are intervals (vs timeouts) - shared with bindings
    pub(crate) intervals: Arc<Mutex<std::collections::HashSet<CallbackId>>>,
}

impl Runtime {
    pub fn new() -> (
        Self,
        mpsc::UnboundedReceiver<SchedulerMessage>,
        mpsc::UnboundedSender<CallbackMessage>,
    ) {
        let (scheduler_tx, scheduler_rx) = mpsc::unbounded_channel();
        let (callback_tx, callback_rx) = mpsc::unbounded_channel();

        let callbacks: Arc<Mutex<HashMap<CallbackId, JSObject>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let next_callback_id: Arc<Mutex<CallbackId>> = Arc::new(Mutex::new(1));
        let intervals: Arc<Mutex<std::collections::HashSet<CallbackId>>> =
            Arc::new(Mutex::new(std::collections::HashSet::new()));

        let mut context = JSContext::default();

        // Setup console.log
        bindings::setup_console(&mut context);

        // Setup timer bindings (pass shared state)
        bindings::setup_timer(
            &mut context,
            scheduler_tx.clone(),
            callbacks.clone(),
            next_callback_id.clone(),
            intervals.clone(),
        );

        let runtime = Self {
            context,
            scheduler_tx,
            callback_rx,
            callbacks,
            next_callback_id,
            intervals,
        };

        (runtime, scheduler_rx, callback_tx)
    }

    /// Clear a timer (remove from callbacks and intervals)
    pub fn clear_timer(&mut self, callback_id: CallbackId) {
        let mut cbs = self.callbacks.lock().unwrap();
        cbs.remove(&callback_id);

        let mut intervals = self.intervals.lock().unwrap();
        intervals.remove(&callback_id);

        // Send clear message to event loop
        let _ = self.scheduler_tx.send(SchedulerMessage::ClearTimer(callback_id));
    }

    /// Process pending callbacks (non-blocking)
    pub fn process_callbacks(&mut self) {
        while let Ok(msg) = self.callback_rx.try_recv() {
            match msg {
                CallbackMessage::ExecuteTimeout(callback_id) => {
                    // Timeouts are one-shot: remove the callback after execution
                    let callback_opt = {
                        let mut cbs = self.callbacks.lock().unwrap();
                        cbs.remove(&callback_id)
                    };

                    if let Some(callback) = callback_opt {
                        log::debug!("Executing timeout callback {}", callback_id);

                        // Call the callback
                        match callback.call_as_function(&self.context, None, &[]) {
                            Ok(_) => log::debug!("Callback {} executed successfully", callback_id),
                            Err(e) => {
                                if let Ok(err_str) = e.to_js_string(&self.context) {
                                    log::error!("Callback {} failed: {}", callback_id, err_str);
                                } else {
                                    log::error!("Callback {} failed with unknown error", callback_id);
                                }
                            }
                        }
                    }
                }
                CallbackMessage::ExecuteInterval(callback_id) => {
                    // Intervals keep the callback for repeated execution
                    let callback_opt = {
                        let cbs = self.callbacks.lock().unwrap();
                        cbs.get(&callback_id).cloned()
                    };

                    if let Some(callback) = callback_opt {
                        // Check if interval is still active
                        let is_active = {
                            let intervals = self.intervals.lock().unwrap();
                            intervals.contains(&callback_id)
                        };

                        if !is_active {
                            log::debug!("Interval {} was cleared, skipping execution", callback_id);
                            continue;
                        }

                        log::debug!("Executing interval callback {}", callback_id);

                        // Call the callback
                        match callback.call_as_function(&self.context, None, &[]) {
                            Ok(_) => log::debug!("Interval {} executed successfully", callback_id),
                            Err(e) => {
                                if let Ok(err_str) = e.to_js_string(&self.context) {
                                    log::error!("Interval {} failed: {}", callback_id, err_str);
                                } else {
                                    log::error!("Interval {} failed with unknown error", callback_id);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Evaluate a JavaScript script
    pub fn evaluate(&mut self, script: &str) -> Result<JSValue, JSValue> {
        self.context.evaluate_script(script, 1)
    }
}

/// Background event loop that handles scheduled tasks
pub async fn run_event_loop(
    mut scheduler_rx: mpsc::UnboundedReceiver<SchedulerMessage>,
    callback_tx: mpsc::UnboundedSender<CallbackMessage>,
) {
    use std::collections::HashMap;
    use tokio::task::JoinHandle;

    log::info!("Event loop started");

    // Track running tasks so we can cancel them
    let mut running_tasks: HashMap<CallbackId, JoinHandle<()>> = HashMap::new();

    while let Some(msg) = scheduler_rx.recv().await {
        match msg {
            SchedulerMessage::ScheduleTimeout(callback_id, delay_ms) => {
                log::debug!("Scheduling timeout {} with delay {}ms", callback_id, delay_ms);

                let callback_tx = callback_tx.clone();
                let handle = tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    let _ = callback_tx.send(CallbackMessage::ExecuteTimeout(callback_id));
                });

                running_tasks.insert(callback_id, handle);
            }
            SchedulerMessage::ScheduleInterval(callback_id, interval_ms) => {
                log::debug!(
                    "Scheduling interval {} with period {}ms",
                    callback_id,
                    interval_ms
                );

                let callback_tx = callback_tx.clone();
                let handle = tokio::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_millis(interval_ms));
                    // Skip the first tick (it fires immediately)
                    interval.tick().await;

                    loop {
                        interval.tick().await;
                        if callback_tx
                            .send(CallbackMessage::ExecuteInterval(callback_id))
                            .is_err()
                        {
                            // Channel closed, stop the interval
                            break;
                        }
                    }
                });

                running_tasks.insert(callback_id, handle);
            }
            SchedulerMessage::ClearTimer(callback_id) => {
                log::debug!("Clearing timer {}", callback_id);

                if let Some(handle) = running_tasks.remove(&callback_id) {
                    handle.abort();
                }
            }
            SchedulerMessage::Shutdown => {
                log::info!("Shutting down event loop");

                // Abort all running tasks
                for (_, handle) in running_tasks.drain() {
                    handle.abort();
                }

                break;
            }
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // Send shutdown message
        let _ = self.scheduler_tx.send(SchedulerMessage::Shutdown);
    }
}
