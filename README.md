# Revo Launcher

A fast, clickable terminal Minecraft launcher written in Rust.

- Click tabs, version rows, and buttons with your mouse — works in any modern terminal (Windows Terminal, iTerm2, Alacritty, Kitty, WezTerm).
- Streams parallel asset/library downloads (16 in flight), SHA-1 verified.
- Microsoft OAuth via your default browser → Xbox Live → XSTS → Minecraft Services. Refresh token cached in the OS keyring.
- Offline mode for testing single-player without an account.
- Tiny release binary (~5 MB after LTO + strip).

## Build & run

```powershell
cargo run --release
```

Keys: `1`–`4` switch tabs, `Tab` cycles them, `q`/`Esc` quits, `Enter` launches the selected version. On the Play tab, type to edit the offline username, `Backspace` to delete. Arrows / PageUp / PageDown / mouse wheel scroll lists and logs.

## Microsoft sign-in

Microsoft requires every Minecraft launcher to bring its own Azure app registration. Free, one-time setup:

1. Go to <https://portal.azure.com> → **App registrations** → **New registration**.
2. Name it anything (e.g. `revo-launcher-local`).
3. Supported account types: **Personal Microsoft accounts only**.
4. Redirect URI: **Public client/native (mobile & desktop)** → `http://localhost/callback` (the port is chosen at runtime; only the path matters for matching).
5. Copy the **Application (client) ID** from the overview page.
6. Set the env var before launching:

```powershell
$env:REVO_MS_CLIENT_ID = "<your-client-id>"
cargo run --release
```

Without that var, you can still use **offline mode** — single-player only.

## Data layout

Saved under the OS data dir (`%APPDATA%\revo\RevoLauncher` on Windows):

```
versions/<id>/<id>.json    # cached version manifest
versions/<id>/<id>.jar     # cached client jar
libraries/...              # Maven-shaped library cache
assets/indexes/<id>.json   # asset index per assets id
assets/objects/<2>/<hash>  # content-addressed asset blobs
natives/<version>/         # extracted .dll/.so/.dylib per launch
instances/<version>/       # game cwd (saves, options.txt, etc.)
logs/revo.log              # tracing log file
```

## Architecture

| File | What |
|---|---|
| `main.rs` | Terminal setup, event loop, action dispatch |
| `app.rs` | Central `App` state, screen enum, worker-msg handler |
| `ui.rs` | Immediate-mode render with mouse hit-test regions |
| `event.rs` | `Hit` (click targets), `Tab`, `WorkerMsg` |
| `manifest.rs` | Mojang `version_manifest_v2.json` |
| `version.rs` | Per-version JSON + rule engine for libraries/args |
| `auth.rs` | MS OAuth → XBL → XSTS → MC token chain (PKCE S256) |
| `download.rs` | Parallel SHA-1-verified streaming downloader (16 conc.) |
| `launch.rs` | Builds JVM command, spawns Java, streams stdout/stderr |
| `java.rs` | Locates Java on PATH / `$JAVA_HOME`, parses `-version` |
| `paths.rs` | Cross-platform data/cache dir resolution |
| `theme.rs` | Color palette and named styles |
| `worker.rs` | Tokio tasks for install/launch with progress reporting |

Java 17+ is required for Minecraft 1.17+; older versions need Java 8. The launcher checks `javaVersion.majorVersion` from the version JSON before launch and refuses to start if the detected JDK is too old.
