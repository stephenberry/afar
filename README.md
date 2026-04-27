# afar

Operate shells from afar: line-mode terminal panes over local PTY or SSH for [egui](https://github.com/emilk/egui), rendered through [`egui-elegance`](https://github.com/stephenberry/egui-elegance)'s `MultiTerminal`.

`afar` is the working-backend companion to elegance's presentational `MultiTerminal` widget. You get a multi-pane terminal that fans out commands to many SSH hosts at once, streams output back, renders it in elegance's design language, and never pulls heavy deps into apps that just want the widget chrome.

## Status

Pre-implementation. The full design is in [`terminal_crate_plan.md`](./terminal_crate_plan.md); the source tree is scaffolded but the backends, ANSI handler, and widget pump are not yet wired up. See the milestone table in §13 of the plan.

## Scope

- Line mode only. `cargo build`, `tail -f`, `git log`, REPLs, fleet command fan-out: yes. `vim`, `htop`, `less`, `tmux`, full-screen TUIs: no, by design (see plan §3).
- Backends: local PTY (`portable-pty`), SSH (`russh`). Browser support via a future `WebSocketBackend` to a trusted relay.
- Tokio-only. The crate owns a process-singleton runtime; host apps already on tokio can share via `with_runtime(handle)`.

## Install

```toml
[dependencies]
afar = "0.0.0"           # not yet published
egui-elegance = "0.4"
egui = "0.34"
```

## License

MIT OR Apache-2.0.
