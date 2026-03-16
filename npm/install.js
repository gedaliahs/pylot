const { execSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const https = require("https");

const REPO = "gedaliahs/pylot";
const BIN_DIR = path.join(__dirname, "bin");
const BIN_PATH = path.join(BIN_DIR, "pylot");

const PLATFORM_MAP = {
  darwin: "macos",
  linux: "linux",
};

const ARCH_MAP = {
  x64: "x86_64",
  arm64: "aarch64",
};

function getTarget() {
  const platform = PLATFORM_MAP[process.platform];
  const arch = ARCH_MAP[process.arch];

  if (!platform || !arch) {
    console.error(
      `Unsupported platform: ${process.platform} ${process.arch}`
    );
    process.exit(1);
  }

  return `${arch}-${platform}`;
}

function fetch(url) {
  return new Promise((resolve, reject) => {
    https
      .get(url, { headers: { "User-Agent": "pylot-npm" } }, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          return fetch(res.headers.location).then(resolve).catch(reject);
        }
        if (res.statusCode !== 200) {
          return reject(new Error(`HTTP ${res.statusCode} for ${url}`));
        }
        const chunks = [];
        res.on("data", (chunk) => chunks.push(chunk));
        res.on("end", () => resolve(Buffer.concat(chunks)));
        res.on("error", reject);
      })
      .on("error", reject);
  });
}

async function getLatestVersion() {
  const data = await fetch(
    `https://api.github.com/repos/${REPO}/releases/latest`
  );
  const release = JSON.parse(data.toString());
  return release.tag_name;
}

async function downloadBinary(version, target) {
  const archiveName = `pylot-${version}-${target}.tar.gz`;
  const url = `https://github.com/${REPO}/releases/download/${version}/${archiveName}`;

  console.log(`Downloading pylot ${version} for ${target}...`);

  const data = await fetch(url);
  const tmpFile = path.join(__dirname, archiveName);
  fs.writeFileSync(tmpFile, data);

  fs.mkdirSync(BIN_DIR, { recursive: true });
  execSync(`tar -xzf "${tmpFile}" -C "${BIN_DIR}"`, { stdio: "inherit" });
  fs.unlinkSync(tmpFile);
  fs.chmodSync(BIN_PATH, 0o755);

  console.log(`pylot ${version} installed successfully.`);
}

async function main() {
  try {
    const target = getTarget();
    const version = await getLatestVersion();
    await downloadBinary(version, target);
  } catch (err) {
    console.error("Failed to install pylot:", err.message);
    process.exit(1);
  }
}

main();
