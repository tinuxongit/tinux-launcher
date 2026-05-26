const { spawnSync } = require("node:child_process");

const cargo = process.platform === "win32" ? "cargo.exe" : "cargo";
const result = spawnSync(cargo, ["build", "--release"], {
  cwd: process.cwd(),
  stdio: "inherit",
  windowsHide: false,
});

if (result.error) {
  console.error(`Failed to build Tinux Launcher: ${result.error.message}`);
  console.error("Install Rust from https://rustup.rs, then run this install again.");
  process.exit(1);
}

process.exit(result.status ?? 0);
