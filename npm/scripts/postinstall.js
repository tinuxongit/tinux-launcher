const { spawnSync } = require("node:child_process");
const path = require("node:path");

// Resolve everything from this script's location, never from process.cwd() —
// package managers don't all run lifecycle scripts from the package root.
const root = path.resolve(__dirname, "..", "..");

const download = spawnSync(process.execPath, [path.join(__dirname, "install-binary.js")], {
  cwd: root,
  stdio: "inherit",
  windowsHide: false,
});

if (!download.error && download.status === 0) {
  process.exit(0);
}

console.error("Falling back to building Tinux Launcher from source.");
const cargo = process.platform === "win32" ? "cargo.exe" : "cargo";
const result = spawnSync(cargo, ["build", "--release"], {
  cwd: root,
  stdio: "inherit",
  windowsHide: false,
});

if (!result.error && result.status === 0) {
  process.exit(0);
}

if (result.error) {
  console.error(`Could not run cargo: ${result.error.message}`);
}
// Don't fail the whole install — the tinuxlauncher wrapper downloads the
// binary on first run, so a transient network problem here isn't fatal.
console.error("Could not set up the Tinux Launcher binary during install.");
console.error("It will be downloaded automatically the first time you run `tinuxlauncher`.");
console.error("(To build from source instead, install Rust from https://rustup.rs.)");
process.exit(0);
