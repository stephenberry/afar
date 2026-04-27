//! `afar` — operate shells from afar.
//!
//! `afar` adds working backends to elegance's `MultiTerminal`: a local
//! shell over a PTY, and remote sessions over SSH. The widget you actually
//! show in your egui app is [`MultiTerminal`] (re-exported here), a thin
//! wrapper that owns one or more backends and pumps bytes between them and
//! the existing line-buffered scrollback model.
//!
//! When both crates are imported, alias one to keep them straight, e.g.
//! `use elegance::MultiTerminal as ElegMultiTerminal;`. The presentational
//! one is documented at
//! <https://docs.rs/egui-elegance/latest/elegance/struct.MultiTerminal.html>.
//!
//! See `terminal_crate_plan.md` in the repo root for the full design.

pub mod ansi;
pub mod backend;
pub mod runtime;

mod multi_terminal;

pub use bytes::Bytes;

pub use backend::{
    spawn_backend, BackendEvent, BackendHandle, CloseReason, TerminalBackend, TerminalStatus,
};
pub use multi_terminal::{MultiTerminal, MultiTerminalControls, OverflowPolicy};

#[cfg(feature = "local")]
pub use backend::local::LocalShell;

#[cfg(feature = "ssh")]
pub use backend::ssh::{
    Decision, HostKeyMismatch, HostKeyPolicy, MismatchCallback, SshAuth, SshConfig, SshSession,
};
