//! Line-mode ANSI handling: SGR → `LineKind` matcher and CSI/OSC stripper.
//!
//! See §8 of `terminal_crate_plan.md`. We use `vte::Parser` for the
//! byte-level state machine and implement `vte::Perform` to:
//!
//! 1. Append printable characters to a per-line buffer.
//! 2. Emit a `TerminalLine` on `\n`/`\r`, with `LineKind` resolved by the
//!    "first non-default colour seen on the line" rule (§8.1).
//! 3. Translate SGR `m` parameters into `LineKind` updates.
//! 4. Silently drop everything else (cursor positioning, OSC, mouse).

use elegance::{LineKind, TerminalLine};

/// Per-line cap. Lines longer than this are force-split. Default 64 KiB.
pub const DEFAULT_LINE_CAP: usize = 64 * 1024;

/// Stateful line-mode handler that consumes backend bytes and emits
/// `TerminalLine`s for the host pane's scrollback.
#[allow(dead_code)] // `parser` drives the state machine in M1.
pub struct AnsiHandler {
    line_cap: usize,
    parser: vte::Parser,
    state: PerformerState,
}

#[derive(Default)]
#[allow(dead_code)] // `out` is the M1 sink for `vte::Perform` callbacks.
struct PerformerState {
    buffer: String,
    /// First non-default `LineKind` seen on the line. `None` means "no
    /// colour seen yet"; resolves to `Out` at newline.
    pending_kind: Option<LineKind>,
    out: Vec<TerminalLine>,
}

impl AnsiHandler {
    pub fn new() -> Self {
        Self::with_line_cap(DEFAULT_LINE_CAP)
    }

    pub fn with_line_cap(line_cap: usize) -> Self {
        Self {
            line_cap,
            parser: vte::Parser::new(),
            state: PerformerState::default(),
        }
    }

    /// Feed bytes from the backend. Any complete lines they produce are
    /// appended to the returned vector; the handler retains any
    /// trailing partial line for the next call.
    pub fn feed(&mut self, _bytes: &[u8]) -> Vec<TerminalLine> {
        // M1: drive `self.parser.advance(&mut performer, byte)` for each byte,
        // then drain `self.state.out`.
        todo!("AnsiHandler::feed — see terminal_crate_plan.md §8")
    }

    /// Force-emit any buffered partial line (e.g. on backend close).
    pub fn flush(&mut self) -> Option<TerminalLine> {
        if self.state.buffer.is_empty() {
            return None;
        }
        let kind = self.state.pending_kind.take().unwrap_or(LineKind::Out);
        let text = std::mem::take(&mut self.state.buffer);
        Some(TerminalLine::new(kind, text))
    }

    pub fn line_cap(&self) -> usize {
        self.line_cap
    }
}

impl Default for AnsiHandler {
    fn default() -> Self {
        Self::new()
    }
}
