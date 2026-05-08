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
  const archiveName = `rpytest-${VERSION}-${target}.tar.gz`;
  const url = `${RELEASE_URL}/v${VERSION}/${archiveName}`;
  const archivePath = path.join(binDir, archiveName);
  const binaryPath = path.join(binDir, "rpytest");

  // Skip if binary already exists (e.g. bundled in the package)
  if (fs.existsSync(binaryPath)) {
    console.log("rpytest binary already present.");
    return;
  }

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

  // Extract
  console.log("Extracting binary...");
  try {
    execSync(`tar xzf "${archivePath}" -C "${binDir}" --strip-components=1`, { stdio: "inherit" });
  } catch (err) {
    console.error("Failed to extract archive:", err.message);
    process.exit(1);
  }

  // Make executable
  fs.chmodSync(binaryPath, 0o755);

  // Cleanup
  fs.unlinkSync(archivePath);

  console.log(`Installed rpytest ${VERSION} at ${binaryPath}`);
}

install().catch((err) => {
  console.error("Install failed:", err.message);
  process.exit(1);
});
