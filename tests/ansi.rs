//! ANSI handler tests: SGR mapper and CSI/OSC stripper.
//!
//! See §11 of `terminal_crate_plan.md`. M1 — these tests should fail with
//! `todo!()` until the handler is implemented.

#[test]
#[ignore = "M1: AnsiHandler not yet implemented"]
fn sgr_red_maps_to_err() {
    // Feed `\x1b[31merror: foo\n` and expect a single `LineKind::Err` line.
}

#[test]
#[ignore = "M1: AnsiHandler not yet implemented"]
fn first_non_default_colour_wins() {
    // §8.1 — given `\x1b[31merror[E0277]:\x1b[0m default text\n`, expect Err.
}

#[test]
#[ignore = "M1: AnsiHandler not yet implemented"]
fn osc_52_clipboard_is_dropped() {
    // OSC 52 must not appear in any emitted line.
}

#[test]
#[ignore = "M1: AnsiHandler not yet implemented"]
fn long_lines_are_force_split_at_64_kib() {
    // Lines longer than DEFAULT_LINE_CAP split at the cap.
}
