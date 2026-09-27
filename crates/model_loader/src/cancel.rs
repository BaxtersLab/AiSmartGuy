//! Process-wide cancellation flag.
//!
//! A run is a strictly sequential fold — one llama.cpp child process at a
//! time — so a single global flag is an honest model of what the pipeline
//! actually is. Every `ModelInstance` shares this flag (via [`flag`]) so a
//! cancel request set through any handle (e.g. the UI's Terminate Run
//! button) is visible to whichever instance's inference loop is currently
//! polling it, and to the pre-spawn check in `run_inference`.
//!
//! Re-applied 2026-08-17: this module was documented as already shipped in
//! `handoffs.md`'s 2026-08-04 entry, but was absent from the source tree
//! mirrored to this box — `ModelInstance::new` was still minting a fresh
//! `AtomicBool` per instance, and `cmd_cancel_run` was setting a flag nothing
//! read. See `crates/model_loader/src/types.rs::regression_tests` for the
//! test that reproduces the bug against the un-fixed code.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

static CANCEL_FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();

/// Returns the single process-wide cancel flag, creating it on first call.
/// Every caller gets a clone of the SAME `Arc` — that sharing is the fix.
pub fn flag() -> Arc<AtomicBool> {
    CANCEL_FLAG
        .get_or_init(|| Arc::new(AtomicBool::new(false)))
        .clone()
}

/// Request cancellation of the current (or next) inference.
pub fn request() {
    flag().store(true, Ordering::SeqCst);
}

/// Clear a pending cancellation request.
///
/// Must be called at the start of every run — otherwise a cancelled run
/// leaves the flag set and the *next* run dies instantly with no
/// explanation.
pub fn clear() {
    flag().store(false, Ordering::SeqCst);
}

/// True if cancellation has been requested and not yet cleared.
pub fn is_requested() -> bool {
    flag().load(Ordering::SeqCst)
}

/// Serializes every test in this crate that touches the flag. CANCEL_FLAG is
/// genuine process-wide global state (that's the whole point), so tests in
/// different modules race unless they all hold this — a lock private to one
/// test module left `types::regression_tests` free to flip the flag mid-test.
/// Poison-tolerant, so one failing test cannot fail the others.
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_and_clear_round_trip() {
        let _guard = test_lock();
        clear();
        assert!(!is_requested());
        request();
        assert!(is_requested());
        clear();
        assert!(!is_requested());
    }

    /// The exact property the pre-fix code lacked: a value set through one
    /// handle is visible through another.
    #[test]
    fn value_set_through_one_handle_is_visible_through_another() {
        let _guard = test_lock();
        clear();
        let handle_a = flag();
        let handle_b = flag();
        assert!(
            Arc::ptr_eq(&handle_a, &handle_b),
            "flag() must return the same shared Arc on every call"
        );
        handle_a.store(true, Ordering::SeqCst);
        assert!(
            handle_b.load(Ordering::SeqCst),
            "a value set through one handle must be visible through another"
        );
        clear();
    }
}
