//! `MultiTerminal` — wraps elegance's `MultiTerminal` with backends.
//!
//! See §4 of `terminal_crate_plan.md`. The widget owns one backend per
//! pane, drains `TerminalEvent::Command` from the inner elegance widget,
//! writes typed bytes to the backend, and pumps stdout/stderr lines back
//! into `push_line` via the [`crate::ansi::AnsiHandler`].
//!
//! When both crates are imported, alias one to keep them straight, e.g.
//! `use elegance::MultiTerminal as ElegMultiTerminal;`.

use std::collections::HashMap;
use std::hash::Hash;
use std::io;

use bytes::Bytes;
use egui::{Response, Ui};
use elegance::{TerminalEvent, TerminalLine, TerminalPane};

use crate::ansi::AnsiHandler;
use crate::backend::{
    spawn_backend, BackendEvent, BackendHandle, CloseReason, TerminalBackend, TerminalStatus,
};

/// Policy for what to drop when a pane's output ring buffer fills.
///
/// Default is [`OverflowPolicy::DropNewest`]: when bytes arrive faster
/// than the UI can drain them, the new bytes are discarded and a
/// `[N bytes dropped]` marker is rendered at the truncation point. This
/// preserves the head of the buffer (where build/log streams keep their
/// first error). Switch to `DropOldest` for live-tail scenarios.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverflowPolicy {
    #[default]
    DropNewest,
    DropOldest,
}

struct Pane {
    backend: BackendHandle,
    ansi: AnsiHandler,
}

/// Multi-pane terminal widget driven by real backends. Wraps
/// [`elegance::MultiTerminal`] and owns one [`BackendHandle`] per pane.
///
/// See `terminal_crate_plan.md` §4.
#[allow(dead_code)] // `overflow` is wired up in the spawn-loop hardening pass.
pub struct MultiTerminal {
    inner: elegance::MultiTerminal,
    panes: HashMap<String, Pane>,
    overflow: OverflowPolicy,
}

impl MultiTerminal {
    pub fn new(id_salt: impl Hash) -> Self {
        Self {
            inner: elegance::MultiTerminal::new(id_salt),
            panes: HashMap::new(),
            overflow: OverflowPolicy::default(),
        }
    }

    pub fn overflow_policy(mut self, policy: OverflowPolicy) -> Self {
        self.overflow = policy;
        self
    }

    /// Add a pane with any [`TerminalBackend`] implementation. The backend
    /// is spawned on the singleton runtime; the returned [`BackendHandle`]
    /// is owned by the widget and drained per-frame via [`Self::pump`].
    pub fn add_pane<B: TerminalBackend>(
        &mut self,
        pane: TerminalPane,
        backend: B,
    ) -> io::Result<()> {
        let id = pane.id.clone();
        let handle = spawn_backend(backend)?;
        self.inner.add_pane(pane);
        self.panes.insert(
            id,
            Pane {
                backend: handle,
                ansi: AnsiHandler::new(),
            },
        );
        Ok(())
    }

    /// Add a pane backed by a local shell. Requires the `local` feature.
    #[cfg(feature = "local")]
    pub fn add_local_pane(
        &mut self,
        _id: impl Into<String>,
        _config: crate::backend::local::LocalShellConfig,
    ) -> io::Result<()> {
        todo!("MultiTerminal::add_local_pane — M1 (LocalShell wiring)")
    }

    /// Add a pane backed by an SSH session. Requires the `ssh` feature.
    #[cfg(feature = "ssh")]
    pub fn add_ssh_pane(
        &mut self,
        _id: impl Into<String>,
        _config: crate::backend::ssh::SshConfig,
    ) -> io::Result<()> {
        todo!("MultiTerminal::add_ssh_pane — M2")
    }

    /// Read-only access to a pane's elegance-side state (host, user, cwd,
    /// status, scrollback). Useful for tests and for host apps that want to
    /// query rendered output without going through the `controls()` view.
    pub fn pane(&self, id: &str) -> Option<&TerminalPane> {
        self.inner.pane(id)
    }

    /// Restricted view of the underlying [`elegance::MultiTerminal`].
    /// Exposes the safe operations (broadcast, solo, collapse, focus,
    /// scrollback queries); structural mutators are deliberately not
    /// reachable so the wrapper invariant ("every pane has a backend")
    /// holds.
    pub fn controls(&mut self) -> MultiTerminalControls<'_> {
        MultiTerminalControls {
            inner: &mut self.inner,
        }
    }

    /// Drive the wiring: forward typed commands to backends, drain backend
    /// events, and update the inner widget. Returns `true` if any output
    /// bytes were processed (the caller can use this to request a
    /// repaint).
    ///
    /// Called once per frame from [`Self::show`]. Exposed publicly for
    /// tests that want to drive the pump without rendering.
    pub fn pump(&mut self) -> bool {
        // Forward Command events to backends. Each Command carries the
        // ids of its broadcast targets; the elegance widget has already
        // pushed a Command echo line into each target pane.
        for event in self.inner.take_events() {
            match event {
                TerminalEvent::Command { targets, command } => {
                    let bytes: Bytes = format!("{command}\n").into_bytes().into();
                    for target in targets {
                        if let Some(pane) = self.panes.get(&target) {
                            // Drop on full; the M1 design accepts this and
                            // future hardening adds an [input dropped] marker.
                            let _ = pane.backend.try_send_input(bytes.clone());
                        }
                    }
                }
            }
        }

        // Drain backend events into the inner widget.
        let mut any_bytes = false;
        let inner = &mut self.inner;
        for (id, pane) in self.panes.iter_mut() {
            loop {
                match pane.backend.try_recv() {
                    Ok(BackendEvent::Bytes(bytes)) => {
                        any_bytes = true;
                        for line in pane.ansi.feed(&bytes) {
                            inner.push_line(id, line);
                        }
                    }
                    Ok(BackendEvent::StatusChanged(status)) => {
                        inner.set_status(id, status_to_elegance(status));
                    }
                    Ok(BackendEvent::Lossy { dropped }) => {
                        inner.push_line(
                            id,
                            TerminalLine::warn(format!("[{dropped} bytes dropped]")),
                        );
                    }
                    Ok(BackendEvent::InputLost { dropped }) => {
                        inner.push_line(
                            id,
                            TerminalLine::warn(format!("[input lost: {dropped} bytes]")),
                        );
                    }
                    Ok(BackendEvent::Closed { reason }) => {
                        inner.set_status(id, elegance::TerminalStatus::Offline);
                        inner.push_line(id, TerminalLine::dim(describe_close(&reason)));
                    }
                    Err(_) => break, // Empty or Disconnected; nothing more this frame.
                }
            }
        }

        any_bytes
    }

    /// Render the widget. Drives [`Self::pump`] first to deliver any new
    /// events arrived since the last frame, then asks egui for a repaint
    /// if output was processed (so the next frame picks up further
    /// streaming bytes promptly).
    pub fn show(&mut self, ui: &mut Ui) -> Response {
        if self.pump() {
            ui.ctx().request_repaint();
        }
        self.inner.show(ui)
    }
}

fn status_to_elegance(status: TerminalStatus) -> elegance::TerminalStatus {
    match status {
        TerminalStatus::Connected => elegance::TerminalStatus::Connected,
        TerminalStatus::Reconnecting => elegance::TerminalStatus::Reconnecting,
        TerminalStatus::Offline => elegance::TerminalStatus::Offline,
    }
}

fn describe_close(reason: &CloseReason) -> String {
    match reason {
        CloseReason::Requested => "[session ended]".to_string(),
        CloseReason::RemoteClosed => "[remote closed]".to_string(),
        CloseReason::TransportError(e) => format!("[transport error: {e}]"),
        CloseReason::AuthFailed(e) => format!("[auth failed: {e}]"),
    }
}

/// Restricted view of the inner [`elegance::MultiTerminal`]. See §4 of
/// the plan.
pub struct MultiTerminalControls<'a> {
    inner: &'a mut elegance::MultiTerminal,
}

impl MultiTerminalControls<'_> {
    pub fn toggle_broadcast(&mut self, id: &str) {
        self.inner.toggle_broadcast(id);
    }

    pub fn solo(&mut self, id: &str) {
        self.inner.solo(id);
    }

    pub fn solo_focused(&mut self) {
        self.inner.solo_focused();
    }

    pub fn broadcast_all(&mut self) {
        self.inner.broadcast_all();
    }

    pub fn invert_broadcast(&mut self) {
        self.inner.invert_broadcast();
    }

    pub fn toggle_collapsed(&mut self, id: &str) {
        self.inner.toggle_collapsed(id);
    }

    pub fn collapse_all(&mut self) {
        self.inner.collapse_all();
    }

    pub fn expand_all(&mut self) {
        self.inner.expand_all();
    }

    pub fn focused(&self) -> Option<&str> {
        self.inner.focused()
    }

    pub fn set_focused(&mut self, id: Option<String>) {
        self.inner.set_focused(id);
    }

    pub fn pane(&self, id: &str) -> Option<&TerminalPane> {
        self.inner.pane(id)
    }

    /// Programmatically queue a command, as if the user typed it and
    /// pressed Enter. Useful for automation; returns `true` if the
    /// command was queued (i.e. at least one pane was a broadcast target).
    pub fn send_command(&mut self, cmd: &str) -> bool {
        self.inner.send_command(cmd)
    }
}
