"use strict";

const fs = require("fs");
const https = require("https");
const path = require("path");
const { execSync } = require("child_process");

const VERSION = "0.1.2";
const RELEASE_URL = "https://github.com/neul-labs/rpytest/releases/download";

function getTarget() {
  const platform = process.platform;
  const arch = process.arch;

  let targetArch;
  if (arch === "x64") {
    targetArch = "x86_64";
  } else if (arch === "arm64") {
    targetArch = "aarch64";
  } else {
    throw new Error(`Unsupported architecture: ${arch}`);
  }

  if (platform === "darwin") {
    return `${targetArch}-apple-darwin`;
  } else if (platform === "linux") {
    return `${targetArch}-unknown-linux-gnu`;
  } else {
    throw new Error(`Unsupported platform: ${platform}. rpytest supports macOS and Linux.`);
  }
}

function downloadFile(url, dest) {
  return new Promise((resolve, reject) => {
    const file = fs.createWriteStream(dest);
    https
      .get(url, { headers: { "User-Agent": "rpytest-npm-installer" } }, (response) => {
        if (response.statusCode === 302 || response.statusCode === 301) {
          downloadFile(response.headers.location, dest).then(resolve).catch(reject);
          return;
        }
        if (response.statusCode !== 200) {
          reject(new Error(`Download failed with status ${response.statusCode}: ${url}`));
          return;
        }
        response.pipe(file);
        file.on("finish", () => {
          file.close(resolve);
        });
      })
      .on("error", (err) => {
        fs.unlink(dest, () => {});
        reject(err);
      });
  });
}

async function install() {
  const binDir = path.join(__dirname, "bin");
  const target = getTarget();
  const platformBinary = path.join(binDir, `rpytest-${target}`);

  // Skip if platform binary already exists (bundled in the package)
  if (fs.existsSync(platformBinary)) {
    console.log(`rpytest ${VERSION} for ${target} already present.`);
    return;
  }

  const archiveName = `rpytest-${VERSION}-${target}.tar.gz`;
  const url = `${RELEASE_URL}/v${VERSION}/${archiveName}`;
  const archivePath = path.join(binDir, archiveName);

  console.log(`Downloading rpytest ${VERSION} for ${target}...`);
  console.log(`URL: ${url}`);

  fs.mkdirSync(binDir, { recursive: true });

  try {
    await downloadFile(url, archivePath);
  } catch (err) {
    if (err.message && err.message.includes("404")) {
      console.error(
        `No prebuilt binary available for ${target}.\n` +
          "Please build from source: cargo install --path crates/rpytest"
      );
      process.exit(1);
    }
    throw err;
  }

  // Extract to temp, then move to platform-specific name
  const tmpDir = path.join(binDir, ".tmp-extract");
  fs.mkdirSync(tmpDir, { recursive: true });

  console.log("Extracting binary...");
  try {
    execSync(`tar xzf "${archivePath}" -C "${tmpDir}" --strip-components=1`, { stdio: "inherit" });
  } catch (err) {
    console.error("Failed to extract archive:", err.message);
    process.exit(1);
  }

  // Find the extracted binary and move it to platform-specific name
  const extractedFiles = fs.readdirSync(tmpDir);
  const binaryFile = extractedFiles.find((f) => f === "rpytest");
  if (!binaryFile) {
    console.error("Binary not found in archive");
    process.exit(1);
  }

  fs.renameSync(path.join(tmpDir, binaryFile), platformBinary);
  fs.chmodSync(platformBinary, 0o755);

  // Cleanup
  fs.rmSync(tmpDir, { recursive: true, force: true });
  fs.unlinkSync(archivePath);

  console.log(`Installed rpytest ${VERSION} at ${platformBinary}`);
}

install().catch((err) => {
  console.error("Install failed:", err.message);
  process.exit(1);
});
