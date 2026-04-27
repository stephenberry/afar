//! SSH backend using `russh`. See §7 of `terminal_crate_plan.md`.
//!
//! Implementation is M2 / M2.5 work; this file declares the public types.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Authentication method for an SSH session.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Clone, Debug)]
pub enum SshAuth {
    /// Walk `SSH_AUTH_SOCK` identities. The default.
    Agent,
    /// Read a private key from `path`. Passphrase is supplied at connect
    /// time; the crate never persists it.
    Key {
        path: PathBuf,
        passphrase: Option<String>,
    },
    /// Password authentication. Feature-gated behind `ssh-password`; we
    /// deliberately do not make this the easy path.
    #[cfg(feature = "ssh-password")]
    Password(String),
}

/// Policy for verifying the server's host key.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Clone, Debug)]
pub enum HostKeyPolicy {
    /// Reject any host whose key isn't already in `~/.ssh/known_hosts`.
    StrictKnownHosts,
    /// On first connect, store the fingerprint at `path`. Reject on
    /// subsequent mismatch unless a [`MismatchCallback`] approves.
    TofuStore { path: PathBuf },
    /// Accept any key. Test-only: `cfg`-gated so it cannot be selected
    /// from a release build without explicit opt-in.
    #[cfg(any(test, feature = "ssh-insecure"))]
    AcceptAny,
}

/// Information passed to a [`MismatchCallback`] when a stored key differs
/// from the one the server presented.
#[derive(Clone, Debug)]
pub struct HostKeyMismatch {
    pub host: String,
    pub stored_fingerprint: String,
    pub presented_fingerprint: String,
    pub algorithm: String,
}

/// Caller decision for a host-key mismatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Close the connection. The default if no callback is set.
    Reject,
    /// Proceed for this session, do not update the store.
    AcceptOnce,
    /// Proceed and overwrite the stored fingerprint.
    AcceptAndStore,
}

/// Callback the host app provides to handle a host-key mismatch.
pub type MismatchCallback =
    Arc<dyn Fn(HostKeyMismatch) -> Decision + Send + Sync + 'static>;

/// SSH session configuration.
#[derive(Clone)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
    pub host_key: HostKeyPolicy,
    /// If set, called on host-key mismatch. Without one the connection
    /// fails closed.
    pub on_mismatch: Option<MismatchCallback>,
    pub keepalive: Option<Duration>,
    pub connect_timeout: Duration,
    pub agent_forward: bool,
    pub env: Vec<(String, String)>,
    /// `TERM` value sent to the server. Default `"xterm-256color"`.
    pub term: String,
    /// Allocate a PTY (default). Use [`SshConfig::no_pty`] for one-shot
    /// `exec`-channel semantics.
    pub allocate_pty: bool,
}

impl SshConfig {
    pub fn for_host(host: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 22,
            user: user.into(),
            auth: SshAuth::Agent,
            host_key: HostKeyPolicy::StrictKnownHosts,
            on_mismatch: None,
            keepalive: Some(Duration::from_secs(30)),
            connect_timeout: Duration::from_secs(10),
            agent_forward: false,
            env: Vec::new(),
            term: "xterm-256color".into(),
            allocate_pty: true,
        }
    }

    pub fn no_pty(mut self) -> Self {
        self.allocate_pty = false;
        self
    }
}

impl Default for SshConfig {
    fn default() -> Self {
        Self::for_host(String::new(), "root")
    }
}

/// SSH session backend. M2: implement using `russh::client::connect` and a
/// per-session task that runs the channel loop.
pub struct SshSession {
    _config: SshConfig,
}

impl SshSession {
    pub fn connect(config: SshConfig) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "SshSession::connect: M2 — see terminal_crate_plan.md",
        ))?;
        #[allow(unreachable_code)]
        Ok(Self { _config: config })
    }
}
