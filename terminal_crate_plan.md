# `afar` — Companion Crate Design

A small, focused egui crate for operating shells from afar: line-mode terminal panes over local PTY or SSH, fanned out across hosts, rendered through elegance's `MultiTerminal`.

A separate crate that turns elegance's presentational `MultiTerminal` into a working terminal: real local shells over PTY, real remote sessions over SSH, line-mode rendering, while keeping `egui-elegance` itself dependency-free.

This is a *proposal*. The **Design Decisions** section at the end records the choices that shaped the scope, with reasoning preserved so the rationale survives the move to a fresh repo.

---

## 1. Why a separate crate

`egui-elegance` today depends only on `egui` and (on wasm) `js-sys`. A working line-mode terminal pulls in:

- a tokio async runtime,
- `russh` for SSH,
- `portable-pty` for local PTYs,
- `vte` as a small byte-level escape state machine (used minimally; see §8).

Each is fine on its own; together they meaningfully expand the dependency footprint of the host crate and add a security-sensitive surface (key handling, host-key trust) that needs its own release cadence. Keeping it out-of-tree means apps that only want elegance widgets pay nothing, and apps that want the terminal can pin the heavier crate independently.

The split also matches a real architectural seam: the existing `MultiTerminal` already exposes input via `TerminalEvent::Command` and output via `push_line` / `pane_mut`. That seam is exactly where a backend belongs.

## 2. Scope

The crate ships **one** widget: `LiveMultiTerminal`, a thin wrapper around elegance's existing `MultiTerminal` that owns one or more backends and pumps bytes between them and the line-buffered scrollback model the host widget already understands.

Goals:

- **Drop-in for the line-buffered case.** If the caller wants "type a command, get output, render it in elegance style," `cargo add afar` and pointing it at a host should be enough. No manual wiring of channels, threads, or parsers.
- **Backend-agnostic.** `LocalShell`, `SshSession`, and a user-implementable `TerminalBackend` trait. Tests run against a `MockBackend` that replays scripted byte streams.
- **Async without poisoning the UI.** All I/O happens on a tokio runtime owned by the crate; the UI side sees only `mpsc` channels and pulls bytes once per frame. egui stays fully synchronous.
- **Visually native to elegance.** Reuses the host widget's palette, typography, indicator glyph, and pill toggles. ANSI colour codes from the remote map onto `LineKind` so `cargo build` errors render in elegance's danger red rather than raw ANSI.
- **Secure by default.** Strict host-key checking on first use (TOFU with a stored fingerprint), no plaintext passwords on disk, agent-only by default for key auth.

## 3. Non-Goals

- **Not a full terminal emulator.** No cell grid, no alternate screen buffer, no cursor positioning, no mouse reporting. **`vim`, `htop`, `less`, `top`, `tmux` will not work.** The use case is line-buffered: shells, builds, log streams, REPLs, fleet command fan-out. Users who need a full TUI already have a real terminal emulator on their machine; running one inside an egui app is a niche we deliberately do not serve.
- **Not a multiplexer.** No tmux/screen reimplementation, no session persistence across app restarts.
- **Not an SSH client suite.** No SFTP, no port forwarding UI, no jump-host configuration UI. The crate exposes the SSH primitives but does not ship UI for them.
- **Not a serial / telnet console.** Pluggable backend, but the shipped backends are local PTY and SSH only.
- **Not a browser-native SSH client.** No russh-in-wasm. When the browser story is needed (v1.x), it follows the VS Code / Codespaces model: a `WebSocketBackend` in the browser pairs with a trusted server-side relay that owns the real `LocalShell` or `SshSession`. See §13 (Resolved) for the rationale.

## 4. The widget: `LiveMultiTerminal`

Wraps elegance's existing `MultiTerminal`. The caller hands it a backend per pane; the wrapper drains `TerminalEvent::Command`, writes the command bytes to the backend, and pumps stdout/stderr lines back into `push_line`. ANSI colour codes (SGR) are matched and mapped onto `LineKind` (red → `Err`, green → `Ok`, yellow → `Warn`, blue/cyan → `Info`, default → `Out`); cursor-positioning escapes (`ED`, `CUP`, `EL`, alt-screen toggles, mouse modes, OSC titles, OSC 52 clipboard) are silently dropped. Anything that requires a 2D grid is discarded; the use case is text in, text out.

`LiveMultiTerminal` exposes a `controls()` method that returns a restricted view (`LiveMultiTerminalControls<'_>`) delegating the safe `MultiTerminal` operations: broadcast/solo/collapse, focus, scrollback queries, pending-input edits. Structural mutators (`add_pane`, `remove_pane`) and direct pane writes (`push_line`, `send_command`) are deliberately *not* exposed: they would break the wrapper's invariant that every pane has a registered backend and every command flows through the input pump. New panes are added via typed builders on `LiveMultiTerminal` itself (`add_ssh_pane`, `add_local_pane`, etc.) so the backend registration and the elegance pane creation happen atomically.

## 5. Crate layout

```
afar/
├── Cargo.toml
├── src/
│   ├── lib.rs              # re-exports, feature gates
│   ├── backend/
│   │   ├── mod.rs          # TerminalBackend trait
│   │   ├── local.rs        # LocalShell (portable-pty)
│   │   ├── ssh.rs          # SshSession (russh)
│   │   └── mock.rs         # MockBackend (test only)
│   ├── runtime.rs          # tokio runtime owner, spawn helpers
│   ├── bytes.rs            # ring buffer between backend and UI
│   ├── ansi.rs             # SGR → LineKind matcher, CSI/OSC stripper
│   └── live_multi.rs       # LiveMultiTerminal (wraps elegance)
├── examples/
│   ├── local_shell.rs      # one-liner: spawn $SHELL into a single pane
│   ├── ssh_session.rs      # connect to one host with key auth
│   └── fleet.rs            # broadcast commands across N hosts
└── tests/
    ├── ansi.rs             # SGR mapper / escape stripper
    └── ssh_mock.rs         # MockBackend round-trips
```

### 5.1 Cargo features

- `default = ["local", "ssh"]`
- `local` — gates `LocalShell` and `portable-pty`. Native targets only.
- `ssh` — gates `SshSession` and `russh`. Compiles `rustls` not `openssl` by default; expose `ssh-openssl` for users who want system OpenSSL. Native targets only.
- `serde` — derive `Serialize` / `Deserialize` on `SshConfig`.
- `tracing` — emit spans for reconnects and auth attempts.
- *(planned, v1.x)* `websocket` — gates a `WebSocketBackend` that connects the browser-side widget to a trusted server-side relay. Wasm-buildable; pairs with a relay process that owns the real `LocalShell` or `SshSession`. Follows the VS Code / Codespaces architecture.

A user who wants only `LiveMultiTerminal` with their own backend can disable `local` and `ssh` and depend on the trait alone. That is also the configuration a wasm build will use today, with the `websocket` feature filling in the real backend once it ships.

## 6. Backend trait

Because we're tokio-only (Resolved §14), the trait reuses `tokio::io` for the byte-stream surface rather than inventing a poll signature of our own:

```rust
use tokio::io::{AsyncRead, AsyncWrite};

/// A bidirectional byte stream tied to a remote or local shell, plus
/// the orthogonal control operations a tty needs.
///
/// Implementations are owned by a per-session task on the runtime; the
/// UI side never holds a `TerminalBackend` directly. It works through
/// `BackendHandle`, which is `Send + Sync` and cheap to clone.
pub trait TerminalBackend: AsyncRead + AsyncWrite + Send + Unpin + 'static {
    /// PTY size changed. Mapped to `ioctl(TIOCSWINSZ)` on local PTYs and
    /// to a `window-change` channel request on SSH sessions. Backends
    /// without a PTY may return `Ok(())` and ignore.
    fn resize(&mut self, cols: u16, rows: u16) -> io::Result<()>;
}
```

Reading and writing go through the standard `tokio::io::AsyncReadExt` / `AsyncWriteExt` extension traits, so any tokio-compatible stream (`tokio::process::ChildStdout`, `tokio::net::TcpStream`, `tokio_tungstenite::WebSocketStream`-via-adapter) can be used as a backend with a small wrapper that adds `resize`. Shutdown is `AsyncWrite::poll_shutdown`; we don't reinvent it.

`BackendHandle` is the UI-facing facade:

```rust
#[repr(u8)]
pub enum TerminalStatus {
    Connected = 0,
    Reconnecting = 1,
    Offline = 2,
}

pub struct BackendHandle {
    tx: mpsc::Sender<UiToBackend>,        // input bytes, resize, shutdown
    rx: mpsc::Receiver<BackendEvent>,     // output bytes, status changes
    status: Arc<AtomicU8>,                // TerminalStatus, repr(u8); cheap polling
    _runtime: Arc<Runtime>,               // keeps the singleton runtime alive (§9)
}

pub enum BackendEvent {
    Bytes(Bytes),                          // raw output
    StatusChanged(TerminalStatus),         // Connected/Reconnecting/Offline
    Lossy { dropped: usize },              // ring buffer overran (drop-newest by default)
    InputLost { dropped: usize },          // input dropped on reconnect (§7.5)
    Closed { reason: CloseReason },
}
```

The status `Arc<AtomicU8>` is a cheap-poll fast path so the per-frame header render doesn't have to drain `rx` just to know whether to colour the pill amber. Authoritative state changes flow through `BackendEvent::StatusChanged`.

The widget's `show` reads from `rx` until empty, feeds bytes to the ANSI splitter, then renders. egui's `request_repaint` is called whenever bytes arrive, so idle sessions cost nothing.

## 7. SSH backend specifics

`SshSession` is the only piece of this crate that needs serious care. The shape:

```rust
pub struct SshConfig {
    pub host: String,
    pub port: u16,                   // default 22
    pub user: String,
    pub auth: SshAuth,               // Agent | Key { path, passphrase } | Password
    pub host_key: HostKeyPolicy,     // StrictKnownHosts | TofuStore { path } | AcceptAny (test only)
    pub keepalive: Option<Duration>, // default Some(30s)
    pub connect_timeout: Duration,   // default 10s
    pub agent_forward: bool,         // default false
    pub env: Vec<(String, String)>,  // exported into the remote shell
    pub term: String,                // TERM, default "xterm-256color"
}
```

### 7.1 Authentication

- **Agent first.** If `SSH_AUTH_SOCK` is present and the user picked `SshAuth::Agent`, walk the agent's identities and try each. This is the default in `examples/ssh_session.rs`.
- **Key files** are read on the runtime thread; passphrases are *not* persisted by the crate. Caller decides whether to prompt or pull from a keyring.
- **Password** is feature-gated behind `ssh-password` (off by default) so it does not appear in IDE autocomplete unless explicitly opted in. The whole industry has moved off password auth and the crate should not make it the easy path.

### 7.2 Host keys

`HostKeyPolicy::StrictKnownHosts` parses `~/.ssh/known_hosts` and refuses unknown hosts. `TofuStore { path }` writes a JSON file of `{host -> fingerprint}` on first connect and rejects mismatches. `AcceptAny` is gated behind `#[cfg(any(test, feature = "ssh-insecure"))]` so it cannot be selected from a production build by accident.

The `known_hosts` parser handles the realistic edge cases: hashed entries (`|1|...` HMAC-SHA1), `@cert-authority` and `@revoked` markers, multiple keys per host, wildcard patterns (`*.foo.com`, `192.168.*`), and IPv6 addresses with the `[host]:port` bracket form. Tests cover each.

### 7.3 Host-key mismatch callback

Strict policies can't tell a legitimate key rotation (host rebuilt, scheduled rotation) from a MITM. Telling users "edit `~/.ssh/known_hosts` by hand" defeats the "drop-in" promise. `SshConfig` therefore takes an optional callback:

```rust
pub struct HostKeyMismatch {
    pub host: String,
    pub stored_fingerprint: String,    // what we expected
    pub presented_fingerprint: String, // what the server offered
    pub algorithm: String,             // e.g. "ssh-ed25519"
}

pub enum Decision {
    Reject,        // close the connection (default if no callback set)
    AcceptOnce,    // proceed for this session, don't update the store
    AcceptAndStore // proceed and overwrite the stored fingerprint
}

pub type MismatchCallback = Box<dyn Fn(HostKeyMismatch) -> Decision + Send + Sync>;
```

The crate parses the keys, compares fingerprints, and persists decisions to the TOFU store. The host app owns the dialog UX (modal, button row, "show diff" link). Default behaviour without a callback is `Reject` so a host app that forgets to wire it up fails closed, not open.

### 7.4 Channels and signals

One session = one channel for v1. `WindowChange` is sent on resize. Signals:

- **Ctrl-C** (`0x03`) is sent as a raw byte; the remote tty driver translates it to `SIGINT` for the foreground process group. The SSH `Signal` channel request is used only when ISIG is off on the remote.
- **Ctrl-D** (`0x04`) is sent as a raw byte; the tty driver delivers EOF to the reading process. Closes interactive shells.
- **Job control** (Ctrl-Z `SIGTSTP`, Ctrl-\ `SIGQUIT`) is **not supported**. We don't track foreground/background process groups, don't render `[1]+ Stopped` markers, don't expose `bg`/`fg`. A pressed Ctrl-Z passes the byte through to the remote tty, which suspends the process; recovery requires the user to type `fg` blindly. This is a documented limitation, not a bug; line-mode terminals don't ship job-control affordances.

No attempt to track multiple concurrent channels in one session; callers needing that instantiate two `SshSession`s with the same `SshConfig`.

### 7.5 PTY mode and echo

A PTY is allocated by default so interactive prompts (`sudo`, ssh asking for a remote passphrase) and signal handling work. Callers who want strict one-shot exec semantics can use `SshConfig::no_pty()`, which switches to an `exec` channel.

PTYs default to **echo on**, which collides with `MultiTerminal`'s existing behaviour of rendering the typed command as a `LineKind::Command` line: without intervention the user sees the command twice (once from elegance's local echo, once from the remote `bash` echoing it back). Resolution: on session establishment we send `stty -echo 2>/dev/null\n` as the first input. This works across `bash`, `zsh`, `dash`, `sh`, `fish`. PowerShell does not have `stty` and will double-echo; this is documented as a known limitation, with the recommendation to set the remote `$PSStyle.OutputRendering = 'Host'` and accept the duplication, or use `no_pty()` for one-shot Windows-host workflows.

### 7.6 Reconnection

On transport error, `SshSession` emits `StatusChanged(Reconnecting)` and retries with exponential backoff (1s, 2s, 4s, capped at 30s, max 5 attempts by default). Output bytes already buffered for the UI are preserved; new bytes after the reconnect appear below a `--- reconnected ---` divider line. After max attempts the status flips to `Offline` and the channel closes.

Pending input is **dropped**, not replayed. Specifically: any bytes queued in the `BackendHandle` send channel and the local `pending` line buffer in `MultiTerminal` are discarded the moment `Reconnecting` is entered, and `BackendEvent::InputLost { dropped: N }` is emitted so the pane can render an `[input lost: N bytes]` marker. Replay would be unsafe: the user's mental model after a transport hiccup is "the command never went," so silently re-running it after reconnect could execute destructive commands twice. We make no claim about input that was *already* sent before the failure, since SSH does not provide per-byte ACKs; users should treat reconnect as a hard input boundary and re-issue commands deliberately.

## 8. ANSI handling

Line mode means every byte from the backend ends up in one of three buckets: line content, an SGR style update affecting the line being assembled, or discarded.

We use `vte::Parser` for the byte-level state machine because getting CSI / OSC parsing right against hostile inputs is exactly the kind of error-prone work where leaning on a battle-tested crate beats writing it ourselves. We then implement a trivial `vte::Perform` that:

1. **`print(c)`**: append `c` to the current line buffer. If the buffer crosses 64 KiB, force-emit a `TerminalLine` with the resolved `LineKind` (see below) and start a new buffer.
2. **`execute(b'\n')` / `execute(b'\r')`**: emit the current line buffer as a `TerminalLine`; clear the buffer.
3. **`csi_dispatch(...)` ending in `m`**: parse SGR parameters and update the current SGR state. Mapping (see §8.1 for how it resolves into a single `LineKind`):
   - `0` → reset to default.
   - `30..37`, `90..97` (foreground): red → `Err`, green → `Ok`, yellow → `Warn`, blue/cyan → `Info`, default/white/grey → `Out`.
   - `38;5;n` (256-colour) and `38;2;r;g;b` (truecolor): collapsed to the nearest of the same five buckets.
   - Bold/italic/underline are ignored for v1; the elegance line model has no axis for them.
4. **Everything else** (`ED`, `EL`, `CUP`, `CUU`, `DECSET`, alt-screen toggles, OSC titles, OSC 52, hyperlink OSC 8, mouse modes): silently dropped. We never observe a 2D grid, so cursor-positioning has nothing to act on.

The total `Performer` impl is roughly 100 to 150 lines. The hot path is `print`; everything else is rare.

`LineKind::Command` is **not** emitted by the SGR mapper. `LiveMultiTerminal` synthesizes a `LineKind::Command` line itself when forwarding user input to the backend, so the typed prompt-and-command echo is rendered with elegance's prompt styling regardless of what the remote sends back. The mapping table above only governs *output* coming up from the backend.

### 8.1 Mid-line SGR resolution

Real-world coloured output (cargo, rustc, git, grep) emits SGR transitions inside a single line: `error[E0277]: ` is bold red, the rest of the line is default. "Last SGR state at newline wins" gives the wrong answer (the trailing source-location span loses the diagnostic colour). v1 uses **first non-default colour seen on the line**:

- We track an `Option<LineKind>` for the current line, initialised to `None`.
- Every SGR-driven non-default colour change writes the slot only if it's still `None`.
- On newline, the slot's value (or `LineKind::Out` if still `None`) is the line's `LineKind`.

This matches human intuition for the cargo/rustc/git case (the diagnostic prefix's colour wins), is cheap to compute, and never silently swaps the kind mid-line. It does lose information when a single line carries multiple semantic spans (e.g., `info: ... warning: ...` collapses to `Info`); that's an accepted v1 limit.

A future direction is per-span styling, where `TerminalLine` becomes a `Vec<Span>` with each span carrying its own `LineKind`. That requires extending elegance's `TerminalLine` model, which is a host-crate API change with its own tradeoffs (rendering complexity, serde shape, `LineKind::Command` interaction). Not in v1; tracked for a future elegance minor.

### 8.2 Backpressure and rate limits

- **Per-line cap (default 64 KiB):** force-split lines longer than this so a single hostile or runaway log entry can't OOM the pane. The 64 KiB floor accommodates `cargo expand` output, single-line JSON payloads, base64 blobs, pip resolution traces, and minified bundles. Configurable per `LiveMultiTerminal`.
- **Per-frame byte cap (default 256 KiB):** if more than this arrives between repaints, process the cap and defer the rest to the next frame. Prevents a flooded shell from blocking the UI thread.
- **Backend ring-buffer overflow: drop *newest*, not oldest.** When the per-pane byte ring fills (default 1 MiB) the bytes that arrive *after* the cap is hit are dropped, and a single `[N bytes dropped]` marker is rendered at the truncation point. Drop-newest is the right default for build/log streams: the head of the buffer holds the first error or stack trace, which is exactly what the user is scrolled up to find. Drop-oldest erases that. Live-tail scenarios (`tail -f /var/log/...` where only the freshest output matters) can opt in via `LiveMultiTerminal::overflow_policy(OverflowPolicy::DropOldest)`.
- **Per-pane scrollback cap:** delegated to `MultiTerminal::scrollback_cap`. Drop-oldest is correct here because the scrollback is a fixed window of history, not transient flood mitigation; old lines age out as new ones arrive.

## 9. Threading model

```
┌──────────────┐  Bytes / Status  ┌────────────────┐
│ tokio task   │ ───────────────▶ │ BackendHandle  │
│ (per session)│ ◀─────────────── │ (Send + Sync)  │
└──────────────┘  Input / Resize  └────────┬───────┘
       ▲                                   │
       │ russh / portable-pty              │ pulled per frame
       ▼                                   ▼
   network / OS                     ┌─────────────────────┐
                                    │ egui UI thread      │
                                    │ LiveMultiTerminal   │
                                    │   ::show            │
                                    └─────────────────────┘
```

### 9.1 Runtime lifecycle

The crate owns a process-singleton multi-thread tokio runtime, behind a `Mutex<Weak<Runtime>>`. Behaviour:

- The first backend spawn upgrades the `Weak` and finds it empty, so it constructs a fresh `Runtime`, stores a `Weak` reference globally, and returns an `Arc<Runtime>` to the caller. The `BackendHandle` holds that `Arc` for the lifetime of its session.
- Subsequent backend spawns find the `Weak` still upgradable and clone the existing `Arc`. All live backends share one runtime.
- When the last `BackendHandle` is dropped, the `Arc` count goes to zero, the `Weak` becomes invalid, and the runtime shuts down (worker threads exit, the `Runtime` is destroyed). A subsequent backend spawn re-initialises a new runtime cleanly.

This handles the realistic patterns explicitly: two `LiveMultiTerminal`s in the same process share the runtime; an app that creates and tears down a `LiveMultiTerminal` repeatedly does not leak threads; a long-idle app pays no runtime cost while no sessions are open.

If the host app already runs tokio, `with_runtime(handle)` accepts an external `tokio::runtime::Handle` and skips the singleton entirely; the host owns the lifecycle. We deliberately keep the lazy path: forcing every caller to wire up a `Runtime` themselves is friction for simple apps that don't otherwise touch tokio.

### 9.2 Channels and overflow

Channels are bounded (`mpsc::channel(64)` for events, ring-buffered `BytesMut` for the actual output stream) so a flooded shell can't OOM the app. The default ring policy is **drop-newest** (see §8.2): when the cap is hit, the bytes still arriving from the backend are dropped, and `BackendEvent::Lossy { dropped }` is emitted at the truncation point. Drop-oldest is opt-in for live-tail scenarios.

## 10. Public API sketch

```rust
use afar::{LiveMultiTerminal, SshAuth, SshConfig};

// Single-host SSH session as one pane.
struct App {
    term: LiveMultiTerminal,
}

impl App {
    fn new() -> Self {
        let mut term = LiveMultiTerminal::new("ssh");
        term.add_ssh_pane("edge-01", SshConfig {
            host: "edge-01.internal".into(),
            user: "root".into(),
            auth: SshAuth::Agent,
            ..SshConfig::default()
        });
        Self { term }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        self.term.show(ui);
    }
}
```

```rust
// Fleet view, broadcasting commands across many hosts.
struct Fleet {
    terms: LiveMultiTerminal,
}

impl Fleet {
    fn new(hosts: &[&str]) -> Self {
        let mut terms = LiveMultiTerminal::new("fleet");
        for h in hosts {
            terms.add_ssh_pane(h, SshConfig::for_host(h, "root"));
        }
        Self { terms }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        self.terms.show(ui); // events drained, output pumped, no extra wiring
    }
}
```

`LiveMultiTerminal` exposes a `controls()` accessor returning `LiveMultiTerminalControls<'_>` for the safe operations (broadcast/solo/collapse, focus, scrollback queries). Structural mutation is reserved to typed builders on `LiveMultiTerminal` itself; see §4 for the rationale.

## 11. Testing strategy

- **ANSI handler (CI).** Unit tests for the SGR matcher: every standard colour code, mixed sequences, malformed parameters, partial sequences split across reads, mid-line transitions resolved by the "first non-default colour" rule (§8.1). Plus the CSI/OSC stripper: alt-screen toggles, mouse modes, OSC 52 attempts, OSC 8 hyperlinks, OSC 0/2 titles. Pure data-in-data-out.
- **Backend round-trips (CI).** `MockBackend` scripts let tests assert "given these bytes from the remote, the rendered scrollback contains these `TerminalLine`s with these `LineKind`s, and the input bytes the widget sent are these." Covers the `Performer` end-to-end without spawning processes.
- **In-process SSH integration (CI).** Test feature `ssh-test-server` spins up a `russh::server` in the same process as the test, connects to it as a client, and exercises auth flows, host-key parsing edge cases, reconnect under simulated transport failure, signal forwarding, and PTY echo handshake. Runs on every PR; this is the path that catches russh API drift.
- **Real-sshd integration (manual).** `cargo test --features ssh-real-sshd` exercises the same flows against a local OpenSSH server. Not in CI because it needs an out-of-process daemon and platform-specific config; run by hand before each release. Catches real-world sshd quirks the in-process server doesn't model (config negotiation differences, kex algorithms, agent-forwarding behaviour).
- **Visual snapshots (CI).** Reuse `egui_kittest` for the elegance look: pane header, status pill, scrollback area, `[N bytes dropped]` marker, `[input lost: N bytes]` marker, `--- reconnected ---` divider.

## 12. Security considerations

- **Host keys are checked.** The TOFU store rejects mismatches by default; legitimate rotations go through the explicit `MismatchCallback` (§7.3) so a host app can show a confirmation dialog. Without a callback wired up the connection fails closed. `AcceptAny` is `cfg`-gated.
- **OSC sequences are silently dropped, not interpreted.** No clipboard injection vector via OSC 52, no title-spoofing via OSC 0/2, no hyperlink-click handling via OSC 8. The class of "hostile remote prints an escape sequence to do X to the host app" simply doesn't apply, because we don't observe any of those sequences.
- **Per-line and per-frame caps** (§8.1) prevent a hostile remote from OOM'ing the app with a giant unbroken line or starving the UI thread with sustained flooding.
- **Agent forwarding is off by default** and warns once per session in `tracing::warn!` when enabled, since it lets the remote impersonate the user against any host the agent can reach.
- **No telemetry.** The crate emits `tracing` events the host app can subscribe to; it never opens a network connection of its own beyond the configured SSH endpoint.
- **Dependencies pinned and audited.** `cargo-deny` config in repo. `russh`, `portable-pty`, `vte` are the surface; treat their advisories as ours.

## 13. Phasing

| Milestone | Scope | Rough size |
|---|---|---|
| **M0** | Crate scaffolding, `TerminalBackend` trait (over `tokio::io`), `MockBackend`, `BackendHandle`, singleton runtime with `Weak`-based lifecycle. No widget yet. | ~1 week |
| **M1** | `LiveMultiTerminal` over `LocalShell`. ANSI handler (SGR → `LineKind` with first-non-default rule, CSI/OSC stripper), 64 KiB per-line cap, drop-newest ring with markers, `LiveMultiTerminalControls` view, examples. | ~1.5 weeks |
| **M2** | `SshSession`: agent auth, strict known_hosts, TOFU, keepalive, signals (Ctrl-C, Ctrl-D), `pty-req` with `stty -echo` handshake, `window-change` on resize, reconnect with input-drop and `[input lost]` marker. | ~3 weeks |
| **M2.5** | Host-key edge cases (hashed entries, `@cert-authority`, IPv6 brackets, multiple keys, wildcards) and the `MismatchCallback` plumbing. Real-sshd integration tests. | ~1 week |
| **M3** | Hardening: rate-limit fuzzing, ANSI matcher fuzzing with `cargo-fuzz`, docs polish, cargo-deny audit, semver-1.0. | ~1 week |

Roughly 7 to 8 weeks of focused work for a v1 that fans out commands across SSH hosts cleanly. The risk is concentrated in M2 and M2.5: russh API ergonomics, host-key parsing edge cases, reconnect semantics, and the cross-shell `stty -echo` handshake. M1 is mostly mechanical because `vte` does the byte-level state machine; the work is getting the SGR mapping right against real-world `cargo`/`git`/`grep --color` output.

## 14. Design Decisions

The decisions below shaped the scope above; each entry records *what* was chosen and *why*, so future contributors don't have to re-derive the reasoning.

- **No cell-grid emulation. Line mode only.** v1 ships only `LiveMultiTerminal`; there is no full-screen `Terminal` widget. `vim`, `htop`, `less`, `tmux` and other TUIs are out of scope and will not be supported by this crate. Reasoning: cell-grid emulation (vte `Performer`, alt-screen, scrollback grid, cursor positioning, mouse, selection, key encoding, palette mapping, resize re-flow) is roughly 80% of the implementation work for marginal additional value to the realistic users of this crate (ops dashboards, fleet automation, build/log streaming, agent-driven shells). A user who wants `vim` over SSH already has a real terminal emulator on their machine; running one inside an egui app is a niche we deliberately don't serve. The library-extraction rule reinforces this: there's no second use case to justify generalising. Side benefits: simpler dep tree, no parser conformance burden, no clipboard-OSC attack vector, ship in 5 to 6 weeks instead of 11.
- **Async runtime: tokio-only.** `russh` and `portable-pty`'s async story both target tokio; abstracting over smol/async-std would double the surface for no real user benefit. The crate's `with_runtime(handle)` escape hatch (§9) lets a host app that already runs tokio share its runtime, which covers the realistic concern. Hosts on a non-tokio runtime can still drive a `TerminalBackend` impl of their own.
- **`MultiTerminal` stays in `egui-elegance`.** Not relocated to this crate. It is already shipped in 0.4.0 and useful purely as a presentational widget for mockups, design demos, and line-buffered apps that have no business pulling tokio/russh into their dependency tree. The seam between the two crates (`TerminalEvent::Command` out, `push_line` in) has been stable across four releases, so the predicted co-evolution churn is mostly hypothetical. The companion crate's `LiveMultiTerminal` wraps `elegance::MultiTerminal` and exposes `inner()` / `inner_mut()` for callers that want the existing broadcast/solo/collapse controls.
- **One crate, not split.** Cargo features (`local`, `ssh`, `serde`, `tracing`, `ssh-openssl`, `ssh-password`) cover the realistic axes of conditional compilation. Splitting into `-core` + `-ssh` + `-local` triples the release surface (three crate versions to coordinate, three changelogs, three sets of dep advisories) for negligible benefit unless compile time on a `default-features = false` build becomes a measured problem. Revisit only if a real user reports it.
- **`LiveMultiTerminal` stays in the companion crate.** Not upstreamed into `egui-elegance`. The hidden cost of upstreaming isn't async deps directly (the backend trait can be runtime-agnostic with `crossbeam-channel` or `std::sync::mpsc`); it's that elegance would gain a `TerminalBackend` trait as **public API**, expanding its brand from "presentational widgets" into "presentational widgets + I/O abstraction." That's a one-way door, and every future shape change to the trait (resize semantics, status variants, byte-loss signaling) would become a coordinated version bump of both crates instead of an internal change in the companion. The library-extraction rule also bites: `LiveMultiTerminal` has exactly one consumer (this crate), which fails the 2+ use cases bar. Concrete revisit trigger: a *third* crate wants to ship its own `MultiTerminal` backend without depending on the SSH/PTY tree.
- **No scrollback persistence in v1.** No auto-save of pane scrollback across app restarts. Terminal scrollback routinely contains secrets (passwords typed under broken echo, API keys, sudo prompts, agent socket paths), and auto-persisting that to disk crosses a line that **no** production terminal (Terminal.app, iTerm2, Alacritty, kitty, Windows Terminal) crosses by default. That industry-wide consensus is the strongest possible signal that this is the right default. The `serde` feature derives `Serialize` / `Deserialize` on `SshConfig` so callers can persist their connection list; persisting scrollback is a host-app concern that requires explicit user consent, and we don't ship it in the widget. Trigger to revisit: multiple adopters report a concrete use case the bare serde derives don't cover, *and* propose a UX for distinguishing live sessions from restored snapshots so users aren't fooled by stale output that looks live.
- **Browser support: VS Code architecture, deferred to v1.x.** Native-only for v1.0. When the browser story is needed, the answer is a `WebSocketBackend`: the browser runs `LiveMultiTerminal` and a thin WebSocket reader, while a trusted server-side relay owns the real `LocalShell` or `SshSession`. This is the architecture VS Code, GitHub Codespaces, github.dev, vscode.dev, and most web-SSH products converged on. Browser-side russh-in-wasm is not on the roadmap; it remains a niche security experiment and ships nowhere mainstream. The `TerminalBackend` trait (§6) already accommodates `WebSocketBackend` without disrupting any existing impl, so v1.x can add it as a non-breaking feature gate (`websocket`) when a real egui-on-wasm adopter shows up.
