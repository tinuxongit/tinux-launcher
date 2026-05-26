#!/usr/bin/env node

const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const GITHUB_SPEC = "github:tinuxongit/tinux-launcher";
const exeName = process.platform === "win32" ? "tinux-launcher.exe" : "tinux-launcher";
const root = path.resolve(__dirname, "..", "..");
const bin = path.join(root, "target", "release", exeName);
const args = process.argv.slice(2);

if (args[0] === "update" || args[0] === "--update") {
  const manager = findPackageManager();
  if (!manager) {
    console.error("Could not find pnpm or npm on PATH.");
    console.error(`Update manually with: pnpm add -g ${GITHUB_SPEC}`);
    process.exit(1);
  }

  const managerArgs =
    manager.name === "pnpm"
      ? ["add", "-g", GITHUB_SPEC]
      : ["install", "-g", GITHUB_SPEC];
  const result = spawnSync(manager.bin, managerArgs, {
    stdio: "inherit",
    windowsHide: false,
  });
  if (result.error) {
    console.error(`Failed to update with ${manager.name}: ${result.error.message}`);
    process.exit(1);
  }
  process.exit(result.status ?? 0);
}

if (args[0] === "--version" || args[0] === "-v") {
  const pkg = require("../../package.json");
  console.log(pkg.version);
  process.exit(0);
}

if (!fs.existsSync(bin)) {
  console.error("Tinux Launcher binary is missing; building it now...");
  const built = buildRelease();
  if (!built) {
    process.exit(1);
  }
}

const result = spawnSync(bin, args, {
  stdio: "inherit",
  windowsHide: false,
});

if (result.error) {
  console.error(`Failed to run ${bin}: ${result.error.message}`);
  console.error(`Try reinstalling with: pnpm add -g ${GITHUB_SPEC}`);
  process.exit(1);
}

process.exit(result.status ?? 0);

function buildRelease() {
  const cargo = process.platform === "win32" ? "cargo.exe" : "cargo";
  const result = spawnSync(cargo, ["build", "--release"], {
    cwd: root,
    stdio: "inherit",
    windowsHide: false,
  });
  if (result.error) {
    console.error(`Failed to build Tinux Launcher: ${result.error.message}`);
    console.error("Install Rust from https://rustup.rs, then run tinuxlauncher again.");
    return false;
  }
  if (result.status !== 0) {
    console.error("Failed to build Tinux Launcher.");
    return false;
  }
  return true;
}

function findPackageManager() {
  for (const manager of [
    { name: "pnpm", bin: process.platform === "win32" ? "pnpm.cmd" : "pnpm" },
    { name: "npm", bin: process.platform === "win32" ? "npm.cmd" : "npm" },
  ]) {
    const result = spawnSync(manager.bin, ["--version"], {
      stdio: "ignore",
      windowsHide: true,
    });
    if (!result.error && result.status === 0) {
      return manager;
    }
  }
  return null;
}
