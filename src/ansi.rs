//! Line-mode ANSI handling: SGR -> [`LineKind`] matcher and CSI/OSC stripper.
//!
//! See §8 of `terminal_crate_plan.md`. The handler drives `vte::Parser`
//! (which handles the byte-level state machine) and implements
//! `vte::Perform` to:
//!
//! 1. Append printable characters to a per-line buffer.
//! 2. Emit a [`TerminalLine`] on `\n`, with [`LineKind`] resolved by the
//!    "first non-default colour seen on the line" rule (§8.1).
//! 3. Translate SGR `m` parameters into [`LineKind`] updates.
//! 4. Silently drop everything else (cursor positioning, OSC, mouse).

use elegance::{LineKind, TerminalLine};

/// Per-line cap. Lines longer than this are force-split. Default 64 KiB.
pub const DEFAULT_LINE_CAP: usize = 64 * 1024;

/// Stateful line-mode handler that consumes backend bytes and emits
/// [`TerminalLine`]s for the host pane's scrollback.
pub struct AnsiHandler {
    line_cap: usize,
    parser: vte::Parser,
    state: PerformerState,
}

#[derive(Default)]
struct PerformerState {
    /// Bytes printed on the current line, not yet emitted.
    buffer: String,
    /// First non-default `LineKind` seen on the current line. `None` means
    /// "no semantic colour seen yet"; resolves to `LineKind::Out` at newline.
    pending_kind: Option<LineKind>,
    /// Lines emitted by `feed`, drained on each call.
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
    /// returned; the handler retains any trailing partial line for the
    /// next call.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<TerminalLine> {
        let line_cap = self.line_cap;
        let parser = &mut self.parser;
        let mut performer = Performer {
            state: &mut self.state,
            line_cap,
        };
        for &byte in bytes {
            parser.advance(&mut performer, byte);
        }
        std::mem::take(&mut self.state.out)
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

struct Performer<'a> {
    state: &'a mut PerformerState,
    line_cap: usize,
}

impl Performer<'_> {
    /// Emit the current line buffer as a [`TerminalLine`]. If `force_split`,
    /// preserve `pending_kind` so the continuation of the same logical line
    /// keeps the same colour; otherwise reset it (the line is over).
    fn emit_line(&mut self, force_split: bool) {
        // `LineKind` is not `Copy` because of the `Command` variant; clone
        // here when force-splitting so the continuation keeps the same
        // colour. We never put a `Command` kind into `pending_kind`, so
        // the clone is always cheap.
        let kind = if force_split {
            self.state.pending_kind.clone().unwrap_or(LineKind::Out)
        } else {
            self.state.pending_kind.take().unwrap_or(LineKind::Out)
        };
        let text = std::mem::take(&mut self.state.buffer);
        self.state.out.push(TerminalLine::new(kind, text));
    }

    /// Apply the "first non-default colour wins" rule. Only updates
    /// `pending_kind` if it's still `None` and the candidate is meaningful
    /// (not `Out`).
    fn maybe_set_kind(&mut self, candidate: LineKind) {
        if candidate == LineKind::Out {
            return;
        }
        if self.state.pending_kind.is_none() {
            self.state.pending_kind = Some(candidate);
        }
    }

    fn process_sgr(&mut self, params: &vte::Params) {
        let mut iter = params.iter();
        while let Some(slice) = iter.next() {
            // Take the first sub-parameter; SGR sub-params are rare and
            // we don't make use of them.
            let Some(&code) = slice.first() else {
                continue;
            };
            match code {
                // CSI m with no params, or CSI 0 m: reset SGR. We do not
                // reset `pending_kind` because the rule is "first non-default
                // wins" — once set, it stays for the line.
                0 => {}

                // Standard 8-colour foreground.
                30..=37 => {
                    if let Some(kind) = standard_fg_to_kind(code as u8) {
                        self.maybe_set_kind(kind);
                    }
                }

                // Bright 8-colour foreground.
                90..=97 => {
                    if let Some(kind) = standard_fg_to_kind((code - 60) as u8) {
                        self.maybe_set_kind(kind);
                    }
                }

                // 38: extended foreground. 38;5;n is 256-colour, 38;2;r;g;b
                // is truecolor. We collapse both to a single LineKind via
                // hue.
                38 => match iter.next().and_then(|s| s.first().copied()) {
                    Some(5) => {
                        if let Some(idx) = iter.next().and_then(|s| s.first().copied()) {
                            let (r, g, b) = xterm_256_to_rgb(idx as u8);
                            if let Some(kind) = rgb_to_kind(r, g, b) {
                                self.maybe_set_kind(kind);
                            }
                        }
                    }
                    Some(2) => {
                        let r = iter.next().and_then(|s| s.first().copied()).unwrap_or(0);
                        let g = iter.next().and_then(|s| s.first().copied()).unwrap_or(0);
                        let b = iter.next().and_then(|s| s.first().copied()).unwrap_or(0);
                        if let Some(kind) = rgb_to_kind(r as u8, g as u8, b as u8) {
                            self.maybe_set_kind(kind);
                        }
                    }
                    _ => {}
                },

                // 39: default foreground. No-op; "first non-default wins"
                // means we don't override pending_kind on reset.
                39 => {}

                // Background colours, attributes (bold/italic/underline),
                // and everything else: ignored. Line-mode rendering doesn't
                // distinguish on those axes today.
                _ => {}
            }
        }
    }
}

impl vte::Perform for Performer<'_> {
    fn print(&mut self, c: char) {
        self.state.buffer.push(c);
        if self.state.buffer.len() >= self.line_cap {
            self.emit_line(true);
        }
    }

    fn execute(&mut self, byte: u8) {
        // `\n` is the line terminator. `\r` is dropped silently so `\r\n`
        // produces one line, not two; loose `\r` (curl-style progress bars)
        // is also dropped, which means progress lines accumulate rather
        // than overwrite. Documented v1 limit; line mode doesn't render
        // in-place updates.
        if byte == b'\n' {
            self.emit_line(false);
        }
        // BEL, BS, HT, VT, FF, and other C0 controls: silently dropped.
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        _intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        if action == 'm' {
            // Empty `CSI m` (no parameters) is equivalent to `CSI 0 m`.
            // params.is_empty() reads as "no SGR change" for our purposes
            // since we don't reset pending_kind on `0` either.
            self.process_sgr(params);
        }
        // Cursor positioning (CUP, CUU, CUD, CUF, CUB), erase (ED, EL),
        // mode toggles (DECSET/DECRST including alt-screen and mouse),
        // and everything else: silently dropped.
    }

    // osc_dispatch, esc_dispatch, hook, put, unhook: default empty impls,
    // which is exactly the "silently drop" behaviour we want for OSC titles
    // (0/2), clipboard (52), hyperlinks (8), and DCS sequences.
}

/// Map an ANSI 8-colour foreground code (30..=37) to a `LineKind`.
/// Returns `None` for colours that don't carry a clear semantic meaning
/// (black, magenta, white) so they don't override `pending_kind`.
fn standard_fg_to_kind(code: u8) -> Option<LineKind> {
    match code {
        31 => Some(LineKind::Err),  // red
        32 => Some(LineKind::Ok),   // green
        33 => Some(LineKind::Warn), // yellow
        34 => Some(LineKind::Info), // blue
        36 => Some(LineKind::Info), // cyan
        // 30 (black), 35 (magenta), 37 (white) intentionally fall through
        // to None: no clean semantic mapping in the elegance line model.
        _ => None,
    }
}

/// Map an RGB triplet to a `LineKind` by hue. Used for both 256-colour
/// (after expansion) and truecolor SGR. Returns `None` when no channel
/// dominates clearly enough to assign a semantic.
fn rgb_to_kind(r: u8, g: u8, b: u8) -> Option<LineKind> {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    // Threshold for "this channel is meaningfully larger than that one."
    const MARGIN: i32 = 32;

    let red_dom = r > g + MARGIN && r > b + MARGIN;
    let green_dom = g > r + MARGIN && g > b + MARGIN;
    let blue_dom = b > r + MARGIN && b > g + MARGIN;
    // Yellow: red+green roughly equal, both well above blue.
    let yellow = (r - g).abs() <= MARGIN && r > b + MARGIN && g > b + MARGIN;
    // Cyan: green+blue roughly equal, both well above red.
    let cyan = (g - b).abs() <= MARGIN && g > r + MARGIN && b > r + MARGIN;

    if red_dom {
        Some(LineKind::Err)
    } else if green_dom {
        Some(LineKind::Ok)
    } else if yellow {
        Some(LineKind::Warn)
    } else if blue_dom || cyan {
        Some(LineKind::Info)
    } else {
        // Greys, near-blacks, magentas, near-whites: no semantic.
        None
    }
}

/// Convert an xterm 256-colour palette index to an approximate RGB.
fn xterm_256_to_rgb(idx: u8) -> (u8, u8, u8) {
    if idx < 16 {
        // Standard 16 colours, approximate xterm sRGB values.
        const STANDARD: [(u8, u8, u8); 16] = [
            (0, 0, 0),
            (170, 0, 0),
            (0, 170, 0),
            (170, 85, 0),
            (0, 0, 170),
            (170, 0, 170),
            (0, 170, 170),
            (170, 170, 170),
            (85, 85, 85),
            (255, 85, 85),
            (85, 255, 85),
            (255, 255, 85),
            (85, 85, 255),
            (255, 85, 255),
            (85, 255, 255),
            (255, 255, 255),
        ];
        STANDARD[idx as usize]
    } else if idx < 232 {
        // 6x6x6 RGB cube. Component values: 0, 95, 135, 175, 215, 255.
        let i = idx - 16;
        let component = |c: u8| -> u8 {
            match c {
                0 => 0,
                n => 55 + n * 40,
            }
        };
        let r = i / 36;
        let g = (i % 36) / 6;
        let b = i % 6;
        (component(r), component(g), component(b))
    } else {
        // Greyscale ramp.
        let v = 8 + (idx - 232) * 10;
        (v, v, v)
    }
}
