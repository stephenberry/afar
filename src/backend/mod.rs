//! Backend trait and the UI-facing facade.
//!
//! See §6 of `terminal_crate_plan.md` for the full design.

use std::io;
use std::sync::atomic::AtomicU8;
use std::sync::Arc;

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use crate::runtime::Runtime;

#[cfg(feature = "local")]
pub mod local;
#[cfg(feature = "ssh")]
pub mod ssh;

pub mod mock;

/// Connection status for a backend, mirrored into the elegance pane header.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalStatus {
    Connected = 0,
    Reconnecting = 1,
    Offline = 2,
}

impl TerminalStatus {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Connected,
            1 => Self::Reconnecting,
            _ => Self::Offline,
        }
    }
}

/// A bidirectional byte stream tied to a remote or local shell, plus the
/// orthogonal control operations a tty needs.
///
/// Implementations are owned by a per-session task on the runtime; the UI
/// side never holds a `TerminalBackend` directly, it works through
/// [`BackendHandle`].
pub trait TerminalBackend: AsyncRead + AsyncWrite + Send + Unpin + 'static {
    /// PTY size changed. Mapped to `ioctl(TIOCSWINSZ)` on local PTYs and to
    /// a `window-change` channel request on SSH sessions. Backends without
    /// a PTY may return `Ok(())` and ignore.
    fn resize(&mut self, cols: u16, rows: u16) -> io::Result<()>;
}

/// Reason a backend session ended.
#[derive(Clone, Debug)]
pub enum CloseReason {
    /// Caller requested shutdown.
    Requested,
    /// Remote end closed cleanly.
    RemoteClosed,
    /// Transport error after exhausting retries.
    TransportError(String),
    /// Auth failed, no retry will help.
    AuthFailed(String),
}

/// Event flowing from the runtime task to the UI.
#[derive(Clone, Debug)]
pub enum BackendEvent {
    /// Output bytes from the remote.
    Bytes(Bytes),
    /// Authoritative status change.
    StatusChanged(TerminalStatus),
    /// Output ring overflowed and `dropped` bytes were discarded.
    /// Default policy is drop-newest at the truncation point.
    Lossy { dropped: usize },
    /// Pending input was discarded on reconnect (§7.6).
    InputLost { dropped: usize },
    /// Session ended; no more events will follow.
    Closed { reason: CloseReason },
}

/// Message flowing from the UI to the runtime task.
#[allow(dead_code)] // Fields read by the per-session task in M0.
pub(crate) enum UiToBackend {
    Input(Bytes),
    Resize { cols: u16, rows: u16 },
    Shutdown,
}

/// UI-facing facade for a backend session.
///
/// `Send + Sync` and cheap to clone.
#[allow(dead_code)] // `rx` is drained by `LiveMultiTerminal::show` in M1.
pub struct BackendHandle {
    pub(crate) tx: mpsc::Sender<UiToBackend>,
    pub(crate) rx: mpsc::Receiver<BackendEvent>,
    pub(crate) status: Arc<AtomicU8>,
    pub(crate) _runtime: Arc<Runtime>,
}

impl BackendHandle {
    /// Cheap-poll the current status. Authoritative state changes flow
    /// through [`BackendEvent::StatusChanged`] events on the receiver.
    pub fn status(&self) -> TerminalStatus {
        TerminalStatus::from_u8(self.status.load(std::sync::atomic::Ordering::Relaxed))
    }

    pub fn try_send_input(&self, bytes: Bytes) -> Result<(), mpsc::error::TrySendError<Bytes>> {
        match self.tx.try_send(UiToBackend::Input(bytes)) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(UiToBackend::Input(b))) => {
                Err(mpsc::error::TrySendError::Full(b))
            }
            Err(mpsc::error::TrySendError::Closed(UiToBackend::Input(b))) => {
                Err(mpsc::error::TrySendError::Closed(b))
            }
            Err(_) => unreachable!(),
        }
    }

    pub fn try_resize(&self, cols: u16, rows: u16) {
        let _ = self.tx.try_send(UiToBackend::Resize { cols, rows });
    }

    pub fn shutdown(&self) {
        let _ = self.tx.try_send(UiToBackend::Shutdown);
    }
}
