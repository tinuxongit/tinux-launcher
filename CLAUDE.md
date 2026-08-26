# CLAUDE.md

Tinux Launcher: terminal Minecraft Java Edition launcher (Rust, ratatui TUI).
Ships as a single prebuilt binary through GitHub Releases.

## How version updates / releases work

1. Land the changes on `main`. Plain pushes to `main` only warm the CI build
   cache; they never publish anything.
2. Bump `version` in `Cargo.toml`, run `cargo check` so `Cargo.lock` follows,
   and commit it as `Bump version to vX.Y.Z`.
3. Tag that commit and push the tag:

   ```
   git tag vX.Y.Z
   git push origin main vX.Y.Z
   ```

4. The tag push triggers `.github/workflows/release.yml`, which creates the
   GitHub release and uploads prebuilt binaries for Windows x64, Linux
   x64/arm64, and macOS x64/arm64, each with a `.sha256` sidecar file.

The `.sha256` sidecars are mandatory: the self-updater (launcher 0.1.36 and
newer) refuses to install an update whose checksum is missing or doesn't
match. Never remove them from a release.

Users receive updates via `tinuxlauncher update` or the install scripts, which
resolve the newest version through the `releases/latest` redirect on
github.com. There is no npm publish step; the npm/pnpm wrappers also download
from GitHub Releases.

## CI and tests

`.github/workflows/ci.yml` runs `cargo test` on Ubuntu and Windows for every
push and PR. The clippy job is informational. It was informational because the
codebase carried 26 warnings; that backlog is cleared and four remain, all of
them `too_many_arguments` at 8 against a limit of 7 on drawing and search
helpers. Gating on clippy means either fixing those four or allowing that one
lint. `cargo build` alone will not show you everything: some warnings only
appear under `--all-targets`, because test code counts as a use.

`cargo test` runs only the fast unit tests. The `#[ignore]`d end-to-end tests
in `src/forge.rs` download and run the real Forge/NeoForge installers
(network and a local JDK required): `cargo test -- --ignored --nocapture`.
Run them after touching forge.rs, the version-JSON merge, or download.rs's
library handling.
