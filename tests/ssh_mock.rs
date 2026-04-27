//! `MockBackend` round-trip tests.
//!
//! See §11 of `terminal_crate_plan.md`. The in-process `russh::server`
//! integration tests live behind a feature flag (`ssh-test-server`) and
//! land in M2/M2.5.

#[test]
#[ignore = "M1: LiveMultiTerminal not yet implemented"]
fn mock_round_trip_emits_expected_lines() {
    // Drive a MockBackend with a scripted byte stream, render through
    // LiveMultiTerminal, assert the resulting TerminalLine sequence.
}
