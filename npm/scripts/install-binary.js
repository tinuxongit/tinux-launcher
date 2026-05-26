const fs = require("node:fs");
const https = require("node:https");
const path = require("node:path");

const REPO = "tinuxongit/tinux-launcher";
const root = path.resolve(__dirname, "..", "..");
const exeName = process.platform === "win32" ? "tinux-launcher.exe" : "tinux-launcher";
const bin = path.join(root, "target", "release", exeName);

install().catch((error) => {
  console.error(error.message);
  process.exit(1);
});

async function install() {
  if (fs.existsSync(bin)) {
    return;
  }

  const asset = assetName();
  const url = `https://github.com/${REPO}/releases/latest/download/${asset}`;
  fs.mkdirSync(path.dirname(bin), { recursive: true });

  process.stderr.write(`Downloading Tinux Launcher binary for ${process.platform}/${process.arch}...\n`);
  try {
    await download(url, bin);
    if (process.platform !== "win32") {
      fs.chmodSync(bin, 0o755);
    }
  } catch (error) {
    try {
      fs.rmSync(bin, { force: true });
    } catch (_) {}
    throw new Error(`Could not download prebuilt binary: ${error.message}`);
  }
}

function assetName() {
  const platform = {
    win32: "windows",
    linux: "linux",
    darwin: "macos",
  }[process.platform];
  const arch = {
    x64: "x64",
    arm64: "arm64",
  }[process.arch];

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
        file.on("finish", () => file.close(resolve));
        file.on("error", reject);
      },
    );
    request.on("error", reject);
  });
}
