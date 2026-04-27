//! Process-singleton tokio runtime owned by the crate.
//!
//! See §9.1 of `terminal_crate_plan.md`. The runtime is held behind a
//! `Mutex<Weak<Runtime>>`: the first backend spawn finds the `Weak` empty
//! and constructs a fresh `Runtime`; subsequent spawns find it upgradable
//! and clone the existing `Arc`. When the last `BackendHandle` drops, the
//! `Arc` count goes to zero and the runtime shuts down. A subsequent
//! spawn re-initialises cleanly.

use std::sync::{Arc, Mutex, Weak};

pub use tokio::runtime::Runtime;

static RUNTIME: Mutex<Weak<Runtime>> = Mutex::new(Weak::new());

/// Get the process-singleton runtime, constructing it on first call after
/// last drop. Each [`crate::BackendHandle`] holds one of these `Arc`s for
/// the lifetime of its session, so the runtime stays alive while any
/// backend is live.
pub fn get_or_init() -> std::io::Result<Arc<Runtime>> {
    let mut guard = RUNTIME.lock().expect("afar runtime mutex poisoned");
    if let Some(rt) = guard.upgrade() {
        return Ok(rt);
    }
    let rt = Arc::new(Runtime::new()?);
    *guard = Arc::downgrade(&rt);
    Ok(rt)
}

/// Adopt an externally-owned tokio runtime. The host app retains
/// responsibility for the lifecycle; the singleton is bypassed.
///
/// Implementation: M0/M1. The handle is plumbed through to per-session
/// task spawns instead of `get_or_init().spawn(...)`.
pub fn with_runtime(_handle: tokio::runtime::Handle) {
    todo!("with_runtime: see terminal_crate_plan.md §9.1")
}
