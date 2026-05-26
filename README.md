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
git clone https://github.com/citropy/tinux-launcher
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

## Microsoft sign-in

Click the **Sign in (MS)** button on the Accounts tab. Your default browser opens to the Microsoft login page; sign in with the account that owns your Minecraft Java Edition. After you confirm the consent screen the browser shows "Sign-in complete" and the launcher's status bar says "Signed in as ⟨username⟩".

Tinux ships with a baked-in Azure App ID (`164bca05-…`) so end users never have to register anything — sign-in works out of the box, exactly like the official launcher. Until the app is approved by Mojang, you'll see a 403 with a clear hint after consent; **offline mode still works** in the meantime.

If you want to use your own Azure app instead (e.g. you're forking and shipping your own build), you have three ways to override the baked id, highest precedence first:

1. **Env var** — `TINUX_MS_CLIENT_ID=<guid>` (one-off, e.g. for CI tests).
2. **Config file** — paste the GUID into `ms_client_id` in `config.json` (see "Data layout" for the path). Persists between runs without editing source.
3. **Source** — replace the `BAKED_CLIENT_ID` constant in `src/auth.rs` and rebuild.

## Data layout

Saved under the OS data dir (`%APPDATA%\revo\RevoLauncher` on Windows; equivalent paths on macOS / Linux):

```
config.json                # optional: { "ms_client_id": "<guid>" }
versions/<id>/<id>.json    # cached version manifest
versions/<id>/<id>.jar     # cached client jar
libraries/...              # Maven-shaped library cache
assets/indexes/<id>.json   # asset index per assets id
assets/objects/<2>/<hash>  # content-addressed asset blobs
natives/<version>/         # extracted .dll/.so/.dylib per launch
instances/<version>/       # game cwd (saves, options.txt, etc.)
logs/tinux.log             # tracing log file
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
| `config.rs` | `config.json` loader, stub creation, default-app open helper |
| `download.rs` | Parallel SHA-1-verified streaming downloader (16 conc.) |
| `launch.rs` | Builds JVM command, spawns `javaw.exe`, streams stdout/stderr |
| `java.rs` | Locates Java on `PATH` / `$JAVA_HOME`, parses `-version` |
| `paths.rs` | Cross-platform data/cache dir resolution |
| `theme.rs` | Color palette and named styles |
| `worker.rs` | Tokio tasks for install/launch with progress reporting |

Java 17+ is required for Minecraft 1.17+; older versions need Java 8. The launcher reads `javaVersion.majorVersion` from each version's JSON and refuses to launch if the detected JDK is too old.

## License

MIT.
