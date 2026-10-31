//! The run loop timers of WTF in the static build (GC, sweeper, deferred
//! work such as FinalizationRegistry callbacks): one host per thread, whose
//! timers the worker fires from its event loop, under its CPU budget.
//!
//! The host belongs to the thread, not to a worker: a worker may fire a timer
//! of another VM of the thread. Its guest callbacks then run under the
//! watchdog of that VM, which is past its deadline when its worker is idle,
//! so they are cut and that worker is not used again.

use openworkers_jsc::timer;
use openworkers_jsc::timer::Timer;
use openworkers_jsc::timer::TimerHost;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::Instant;

#[derive(Default)]
pub(crate) struct Timers {
    pending: Mutex<Vec<(Timer, Instant)>>,
    changed: Notify,
}

thread_local! {
    static HOST: Arc<Timers> = {
        let timers = Arc::new(Timers::default());

        timer::set_host(timers.clone());

        timers
    };
}

/// The host of this thread. Call it before the first context of the thread:
/// WTF picks the kind of run loop of a thread once.
pub(crate) fn host() -> Arc<Timers> {
    HOST.with(Arc::clone)
}

impl TimerHost for Timers {
    fn schedule(&self, timer: Timer, delay: Duration) {
        let mut pending = self.pending.lock().unwrap();

        pending.retain(|(other, _)| *other != timer);
        pending.push((timer, Instant::now() + delay));
        drop(pending);

        self.changed.notify_one();
    }

    fn cancel(&self, timer: Timer) {
        self.pending
            .lock()
            .unwrap()
            .retain(|(other, _)| *other != timer);
    }
}

impl Timers {
    /// Completes when a timer is due.
    pub(crate) async fn due(&self) {
        loop {
            let next = self.pending.lock().unwrap().iter().map(|(_, at)| *at).min();

            match next {
                Some(at) => tokio::select! {
                    _ = tokio::time::sleep_until(at) => return,
                    _ = self.changed.notified() => {}
                },
                None => self.changed.notified().await,
            }
        }
    }

    /// Takes one timer that is due. One at a time, because firing a timer
    /// can free others, such as the timers of a VM that dies.
    pub(crate) fn take_due(&self) -> Option<Timer> {
        let now = Instant::now();
        let mut pending = self.pending.lock().unwrap();
        let index = pending.iter().position(|(_, at)| *at <= now)?;

        Some(pending.swap_remove(index).0)
    }
}
