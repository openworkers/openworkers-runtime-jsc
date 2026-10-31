//! With the JIT and polling traps, the check of the CPU limit passes. A test
//! binary of its own: JSC reads its options once.

#![cfg(feature = "static")]

use openworkers_core::Script;
use openworkers_runtime_jsc::JscOptions;
use openworkers_runtime_jsc::Worker;

#[tokio::test]
async fn a_worker_starts_with_polling_traps() {
    let options = JscOptions {
        polling_traps: true,
    };

    // SAFETY: the one test of this binary; no other thread reads the
    // environment yet
    unsafe { openworkers_runtime_jsc::init_jsc(options) }.unwrap();

    let made = Worker::new(Script::new("addEventListener('fetch', () => {});"), None).await;

    assert!(made.is_ok(), "{:?}", made.err());
}
