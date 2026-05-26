# Tinux Launcher

A fast, clickable terminal Minecraft Java Edition launcher written in Rust.

- Click tabs, version rows, and buttons with your mouse — works in any modern terminal (Windows Terminal, iTerm2, Alacritty, Kitty, WezTerm).
- Parallel asset / library downloads (16 in flight), SHA-1 verified, streamed to disk.
- Microsoft OAuth via your default browser → Xbox Live → XSTS → Minecraft Services. Refresh token cached in the OS keyring.
- Offline mode for single-player without an account.
- In-app log copy: click a line, Ctrl+click to extend, Ctrl+C to clipboard.
- Tiny release binary (~5 MB after LTO + strip).

## Install & run

You need a Rust toolchain (1.80+) and a Java 17+ JDK on `PATH` for modern Minecraft versions (Java 8 still works for ≤ 1.16).

```bash
git clone https://github.com/tinuxongit/tinux-launcher
cd tinux-launcher
cargo run --release
```

## Keys & mouse

| Action | Key / Mouse |
|---|---|
| Switch tab | `1`–`4`, `Tab`, or click the tab |
| Launch selected version | `Enter` (on Play tab) or click `▶ Launch` |
| Edit offline username | Click the field, type, `Esc` / click away to unfocus |
| Scroll lists / logs | `↑`/`↓`, `PageUp`/`PageDown`, or mouse wheel |
| Select a log line | Click it |
| Extend log selection | `Ctrl`+click another line |
| Select every log line | `Ctrl+A` (on Logs tab) |
| Copy selection to clipboard | `Ctrl+C` |
| Paste into offline-name field | `Ctrl+V` (while field is focused) |
| Force a full repaint | `Ctrl+L` |
| Quit | `Esc` or `q` |
