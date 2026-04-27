//! Backend trait and the UI-facing facade.
//!
//! See §6 of `terminal_crate_plan.md` for the full design.

use std::io;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::runtime::Runtime;

/// Per-poll read buffer size. Sized to amortise context switches without
/// holding back UI repaints under heavy output.
const READ_BUF_SIZE: usize = 4096;

/// Bound on each direction's mpsc channel. Inputs queue when the per-session
/// task is busy writing; events queue when the UI hasn't pulled this frame yet.
/// Bounded so a flooded shell can't OOM the app.
const CHANNEL_CAP: usize = 64;

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
pub(crate) enum UiToBackend {
    Input(Bytes),
    Resize { cols: u16, rows: u16 },
    Shutdown,
}

/// UI-facing facade for a backend session.
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
        TerminalStatus::from_u8(self.status.load(Ordering::Relaxed))
    }

    /// Forward typed bytes to the backend. Non-blocking; if the per-session
    /// task is full, returns the bytes back so the caller can decide whether
    /// to drop, retry, or render an `[input dropped]` marker.
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

    /// Tell the backend its window changed. Best-effort; resize is dropped
    /// silently if the channel is full because the next frame will replace it.
    pub fn try_resize(&self, cols: u16, rows: u16) {
        let _ = self.tx.try_send(UiToBackend::Resize { cols, rows });
    }

    /// Request graceful shutdown. The session task will close the backend
    /// and emit [`BackendEvent::Closed`] before exiting.
    pub fn shutdown(&self) {
        let _ = self.tx.try_send(UiToBackend::Shutdown);
    }

    /// Drain the next event without blocking. The UI render loop calls this
    /// in a `while let Ok(...)` until [`mpsc::error::TryRecvError::Empty`].
    pub fn try_recv(&mut self) -> Result<BackendEvent, mpsc::error::TryRecvError> {
        self.rx.try_recv()
    }

    /// Await the next event. Returns `None` when the session has ended and
    /// no further events will arrive. Most call sites want [`Self::try_recv`];
    /// this is for tests and async contexts that want backpressure.
    pub async fn recv(&mut self) -> Option<BackendEvent> {
        self.rx.recv().await
    }
}

/// Spawn a session task on the singleton runtime that pumps bytes between
/// `backend` and the returned [`BackendHandle`].
///
/// The task runs until: the UI sends [`BackendHandle::shutdown`], the UI
/// drops its handle, the backend signals EOF, or a transport error occurs.
/// In every case a final [`BackendEvent::Closed`] is sent before the task
/// exits, with an updated [`TerminalStatus`] reflected on the cheap-poll
/// atomic.
pub fn spawn_backend<B: TerminalBackend>(backend: B) -> io::Result<BackendHandle> {
    let runtime = crate::runtime::get_or_init()?;

    let (ui_tx, ui_rx) = mpsc::channel::<UiToBackend>(CHANNEL_CAP);
    let (event_tx, event_rx) = mpsc::channel::<BackendEvent>(CHANNEL_CAP);
    let status = Arc::new(AtomicU8::new(TerminalStatus::Connected as u8));

    let status_for_task = Arc::clone(&status);
    runtime.spawn(async move {
        run_session(backend, ui_rx, event_tx, status_for_task).await;
    });

    Ok(BackendHandle {
        tx: ui_tx,
        rx: event_rx,
        status,
        _runtime: runtime,
    })
}

/// The per-session loop. Concurrently:
///
/// - reads bytes from the backend (forwarded as [`BackendEvent::Bytes`]),
/// - drains [`UiToBackend`] messages and applies them to the backend.
///
/// `tokio::select!` is `biased` so UI commands take precedence when both
/// branches are ready; this matters for `Shutdown`, which should beat a
/// pending read.
async fn run_session<B: TerminalBackend>(
    mut backend: B,
    mut ui_rx: mpsc::Receiver<UiToBackend>,
    event_tx: mpsc::Sender<BackendEvent>,
    status: Arc<AtomicU8>,
) {
    let mut buf = vec![0u8; READ_BUF_SIZE];

    let close_reason = loop {
        tokio::select! {
            biased;

            ui_msg = ui_rx.recv() => {
                match ui_msg {
                    Some(UiToBackend::Input(bytes)) => {
                        if let Err(e) = backend.write_all(&bytes).await {
                            break CloseReason::TransportError(e.to_string());
                        }
                    }
                    Some(UiToBackend::Resize { cols, rows }) => {
                        if let Err(e) = backend.resize(cols, rows) {
                            break CloseReason::TransportError(e.to_string());
                        }
                    }
                    Some(UiToBackend::Shutdown) => {
                        let _ = backend.shutdown().await;
                        break CloseReason::Requested;
                    }
                    None => {
                        // UI dropped its handle; treat as a graceful close.
                        let _ = backend.shutdown().await;
                        break CloseReason::Requested;
                    }
                }
            }

            n = backend.read(&mut buf) => {
                match n {
                    Ok(0) => break CloseReason::RemoteClosed,
                    Ok(n) => {
                        let bytes = Bytes::copy_from_slice(&buf[..n]);
                        if event_tx.send(BackendEvent::Bytes(bytes)).await.is_err() {
                            // UI dropped its receiver; nothing more to do.
                            return;
                        }
                    }
                    Err(e) => break CloseReason::TransportError(e.to_string()),
                }
            }
        }
    };

    // Every termination path reports Offline + a final Closed event so the
    // UI's status pill and `try_recv` consumers see consistent shutdown.
    status.store(TerminalStatus::Offline as u8, Ordering::Relaxed);
    let _ = event_tx
        .send(BackendEvent::StatusChanged(TerminalStatus::Offline))
        .await;
    let _ = event_tx
        .send(BackendEvent::Closed {
            reason: close_reason,
        })
        .await;
}
