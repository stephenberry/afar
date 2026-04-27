//! Local PTY backend, spawning the user's `$SHELL` via `portable-pty`.
//!
//! See §5 / §7 of `terminal_crate_plan.md`. Implementation is M1 work.

use std::io;

/// Configuration for a local shell session.
#[derive(Clone, Debug)]
pub struct LocalShellConfig {
    /// Command to spawn. Defaults to `$SHELL`, falling back to `/bin/sh`
    /// (Unix) or `cmd.exe` (Windows).
    pub program: Option<String>,
    /// Arguments. Empty for an interactive shell.
    pub args: Vec<String>,
    /// Environment overrides. Inherited otherwise.
    pub env: Vec<(String, String)>,
    /// Initial PTY size, in cols/rows. Reset by [`super::TerminalBackend::resize`].
    pub initial_size: (u16, u16),
}

impl Default for LocalShellConfig {
    fn default() -> Self {
        Self {
            program: None,
            args: Vec::new(),
            env: Vec::new(),
            initial_size: (80, 24),
        }
    }
}

/// Local shell backend. M1: implement using `portable_pty::native_pty_system`
/// and a tokio task that copies bytes between the PTY master fds and the
/// `BackendHandle` channels.
pub struct LocalShell {
    _config: LocalShellConfig,
}

impl LocalShell {
    pub fn new(config: LocalShellConfig) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "LocalShell::new: M1 — see terminal_crate_plan.md",
        ))?;
        #[allow(unreachable_code)]
        Ok(Self { _config: config })
    }
}
