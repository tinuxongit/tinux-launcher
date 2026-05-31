#!/usr/bin/env node

const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const GITHUB_SPEC = "github:tinuxongit/tinux-launcher";
const exeName = process.platform === "win32" ? "tinux-launcher.exe" : "tinux-launcher";
const root = path.resolve(__dirname, "..", "..");
const bin = path.join(root, "target", "release", exeName);
const icon = path.join(root, "assets", "tinux-icon.ico");
const args = process.argv.slice(2);
const WINDOWS_OWN_CONSOLE_ARG = "--tinux-own-console";
const WINDOWS_TERMINAL_PROFILE = "Tinux Launcher";

if (args[0] === "update" || args[0] === "--update") {
  if (installBinary(["--update"])) {
    process.exit(0);
  }
  console.error("Update failed. You can reinstall manually with:");
  console.error(`  pnpm add -g ${GITHUB_SPEC}`);
  process.exit(1);
}

if (args[0] === "--version" || args[0] === "-v") {
  if (!fs.existsSync(bin)) {
    installBinary() || buildRelease();
  }
  if (fs.existsSync(bin)) {
    const result = spawnSync(bin, ["--version"], {
      encoding: "utf8",
      windowsHide: true,
    });
    if (!result.error && result.status === 0 && result.stdout.trim()) {
      console.log(result.stdout.trim());
      process.exit(0);
    }
  }
  console.log(require("../../package.json").version);
  process.exit(0);
}

if (!fs.existsSync(bin)) {
  console.error("Tinux Launcher binary is missing; installing it now...");
  if (!installBinary() && !buildRelease()) {
    process.exit(1);
  }
}

if (process.platform === "win32" && args.length === 0 && !process.env.TINUX_INLINE) {
  if (spawnDetachedWindow()) {
    process.exit(0);
  }
  process.exit(1);
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

function installBinary(extraArgs = []) {
  const result = spawnSync(process.execPath, ["npm/scripts/install-binary.js", ...extraArgs], {
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

function spawnDetachedWindow() {
  if (installWindowsTerminalProfile()) {
    const wt = spawnSync(
      "wt.exe",
      [
        "-w",
        "-1",
        "-p",
        WINDOWS_TERMINAL_PROFILE,
      ],
      {
        stdio: "ignore",
        windowsHide: true,
      },
    );
    if (!wt.error && wt.status === 0) {
      return true;
    }
  }

  const result = spawnSync(
    "cmd.exe",
    [
      "/d",
      "/c",
      "start",
      "Tinux Launcher",
      "/D",
      path.dirname(bin),
      bin,
      WINDOWS_OWN_CONSOLE_ARG,
    ],
    {
      stdio: "ignore",
      windowsHide: true,
    },
  );
  if (result.error) {
    console.error(`Failed to open Tinux Launcher: ${result.error.message}`);
    return false;
  }
  return result.status === 0;
}

function installWindowsTerminalProfile() {
  if (!process.env.LOCALAPPDATA || !fs.existsSync(icon)) {
    return false;
  }
  try {
    const dir = path.join(
      process.env.LOCALAPPDATA,
      "Microsoft",
      "Windows Terminal",
      "Fragments",
      "TinuxLauncher",
    );
    fs.mkdirSync(dir, { recursive: true });
    const profileIcon = path.join(dir, "tinux-icon.ico");
    fs.copyFileSync(icon, profileIcon);
    const fragment = {
      profiles: [
        {
          guid: "{8f5d7c45-c75f-4c5d-a151-8e3b6a19f1a5}",
          name: WINDOWS_TERMINAL_PROFILE,
          commandline: `${quoteWindowsArg(bin)} ${WINDOWS_OWN_CONSOLE_ARG}`,
          startingDirectory: path.dirname(bin),
          icon: profileIcon,
          suppressApplicationTitle: true,
        },
      ],
    };
    fs.writeFileSync(
      path.join(dir, "tinux-launcher.json"),
      `${JSON.stringify(fragment, null, 2)}\n`,
    );
    return true;
  } catch (_) {
    return false;
  }
}

function quoteWindowsArg(value) {
  return `"${String(value).replace(/"/g, '\\"')}"`;
}
