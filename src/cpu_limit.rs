//! The CPU budget of a request, through the watchdog of JavaScriptCore.
//!
//! A request enters JavaScript many times: the event, each callback, and
//! each read of a value the guest made. Every call from Rust into JavaScript
//! goes through `run`. Before the call, `run` sets the deadline: the CPU time
//! of the thread at which the budget runs out. After the call, the CPU time
//! that the call used is taken from the budget. A spent budget ends the
//! request.
//!
//! JavaScriptCore starts the watchdog again at each entry into the VM, and
//! one API call can enter it more than once: when an exception leaves
//! JSEvaluateScript, JavaScriptCore reads its text for the inspector, which
//! runs the toString of the guest in a new entry. So the watchdog does not
//! count the budget itself. It fires at least every PERIOD of CPU time of an
//! entry, and its callback cuts the entry when the thread is past the
//! deadline. Any entry is cut at most PERIOD after the budget runs out.
//!
//! JSContextGroupSetExecutionTimeLimit is private API of JavaScriptCore
//! (JSContextRefPrivate.h), declared here. The Linux path is not tested.
//!
//! Known limits:
//! - The watchdog does not cut native code, such as a regular expression
//!   that backtracks: `(a+)+b` ran 1.7 s against a limit of 50 ms. Its time
//!   is still taken from the budget when the entry returns.
//! - With the JIT, the watchdog cuts JIT code only with polling traps: set
//!   JSC_usePollingTraps=true. Without it the check below fails, and no worker
//!   starts.
//! - The check runs a loop on a thread of its own. If the limit does not
//!   work, that thread spins for the life of the process.
//! - Only the CPU time of the thread that calls is counted, not the GC and
//!   JIT threads of JavaScriptCore.

use openworkers_core::TerminationReason;
use rusty_jsc::JSContext;
use rusty_jsc::private::{JSContextGetGroup, JSContextGroupRef, JSContextRef};
use std::cell::Cell;
use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

type ShouldTerminate = unsafe extern "C" fn(JSContextRef, *mut c_void) -> bool;

unsafe extern "C" {
    fn JSContextGroupSetExecutionTimeLimit(
        group: JSContextGroupRef,
        limit: f64,
        callback: Option<ShouldTerminate>,
        context: *mut c_void,
    );
}

/// The longest CPU time of an entry between two checks of the deadline
const PERIOD: Duration = Duration::from_millis(5);

/// The CPU floor of the top level of a script: a large bundle needs more to
/// parse and start than a request gets (LOAD_CPU_TIME_MS of runtime-v8)
const LOAD: Duration = Duration::from_millis(400);

/// What the callback of the watchdog reads and writes
struct Watch {
    /// Set once a budget ran out
    hit: AtomicBool,
    /// The CPU time of the thread, in nanoseconds, at which the budget runs out
    deadline: AtomicU64,
}

/// Called by the watchdog at each PERIOD of an entry, on the thread that runs
/// it: cuts the entry once the thread is past the deadline.
unsafe extern "C" fn check(context: JSContextRef, data: *mut c_void) -> bool {
    // SAFETY: `data` is the boxed Watch of a CpuLimit, which lives as long as
    // the worker and so as long as its context group
    let watch = unsafe { &*(data as *const Watch) };
    let now = thread_cpu_time().as_nanos() as u64;
    let deadline = watch.deadline.load(Ordering::SeqCst);

    if now < deadline {
        // Not yet. A callback that does not cut arms the next period itself,
        // or the watchdog does not fire again in this entry.
        let left = Duration::from_nanos(deadline - now).min(PERIOD);

        // SAFETY: the context is the one the watchdog runs in; JavaScriptCore
        // allows a new limit from inside the callback
        unsafe {
            JSContextGroupSetExecutionTimeLimit(
                JSContextGetGroup(context),
                left.as_secs_f64(),
                Some(check),
                data,
            );
        }

        return false;
    }

    watch.hit.store(true, Ordering::SeqCst);

    true
}

/// The CPU time of the calling thread.
fn thread_cpu_time() -> Duration {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };

    // SAFETY: `now` is a valid timespec to write into
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut now) };

    Duration::new(now.tv_sec as u64, now.tv_nsec as u32)
}

/// The limit of one worker. The flag stays set once a budget ran out: the
/// worker may hold half an event and must not be used again.
pub struct CpuLimit {
    watch: Box<Watch>,
    /// None when there is no limit
    group: Option<JSContextGroupRef>,
    budget: Duration,
    remaining: Cell<Duration>,
}

/// An entry into JavaScript: the CPU time of the thread when it started.
struct Entry(Duration);

impl CpuLimit {
    /// A budget of `limit_ms` on the group of `context`. 0 ms means none.
    pub fn new(context: &JSContext, limit_ms: u64) -> Result<Self, TerminationReason> {
        let budget = Duration::from_millis(limit_ms);
        let mut limit = Self {
            watch: Box::new(Watch {
                hit: AtomicBool::new(false),
                deadline: AtomicU64::new(u64::MAX),
            }),
            group: None,
            budget,
            remaining: Cell::new(budget),
        };

        if limit_ms == 0 {
            return Ok(limit);
        }

        if !works() {
            return Err(TerminationReason::InitializationError(
                "the CPU limit of JavaScriptCore does not work here (with the JIT, set \
                 JSC_usePollingTraps=true)"
                    .to_string(),
            ));
        }

        // SAFETY: the context is live
        limit.group = Some(unsafe { JSContextGetGroup(context.get_ref()) });

        Ok(limit)
    }

    /// Gives the whole budget again, for a new event. A spent budget stays
    /// spent.
    pub fn refill(&self) {
        self.remaining.set(self.budget);
    }

    /// Gives the budget of the top level of a script: the budget of a
    /// request or LOAD, the larger of the two.
    pub fn refill_for_load(&self) {
        self.remaining.set(self.budget.max(LOAD));
    }

    /// Makes one call into JavaScript under what is left of the budget, and
    /// takes its CPU time from it. An error when the budget was spent before
    /// the call (the call is not made) or by it.
    pub fn run<T>(&self, call: impl FnOnce() -> T) -> Result<T, TerminationReason> {
        let entry = self.begin()?;
        let out = call();

        self.end(entry)?;

        Ok(out)
    }

    /// Starts an entry into JavaScript under what is left of the budget.
    fn begin(&self) -> Result<Entry, TerminationReason> {
        if self.hit() {
            return Err(TerminationReason::CpuTimeLimit);
        }

        let now = thread_cpu_time();

        if let Some(group) = self.group {
            let remaining = self.remaining.get();
            let deadline = (now + remaining).as_nanos() as u64;

            self.watch.deadline.store(deadline, Ordering::SeqCst);

            // SAFETY: the group belongs to the live context of the worker;
            // `watch` is boxed, so its address stays the same while it lives
            unsafe {
                JSContextGroupSetExecutionTimeLimit(
                    group,
                    remaining.min(PERIOD).as_secs_f64(),
                    Some(check),
                    &*self.watch as *const Watch as *mut c_void,
                );
            }
        }

        Ok(Entry(now))
    }

    /// Ends an entry: takes its CPU time from the budget. An error when the
    /// entry was cut or the budget is spent.
    fn end(&self, entry: Entry) -> Result<(), TerminationReason> {
        if self.group.is_some() {
            let used = thread_cpu_time().saturating_sub(entry.0);
            let left = self.remaining.get().saturating_sub(used);

            self.remaining.set(left);

            if left.is_zero() {
                self.watch.hit.store(true, Ordering::SeqCst);
            }
        }

        match self.hit() {
            true => Err(TerminationReason::CpuTimeLimit),
            false => Ok(()),
        }
    }

    /// Whether a budget of this worker ran out.
    pub fn hit(&self) -> bool {
        self.watch.hit.load(Ordering::SeqCst)
    }
}

/// Whether the limit cuts a loop that never ends, checked once per process.
fn works() -> bool {
    static WORKS: OnceLock<bool> = OnceLock::new();

    *WORKS.get_or_init(|| {
        let (done, cut) = std::sync::mpsc::channel();

        let spawned = std::thread::Builder::new()
            .name("jsc-cpu-limit-check".into())
            .spawn(move || {
                let mut context = JSContext::default();

                // A deadline of 0: the first check cuts
                let watch = Watch {
                    hit: AtomicBool::new(false),
                    deadline: AtomicU64::new(0),
                };

                // SAFETY: the group belongs to `context`; `watch` outlives the
                // script, the only time the watchdog runs
                unsafe {
                    JSContextGroupSetExecutionTimeLimit(
                        JSContextGetGroup(context.get_ref()),
                        0.01,
                        Some(check),
                        &watch as *const Watch as *mut c_void,
                    );
                }

                let ended = context.evaluate_script("for (;;) {}", 1).is_err();

                done.send(ended && watch.hit.load(Ordering::SeqCst)).ok();
            });

        spawned.is_ok() && cut.recv_timeout(Duration::from_secs(2)).unwrap_or(false)
    })
}
