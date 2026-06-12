const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const https = require("node:https");
const path = require("node:path");

const REPO = "tinuxongit/tinux-launcher";
const root = path.resolve(__dirname, "..", "..");
const exeName = process.platform === "win32" ? "tinux-launcher.exe" : "tinux-launcher";
const bin = path.join(root, "target", "release", exeName);
const updateMode = process.argv.includes("--update");
const forceMode = process.argv.includes("--force");

install({ update: updateMode, force: forceMode }).catch((error) => {
  console.error(error.message);
  process.exit(1);
});

async function install({ update = false, force = false } = {}) {
  if (update) {
    await updateBinary();
    return;
  }

  if (!force && fs.existsSync(bin)) {
    return;
  }

  await downloadLatestBinary(bin);
}

async function updateBinary() {
  const latest = normalizeVersion(await fetchLatestVersion());
  const current = currentBinaryVersion();

  if (current && normalizeVersion(current) === latest) {
    console.log(`Tinux Launcher is up to date (v${latest}).`);
    return;
  }

  const from = current ? ` from v${normalizeVersion(current)}` : "";
  process.stderr.write(`Updating Tinux Launcher${from} to v${latest}...\n`);
  await downloadLatestBinary(bin);

  const updated = currentBinaryVersion();
  if (updated && normalizeVersion(updated) !== latest) {
    throw new Error(`Downloaded binary reports v${normalizeVersion(updated)}, expected v${latest}`);
  }

  console.log(`Tinux Launcher updated to v${latest}.`);
}

async function downloadLatestBinary(dest) {
  const asset = assetName();
  const url = `https://github.com/${REPO}/releases/latest/download/${asset}`;
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  // Always download to a temp file first — an interrupted download must never
  // leave a half-written binary at the final path (it would be mistaken for a
  // working install and never re-downloaded).
  const downloadPath = path.join(
    path.dirname(dest),
    `${path.basename(dest)}.${process.pid}.${Date.now()}.download`,
  );

  process.stderr.write(`Downloading Tinux Launcher binary for ${process.platform}/${process.arch}...\n`);
  try {
    await download(url, downloadPath);
    if (process.platform !== "win32") {
      fs.chmodSync(downloadPath, 0o755);
    }
    replaceFile(downloadPath, dest);
  } catch (error) {
    try {
      fs.rmSync(downloadPath, { force: true });
    } catch (_) {}
    throw new Error(`Could not download prebuilt binary: ${error.message}`);
  }
}

function currentBinaryVersion() {
  if (!fs.existsSync(bin)) {
    return null;
  }
  const result = spawnSync(bin, ["--version"], {
    encoding: "utf8",
    windowsHide: true,
  });
  if (result.error || result.status !== 0) {
    return null;
  }
  return result.stdout.trim() || null;
}

function normalizeVersion(version) {
  return String(version).trim().replace(/^v/i, "");
}

async function fetchLatestVersion() {
  return fetchLatestTag(`https://github.com/${REPO}/releases/latest`);
}

function replaceFile(temp, dest) {
  if (!fs.existsSync(dest)) {
    fs.renameSync(temp, dest);
    return;
  }

  const backup = `${dest}.old-${process.pid}-${Date.now()}`;
  try {
    fs.renameSync(dest, backup);
  } catch (error) {
    throw new Error(
      `Could not replace old binary at ${dest}: ${error.message}. ` +
      "If Tinux Launcher is currently running, close it and try again.",
    );
  }

  try {
    fs.renameSync(temp, dest);
  } catch (error) {
    try {
      fs.renameSync(backup, dest);
    } catch (_) {}
    throw error;
  }

  try {
    fs.rmSync(backup, { force: true });
  } catch (_) {}
}

function assetName() {
  const platform = {
    win32: "windows",
    linux: "linux",
    darwin: "macos",
  }[process.platform];
  let arch = {
    x64: "x64",
    arm64: "arm64",
  }[process.arch];

  // Windows on ARM runs x64 binaries through emulation and there is no
  // native arm64 release asset, so use the x64 one.
  if (process.platform === "win32" && arch === "arm64") {
    arch = "x64";
  }

  if (!platform || !arch) {
    throw new Error(`No prebuilt binary for ${process.platform}/${process.arch}`);
  }

  const ext = process.platform === "win32" ? ".exe" : "";
  return `tinux-launcher-${platform}-${arch}${ext}`;
}

function download(url, dest) {
  return new Promise((resolve, reject) => {
    const request = https.get(
      url,
      {
        headers: {
          "User-Agent": "tinuxlauncher-installer",
        },
      },
      (response) => {
        if (
          response.statusCode >= 300 &&
          response.statusCode < 400 &&
          response.headers.location
        ) {
          response.resume();
          download(response.headers.location, dest).then(resolve, reject);
          return;
        }

        if (response.statusCode !== 200) {
          response.resume();
          reject(new Error(`HTTP ${response.statusCode} from ${url}`));
          return;
        }

        const file = fs.createWriteStream(dest);
        response.pipe(file);
        // Without this, a connection dropped mid-download never settles the
        // promise and the install hangs forever.
        response.on("error", (error) => {
          file.destroy();
          reject(error);
        });
        file.on("finish", () => file.close(resolve));
        file.on("error", reject);
      },
    );
    request.on("error", reject);
  });
}

function fetchLatestTag(url, redirectCount = 0) {
  return new Promise((resolve, reject) => {
    if (redirectCount > 5) {
      reject(new Error("too many redirects while checking latest release"));
      return;
    }

    const request = https.get(
      url,
      {
        headers: {
          "User-Agent": "tinuxlauncher-installer",
        },
      },
      (response) => {
        if (
          response.statusCode >= 300 &&
          response.statusCode < 400 &&
          response.headers.location
        ) {
          const location = new URL(response.headers.location, url).toString();
          const match = location.match(/\/releases\/tag\/([^/?#]+)/);
          response.resume();
          if (match) {
            resolve(decodeURIComponent(match[1]));
            return;
          }
          fetchLatestTag(location, redirectCount + 1).then(resolve, reject);
          return;
        }

        response.resume();
        reject(new Error(`could not resolve latest release tag from ${url}`));
      },
    );
    request.on("error", reject);
  });
}
