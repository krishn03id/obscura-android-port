//! Process-wide async lock that serializes JS work.
//!
//! In the V8/deno_core build this was critical because V8 requires only one
//! Isolate entered per OS thread. QuickJS is single-threaded per runtime, so
//! the lock is less critical — but the CDP dispatcher still uses it to
//! serialize commands, and keeping it preserves the existing call sites.

use std::sync::OnceLock;
use tokio::sync::Mutex;

static JS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Returns the process-wide JS serialization lock.
pub fn global() -> &'static Mutex<()> {
    JS_LOCK.get_or_init(|| Mutex::new(()))
}
