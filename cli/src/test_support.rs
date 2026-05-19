//! Test-only helpers for serializing `$HOME` / env-var mutations across
//! test modules. Cargo's default test runner is multi-threaded, so any
//! two tests that override `HOME` (we have several across `auth`,
//! `api`, and `telemetry::consent`) must share a single mutex or they
//! race. Using one shared `OnceLock<Mutex<()>>` here ensures that.

use std::sync::{Mutex, OnceLock};

/// Return the process-global env-mutation lock. Acquire this for the
/// entire duration of a test that calls `std::env::set_var("HOME", …)`
/// (or similar env-var manipulation) before doing file-system IO that
/// resolves relative to `$HOME`.
pub fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
