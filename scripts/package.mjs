#!/usr/bin/env node
// Packages the plugin into one archive per platform, under the asset names
// Lithe Store's release tracker recognizes (docs/publishing.md there):
//
//   dist/rust-support-<version>-darwin.zip         Apple Silicon and Intel Macs
//   dist/rust-support-<version>-windows-x64.zip    Windows x64
//   dist/rust-support-<version>-windows-arm64.zip  Windows ARM64
//
// Each holds the package directory and the backend, built beforehand by
// scripts/build-backend.mjs into .artifacts/backend-<platform>/: bin/backend, a
// universal binary, on macOS and bin/backend.exe on Windows, with
// THIRD_PARTY_LICENSES.txt next to plugin.json. The backend is both the plugin
// backend (it decides how rust-analyzer starts) and the Cargo build provider, so
// the Windows archives' plugin.json names bin/backend.exe in both places; all
// three declare the same version, pluginAPIVersion and requires, as the Store
// requires. Everything else, Lithe running the user's own cargo, rustup and
// rust-analyzer, is the same on every platform. Nothing is downloaded at build or
// install time: CodeLLDB is downloaded by Lithe the first time a debug session
// needs it, from the pinned release the manifest's command installer names.
// Packaging needs POSIX zip, so it runs on macOS or Linux.
//
// Usage: node scripts/package.mjs [--expect-tag <tag>] [--platforms darwin,windows-x64,windows-arm64]

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";

const PACKAGE_DIR = "io.github.lithe-idea.rust-support";
const ASSET_PREFIX = "rust-support";
const BACKEND_PATH = "bin/backend";
const PLATFORMS = {
  darwin: { program: "backend", builtOn: "macOS" },
  "windows-x64": { program: "backend.exe", builtOn: "Windows" },
  "windows-arm64": { program: "backend.exe", builtOn: "Windows" },
};

const { values } = parseArgs({
  options: {
    "expect-tag": { type: "string" },
    platforms: { type: "string", default: Object.keys(PLATFORMS).join(",") },
  },
});
const platforms = values.platforms.split(",").filter(Boolean);

const source = readFileSync(`${PACKAGE_DIR}/plugin.json`, "utf8");
const manifest = JSON.parse(source);
const problems = [];
if (manifest.id !== PACKAGE_DIR) problems.push(`plugin.json id ${manifest.id} must equal the directory ${PACKAGE_DIR}`);
if (!/^[0-9]+\.[0-9]+\.[0-9]+$/.test(manifest.version)) problems.push(`version ${manifest.version} is not MAJOR.MINOR.PATCH`);
if (manifest.vendor?.signatureRequirement !== "none") problems.push('vendor.signatureRequirement must be "none"');
if (manifest.id === "dev.lithe" || manifest.id.startsWith("dev.lithe.")) problems.push("dev.lithe.* is reserved for official plugins");
for (const [where, launch] of launches(manifest)) {
  if (launch?.kind !== "packaged" || launch.path !== BACKEND_PATH) {
    problems.push(`plugin.json ${where} must be {"kind": "packaged", "path": "${BACKEND_PATH}"}`);
  }
}
const cargoVersion = /^\[package\][^[]*?^version = "([^"]+)"/ms.exec(readFileSync("backend/Cargo.toml", "utf8"))?.[1];
if (cargoVersion !== manifest.version) problems.push(`backend/Cargo.toml version ${cargoVersion} must equal plugin.json version ${manifest.version}`);
if (values["expect-tag"] !== undefined && values["expect-tag"] !== `v${manifest.version}`) {
  problems.push(`tag ${values["expect-tag"]} does not match plugin.json version ${manifest.version} (expected v${manifest.version})`);
}
for (const platform of platforms) {
  const spec = PLATFORMS[platform];
  if (!spec) {
    problems.push(`unknown platform ${platform}; use ${Object.keys(PLATFORMS).join(", ")}`);
    continue;
  }
  for (const file of [spec.program, "THIRD_PARTY_LICENSES.txt"]) {
    if (!existsSync(join(".artifacts", `backend-${platform}`, file))) {
      problems.push(`.artifacts/backend-${platform}/${file} is missing: run node scripts/build-backend.mjs ${platform} on ${spec.builtOn}`);
    }
  }
}
if (problems.length > 0) {
  for (const problem of problems) console.error(`✗ ${problem}`);
  process.exit(1);
}

rmSync("dist", { recursive: true, force: true });
mkdirSync("dist", { recursive: true });
const staging = resolve(".artifacts", "package");
for (const platform of platforms) {
  const { program } = PLATFORMS[platform];
  const built = join(".artifacts", `backend-${platform}`);
  rmSync(staging, { recursive: true, force: true });
  const root = join(staging, PACKAGE_DIR);
  cpSync(PACKAGE_DIR, root, { recursive: true, filter: (path) => !path.endsWith(".DS_Store") });
  mkdirSync(join(root, "bin"), { recursive: true });
  cpSync(join(built, program), join(root, "bin", program));
  // A downloaded build artifact loses its mode, and the archive keeps this one.
  chmodSync(join(root, "bin", program), 0o755);
  cpSync(join(built, "THIRD_PARTY_LICENSES.txt"), join(root, "THIRD_PARTY_LICENSES.txt"));
  if (program !== "backend") {
    const forPlatform = JSON.parse(source);
    for (const [, launch] of launches(forPlatform)) launch.path = `bin/${program}`;
    writeFileSync(join(root, "plugin.json"), `${JSON.stringify(forPlatform, null, 2)}\n`);
  }
  const name = `${ASSET_PREFIX}-${manifest.version}-${platform}.zip`;
  // -X drops extra file attributes; -y stores a symbolic link as a link so
  // Lithe refuses it instead of the archive silently carrying its target.
  execFileSync("zip", ["-qrXy", resolve("dist", name), PACKAGE_DIR], { cwd: staging, timeout: 60_000 });
  const sha256 = createHash("sha256").update(readFileSync(resolve("dist", name))).digest("hex");
  console.log(`dist/${name} ${sha256}`);
}
rmSync(staging, { recursive: true, force: true });

/** Every packaged launch of the backend program: the plugin backend and each build provider. */
function launches(entry) {
  return [
    ["backend.launch", entry.backend?.launch],
    ...(entry.buildSystems ?? []).map((system) => [`buildSystems[${system.id}].provider.launch`, system.provider?.launch]),
  ];
}
