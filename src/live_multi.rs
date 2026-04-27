//! `LiveMultiTerminal` — wraps elegance's `MultiTerminal` with backends.
//!
//! See §4 of `terminal_crate_plan.md`. The widget owns one backend per
//! pane, drains `TerminalEvent::Command` from the inner `MultiTerminal`,
//! writes typed bytes to the backend, and pumps stdout/stderr lines back
//! into `push_line` via the [`crate::ansi::AnsiHandler`].

use std::collections::HashMap;
use std::hash::Hash;

use egui::{Response, Ui};
use elegance::MultiTerminal;

use crate::ansi::AnsiHandler;
use crate::backend::BackendHandle;

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

#[allow(dead_code)]
struct Pane {
    backend: BackendHandle,
    ansi: AnsiHandler,
}

/// Multi-pane terminal widget driven by real backends.
///
/// See `terminal_crate_plan.md` §4.
#[allow(dead_code)] // `panes` and `overflow` are wired up in M1.
pub struct LiveMultiTerminal {
    inner: MultiTerminal,
    panes: HashMap<String, Pane>,
    overflow: OverflowPolicy,
}

impl LiveMultiTerminal {
    pub fn new(id_salt: impl Hash) -> Self {
        Self {
            inner: MultiTerminal::new(id_salt),
            panes: HashMap::new(),
            overflow: OverflowPolicy::default(),
        }
    }

    pub fn overflow_policy(mut self, policy: OverflowPolicy) -> Self {
        self.overflow = policy;
        self
    }

    /// Add a pane backed by a local shell. Requires the `local` feature.
    #[cfg(feature = "local")]
    pub fn add_local_pane(
        &mut self,
        _id: impl Into<String>,
        _config: crate::backend::local::LocalShellConfig,
    ) {
        todo!("LiveMultiTerminal::add_local_pane — M1")
    }

    /// Add a pane backed by an SSH session. Requires the `ssh` feature.
    #[cfg(feature = "ssh")]
    pub fn add_ssh_pane(
        &mut self,
        _id: impl Into<String>,
        _config: crate::backend::ssh::SshConfig,
    ) {
        todo!("LiveMultiTerminal::add_ssh_pane — M2")
    }

    /// Restricted view of the underlying [`MultiTerminal`]. Exposes the
    /// safe operations (broadcast, solo, collapse, focus, scrollback
    /// queries); structural mutators are deliberately not reachable so
    /// the wrapper invariant ("every pane has a backend") holds.
    pub fn controls(&mut self) -> LiveMultiTerminalControls<'_> {
        LiveMultiTerminalControls {
            inner: &mut self.inner,
        }
    }

    pub fn show(&mut self, _ui: &mut Ui) -> Response {
        // M1: drain `inner.take_events()`, forward Command events as input
        // bytes to the targeted backends; for each pane, drain BackendEvent
        // from its handle, feed bytes through `ansi`, and push each emitted
        // TerminalLine into `inner`. Then call `inner.show(ui)`.
        todo!("LiveMultiTerminal::show — M1")
    }
}

/// Restricted view of the inner [`MultiTerminal`]. See §4 of the plan.
pub struct LiveMultiTerminalControls<'a> {
    inner: &'a mut MultiTerminal,
}

impl<'a> LiveMultiTerminalControls<'a> {
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
}
