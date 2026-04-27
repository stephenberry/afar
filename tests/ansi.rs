//! ANSI handler tests: SGR mapper and CSI/OSC stripper.
//!
//! See §8 of `terminal_crate_plan.md`.

use afar::ansi::{AnsiHandler, DEFAULT_LINE_CAP};
use elegance::LineKind;

fn feed(handler: &mut AnsiHandler, bytes: &[u8]) -> Vec<(LineKind, String)> {
    handler
        .feed(bytes)
        .into_iter()
        .map(|line| (line.kind, line.text))
        .collect()
}

#[test]
fn sgr_red_maps_to_err() {
    let mut h = AnsiHandler::new();
    let lines = feed(&mut h, b"\x1b[31merror: foo\n");
    assert_eq!(lines, vec![(LineKind::Err, "error: foo".into())]);
}

#[test]
fn sgr_green_maps_to_ok_and_yellow_to_warn() {
    let mut h = AnsiHandler::new();
    let lines = feed(&mut h, b"\x1b[32mtest passed\n\x1b[33mwarning: x\n");
    assert_eq!(
        lines,
        vec![
            (LineKind::Ok, "test passed".into()),
            (LineKind::Warn, "warning: x".into()),
        ]
    );
}

#[test]
fn first_non_default_colour_wins() {
    // §8.1: the first non-default SGR colour seen on a line owns the
    // line's LineKind, even if a reset and a different colour follow.
    let mut h = AnsiHandler::new();
    let lines = feed(&mut h, b"\x1b[31merror[E0277]:\x1b[0m default text\n");
    assert_eq!(
        lines,
        vec![(LineKind::Err, "error[E0277]: default text".into())]
    );
}

#[test]
fn second_colour_does_not_override_first() {
    // Two non-default colours on one line: first one wins.
    let mut h = AnsiHandler::new();
    let lines = feed(&mut h, b"\x1b[31mred \x1b[32mthen green\n");
    assert_eq!(lines, vec![(LineKind::Err, "red then green".into())]);
}

#[test]
fn no_colour_means_out() {
    let mut h = AnsiHandler::new();
    let lines = feed(&mut h, b"plain output\n");
    assert_eq!(lines, vec![(LineKind::Out, "plain output".into())]);
}

#[test]
fn pending_kind_resets_on_newline() {
    // Line 1 should be Err; line 2 is plain text with no SGR change of
    // its own and must come back as Out (pending_kind resets per line).
    let mut h = AnsiHandler::new();
    let lines = feed(&mut h, b"\x1b[31mfirst\nsecond\n");
    assert_eq!(
        lines,
        vec![
            (LineKind::Err, "first".into()),
            (LineKind::Out, "second".into()),
        ]
    );
}

#[test]
fn cursor_positioning_is_dropped() {
    // CUP, ED, EL, alt-screen DECSET — all stripped silently. The
    // surrounding text passes through.
    let mut h = AnsiHandler::new();
    let lines = feed(&mut h, b"\x1b[2J\x1b[H\x1b[?1049hhello\x1b[?1049lworld\n");
    assert_eq!(lines, vec![(LineKind::Out, "helloworld".into())]);
}

#[test]
fn osc_52_clipboard_is_dropped() {
    // OSC 52 with base64 payload, BEL-terminated, sandwiched between
    // visible text. No clipboard interaction; payload disappears.
    let mut h = AnsiHandler::new();
    let lines = feed(&mut h, b"prefix \x1b]52;c;ZmFrZWNyZWRz\x07suffix\n");
    assert_eq!(lines, vec![(LineKind::Out, "prefix suffix".into())]);
}

#[test]
fn osc_0_title_is_dropped() {
    // OSC 0 (icon and window title), ST-terminated.
    let mut h = AnsiHandler::new();
    let lines = feed(&mut h, b"\x1b]0;my title\x1b\\hello\n");
    assert_eq!(lines, vec![(LineKind::Out, "hello".into())]);
}

#[test]
fn carriage_return_is_dropped_so_crlf_is_one_line() {
    let mut h = AnsiHandler::new();
    let lines = feed(&mut h, b"line one\r\nline two\r\n");
    assert_eq!(
        lines,
        vec![
            (LineKind::Out, "line one".into()),
            (LineKind::Out, "line two".into()),
        ]
    );
}

#[test]
fn long_lines_are_force_split_at_64_kib() {
    // Build a line of 70 KiB followed by a newline. Expect at least two
    // emissions: the first force-split at the cap, the rest as a final line.
    let mut h = AnsiHandler::new();
    let mut input = vec![b'x'; 70 * 1024];
    input.push(b'\n');
    let lines = h.feed(&input);

    assert!(
        lines.len() >= 2,
        "expected force-split, got {} lines",
        lines.len()
    );
    assert_eq!(lines[0].text.len(), DEFAULT_LINE_CAP);
    let total: usize = lines.iter().map(|l| l.text.len()).sum();
    assert_eq!(total, 70 * 1024);
}

#[test]
fn force_split_preserves_kind_across_continuation() {
    // A line that's all red, longer than the cap, should produce all-red
    // segments because pending_kind survives force-splits.
    let mut h = AnsiHandler::with_line_cap(16);
    let lines = h.feed(b"\x1b[31maaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n");
    assert!(lines.iter().all(|l| l.kind == LineKind::Err));
}

#[test]
fn partial_line_buffered_until_newline() {
    // No newline yet -> nothing emitted, but flush() returns the partial.
    let mut h = AnsiHandler::new();
    let lines = h.feed(b"\x1b[33mpartial");
    assert!(lines.is_empty());
    let trailing = h.flush().expect("partial line flushed");
    assert_eq!(trailing.kind, LineKind::Warn);
    assert_eq!(trailing.text, "partial");
}

#[test]
fn sgr_split_across_feeds_is_handled() {
    // vte::Parser is byte-stream based, so an SGR introducer split across
    // two reads must still be parsed correctly.
    let mut h = AnsiHandler::new();
    assert!(h.feed(b"\x1b[3").is_empty());
    let lines = feed(&mut h, b"1mhello\n");
    assert_eq!(lines, vec![(LineKind::Err, "hello".into())]);
}

#[test]
fn truecolor_red_maps_to_err() {
    // 38;2;255;0;0 is pure red.
    let mut h = AnsiHandler::new();
    let lines = feed(&mut h, b"\x1b[38;2;255;0;0mscarlet\n");
    assert_eq!(lines, vec![(LineKind::Err, "scarlet".into())]);
}

#[test]
fn xterm256_yellow_maps_to_warn() {
    // Index 226 in the 256-colour palette is bright yellow.
    let mut h = AnsiHandler::new();
    let lines = feed(&mut h, b"\x1b[38;5;226mhighlight\n");
    assert_eq!(lines, vec![(LineKind::Warn, "highlight".into())]);
}

#[test]
fn empty_sgr_resets_without_clearing_pending_kind() {
    // `CSI m` is equivalent to `CSI 0 m` (reset). Per "first non-default
    // wins", a reset should not clear an already-set pending_kind.
    let mut h = AnsiHandler::new();
    let lines = feed(&mut h, b"\x1b[31mred\x1b[mstill red\n");
    assert_eq!(lines, vec![(LineKind::Err, "redstill red".into())]);
}
