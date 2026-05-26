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
  if (fs.existsSync(bin)) {
    try {
      fs.rmSync(bin, { force: true });
    } catch (error) {
      console.error(`Could not remove old binary at ${bin}: ${error.message}`);
      console.error("If Tinux Launcher is currently running, close it and try again.");
      process.exit(1);
    }
  }
  if (installBinary()) {
    console.log("Tinux Launcher is up to date.");
    process.exit(0);
  }
  console.error("Update failed. You can reinstall manually with:");
  console.error(`  pnpm add -g ${GITHUB_SPEC}`);
  process.exit(1);
}

if (args[0] === "--version" || args[0] === "-v") {
  const pkg = require("../../package.json");
  console.log(pkg.version);
  process.exit(0);
}

if (!fs.existsSync(bin)) {
  console.error("Tinux Launcher binary is missing; installing it now...");
  if (!installBinary() && !buildRelease()) {
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

function installBinary() {
  const result = spawnSync(process.execPath, ["npm/scripts/install-binary.js"], {
    cwd: root,
    stdio: "inherit",
    windowsHide: false,
  });
  return !result.error && result.status === 0;
}

function buildRelease() {
  console.error("Falling back to building Tinux Launcher from source.");
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

