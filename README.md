# Tinux Launcher

Terminal Minecraft Java Edition launcher.

Plays vanilla (every version), Fabric, NeoForge, and Forge (1.13+), with a
built-in Modrinth browser for mods, shaders, resource packs, datapacks, and
modpacks. Pick the mod loader on the Versions tab via the `Loader:` control.
Forge and NeoForge installs run the official installer headlessly, so they
need a JDK (17+ for modern versions).

## Install & run

### Quick install (no Node required)

Windows (PowerShell):

```powershell
irm tinux.dev/install-launcher.ps1 | iex
```

macOS / Linux:

```bash
curl -fsSL tinux.dev/install-launcher.sh | sh
```

Installs a single binary on your PATH. Update later with `tinuxlauncher update`.

### Uninstall

```bash
tinuxlauncher uninstall
```

Removes the binary and, on Windows, the PATH entry the installer added.
Asks separately before deleting game data (worlds, instances, mods, settings).

### npm

From GitHub:

```bash
npm install -g https://github.com/tinuxongit/tinux-launcher/archive/refs/heads/main.tar.gz
tinuxlauncher
```

Use the GitHub tarball URL with npm. The `github:tinuxongit/tinux-launcher`
shorthand can install incompletely on Windows with newer npm versions.

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

Both installers download a prebuilt binary from GitHub Releases (Windows x64,
Linux x64/arm64, macOS x64/arm64). If there is no prebuilt binary for the
user's OS/CPU, they fall back to building from source with Rust/Cargo.

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
| `1`–`5`, `Tab` | switch tab |
| `Enter` | launch |
| arrows / wheel | scroll |
| click row | select log line |
| `Ctrl`+click | extend selection |
| `Ctrl+A` | select all logs |
| `Ctrl+C` | copy selection |
| `Ctrl+V` | paste (offline name field) |
| `Ctrl+L` | redraw |
| `Esc` / `q` | quit |
