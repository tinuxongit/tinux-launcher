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

## CI

`.github/workflows/ci.yml` runs `cargo test` on Ubuntu and Windows for every
push and PR. The clippy job is informational only because the codebase has
pre-existing warnings; don't gate on clippy or rustfmt until those are cleaned
up repo-wide.
