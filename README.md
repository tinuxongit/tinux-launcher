# Tinux Launcher

Terminal Minecraft Java Edition launcher.

## Install & run

### pnpm

From GitHub:

```bash
pnpm add -g github:tinuxongit/tinux-launcher
tinuxlauncher
```

From this repository:

```bash
pnpm add -g .
tinuxlauncher
```

The pnpm installer downloads a prebuilt binary from GitHub Releases when one is available.
If there is no prebuilt binary for the user's OS/CPU, it falls back to building from source with Rust/Cargo.

To update to the latest GitHub version:

```bash
tinuxlauncher update
```

### Source

```bash
git clone https://github.com/tinuxongit/tinux-launcher
cd tinux-launcher
cargo run --release
```

Requires Rust 1.80+ and a JDK on `PATH` (Java 17+ for 1.17+, Java 8 for older).

## Keys

| | |
|---|---|
| `1`–`4`, `Tab` | switch tab |
| `Enter` | launch |
| arrows / wheel | scroll |
| click row | select log line |
| `Ctrl`+click | extend selection |
| `Ctrl+A` | select all logs |
| `Ctrl+C` | copy selection |
| `Ctrl+V` | paste (offline name field) |
| `Ctrl+L` | redraw |
| `Esc` / `q` | quit |
