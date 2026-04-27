//! End-to-end round-trip tests for `spawn_backend`.
//!
//! Drive a `MockBackend` through the per-session task and assert the events
//! the UI receives, the bytes the backend was asked to write, and the
//! resize/shutdown plumbing. Pure data-in-data-out; no SSH, no PTY.

use std::future::Future;
use std::time::Duration;

use afar::backend::mock::MockBackend;
use afar::{spawn_backend, BackendEvent, BackendHandle, CloseReason, TerminalStatus};

/// Run an async block to completion under a current-thread test runtime,
/// timing out so a hung session task can't wedge CI for the full default.
///
/// Note on runtime drop: tokio panics if a `Runtime` is dropped inside
/// another runtime's executor context. Tests that hold a `BackendHandle`
/// (which keeps the library's singleton runtime alive) must therefore
/// return the handle out of `block_on` and drop it in sync context.
fn run<F: Future>(future: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    rt.block_on(async move {
        tokio::time::timeout(Duration::from_secs(5), future)
            .await
            .expect("test timed out")
    })
}

#[test]
fn round_trip_bytes_and_remote_eof() {
    let (handle, received, closed, last_status) = run(async {
        let mock = MockBackend::new(vec![b"hello".to_vec(), b" world\n".to_vec()]);
        let mut handle: BackendHandle = spawn_backend(mock).expect("spawn_backend");

        let mut received = Vec::new();
        let mut closed: Option<CloseReason> = None;
        let mut last_status: Option<TerminalStatus> = None;

        while let Some(event) = handle.recv().await {
            match event {
                BackendEvent::Bytes(b) => received.extend_from_slice(&b),
                BackendEvent::StatusChanged(s) => last_status = Some(s),
                BackendEvent::Closed { reason } => {
                    closed = Some(reason);
                    break;
                }
                BackendEvent::Lossy { .. } | BackendEvent::InputLost { .. } => {}
            }
        }

        (handle, received, closed, last_status)
    });

    // Sync context: dropping `handle` here releases the singleton runtime
    // safely, since we're not inside any executor.
    let final_status = handle.status();
    drop(handle);

    assert_eq!(received, b"hello world\n");
    assert!(matches!(closed, Some(CloseReason::RemoteClosed)));
    assert_eq!(last_status, Some(TerminalStatus::Offline));
    assert_eq!(final_status, TerminalStatus::Offline);
}

#[test]
fn shutdown_request_closes_cleanly() {
    let (handle, closed) = run(async {
        // Empty script: backend yields EOF on first read. We race a Shutdown
        // request to verify either path produces a clean Closed event.
        let mock = MockBackend::new(Vec::<Vec<u8>>::new());
        let mut handle: BackendHandle = spawn_backend(mock).expect("spawn_backend");

        handle.shutdown();

        let mut closed: Option<CloseReason> = None;
        while let Some(event) = handle.recv().await {
            if let BackendEvent::Closed { reason } = event {
                closed = Some(reason);
                break;
            }
        }

        (handle, closed)
    });

    drop(handle);

    // Either Shutdown beat the EOF (Requested) or the read won (RemoteClosed).
    // Both are valid clean shutdowns; the contract is that exactly one Closed
    // event is emitted.
    assert!(matches!(
        closed,
        Some(CloseReason::Requested) | Some(CloseReason::RemoteClosed)
    ));
}
