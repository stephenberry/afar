//! Integration tests for `MultiTerminal::pump`.
//!
//! Drives a `MockBackend`-backed pane through the full pipeline (spawn
//! task → BackendEvent channels → ANSI handler → MultiTerminal scrollback)
//! and asserts the rendered state.

use std::time::{Duration, Instant};

use afar::backend::mock::MockBackend;
use afar::MultiTerminal;
use elegance::{LineKind, TerminalPane, TerminalStatus};

/// Drive `pump()` in a polling loop until `condition` holds or we hit a
/// 2-second deadline. The per-session task delivers events on the singleton
/// runtime; the test thread sleeps briefly between pumps so the runtime
/// gets cycles to produce them.
fn pump_until(term: &mut MultiTerminal, condition: impl Fn(&MultiTerminal) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        term.pump();
        if condition(term) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("pump_until: condition not satisfied within deadline");
}

#[test]
fn bytes_flow_through_ansi_handler_into_pane_lines() {
    let mock = MockBackend::new(vec![b"\x1b[31merror: boom\n".to_vec()]);
    let mut term = MultiTerminal::new("test");
    term.add_pane(TerminalPane::new("p1", "host"), mock)
        .unwrap();

    pump_until(&mut term, |t| {
        t.pane("p1").is_some_and(|p| !p.lines.is_empty())
    });

    let pane = term.pane("p1").expect("pane added");
    let first = pane
        .lines
        .iter()
        .find(|l| l.text == "error: boom")
        .expect("expected the SGR-coloured line in scrollback");
    assert_eq!(first.kind, LineKind::Err);
}

#[test]
fn closed_event_sets_status_offline_and_emits_dim_marker() {
    let mock = MockBackend::new(vec![b"hi\n".to_vec()]);
    let mut term = MultiTerminal::new("test");
    term.add_pane(TerminalPane::new("p2", "host"), mock)
        .unwrap();

    pump_until(&mut term, |t| {
        t.pane("p2")
            .is_some_and(|p| p.status == TerminalStatus::Offline)
    });

    let pane = term.pane("p2").expect("pane added");
    assert_eq!(pane.status, TerminalStatus::Offline);
    assert!(
        pane.lines.iter().any(|l| l.text.contains("remote closed")),
        "expected [remote closed] divider; lines: {:?}",
        pane.lines.iter().map(|l| &l.text).collect::<Vec<_>>()
    );
}

#[test]
fn multiple_panes_are_drained_independently() {
    let mock_a = MockBackend::new(vec![b"alpha\n".to_vec()]);
    let mock_b = MockBackend::new(vec![b"beta\n".to_vec()]);
    let mut term = MultiTerminal::new("test");
    term.add_pane(TerminalPane::new("a", "host-a"), mock_a)
        .unwrap();
    term.add_pane(TerminalPane::new("b", "host-b"), mock_b)
        .unwrap();

    pump_until(&mut term, |t| {
        t.pane("a").is_some_and(|p| !p.lines.is_empty())
            && t.pane("b").is_some_and(|p| !p.lines.is_empty())
    });

    assert!(term
        .pane("a")
        .unwrap()
        .lines
        .iter()
        .any(|l| l.text == "alpha"));
    assert!(term
        .pane("b")
        .unwrap()
        .lines
        .iter()
        .any(|l| l.text == "beta"));
}
