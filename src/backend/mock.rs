//! `MockBackend` for tests. Replays scripted byte streams back to the UI
//! and records bytes the widget sent. Used by `tests/ssh_mock.rs` and the
//! ANSI handler tests.

use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::TerminalBackend;

/// Backend that produces a scripted byte stream and captures whatever the
/// widget writes. Resize calls are recorded but otherwise ignored.
pub struct MockBackend {
    pub script: VecDeque<Vec<u8>>,
    pub written: Vec<u8>,
    pub resizes: Vec<(u16, u16)>,
}

impl MockBackend {
    pub fn new<I>(script: I) -> Self
    where
        I: IntoIterator<Item = Vec<u8>>,
    {
        Self {
            script: script.into_iter().collect(),
            written: Vec::new(),
            resizes: Vec::new(),
        }
    }
}

impl AsyncRead for MockBackend {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Some(chunk) = self.script.pop_front() {
            let n = chunk.len().min(buf.remaining());
            buf.put_slice(&chunk[..n]);
            if n < chunk.len() {
                self.script.push_front(chunk[n..].to_vec());
            }
        }
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for MockBackend {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.written.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl TerminalBackend for MockBackend {
    fn resize(&mut self, cols: u16, rows: u16) -> io::Result<()> {
        self.resizes.push((cols, rows));
        Ok(())
    }
}
