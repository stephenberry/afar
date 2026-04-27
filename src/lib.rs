//! `afar` — operate shells from afar.
//!
//! `afar` adds working backends to [`elegance::MultiTerminal`]: a local
//! shell over a PTY, and remote sessions over SSH. The widget you actually
//! show in your egui app is [`LiveMultiTerminal`], a thin wrapper that owns
//! one or more backends and pumps bytes between them and the existing
//! line-buffered scrollback model.
//!
//! [`elegance::MultiTerminal`]: https://docs.rs/egui-elegance/latest/elegance/struct.MultiTerminal.html
//!
//! See `terminal_crate_plan.md` in the repo root for the full design.

pub mod ansi;
pub mod backend;
pub mod runtime;

mod live_multi;

pub use bytes::Bytes;

pub use backend::{
    spawn_backend, BackendEvent, BackendHandle, CloseReason, TerminalBackend, TerminalStatus,
};
pub use live_multi::{LiveMultiTerminal, LiveMultiTerminalControls, OverflowPolicy};

#[cfg(feature = "local")]
pub use backend::local::LocalShell;

#[cfg(feature = "ssh")]
pub use backend::ssh::{
    Decision, HostKeyMismatch, HostKeyPolicy, MismatchCallback, SshAuth, SshConfig, SshSession,
};
