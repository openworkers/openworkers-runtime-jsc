//! With the JIT and no polling traps, the check of the CPU limit refuses the
//! worker. A test binary of its own: JSC reads its options once.

#![cfg(feature = "static")]

use openworkers_core::{Script, TerminationReason};
use openworkers_runtime_jsc::Worker;

#[tokio::test]
async fn a_worker_is_refused_without_polling_traps() {
    // SAFETY: the one test of this binary; no other thread reads the
    // environment yet
    unsafe { std::env::remove_var("JSC_usePollingTraps") };

    let made = Worker::new(Script::new("addEventListener('fetch', () => {});"), None).await;

    assert!(
        matches!(made, Err(TerminationReason::InitializationError(_))),
        "{:?}",
        made.err()
    );
}
