#!/usr/bin/env node
// Builds the `aw-manual` CLI and places it where the Tauri bundler expects
// the sidecar: crates/daemon/binaries/aw-manual-<target-triple>[.exe].
//
// Usage: node scripts/prepare-sidecar.mjs [--target <triple>]
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const ti = args.indexOf("--target");
let target = ti !== -1 ? args[ti + 1] : null;
if (!target) {
  const info = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  target = /host: (\S+)/.exec(info)[1];
}

function build(triple) {
  const cargoArgs = ["build", "--release", "-p", "aw-manual-cli", "--target", triple];
  console.log(`cargo ${cargoArgs.join(" ")}`);
  execFileSync("cargo", cargoArgs, { cwd: root, stdio: "inherit" });
  const exe = triple.includes("windows") ? ".exe" : "";
  return join(root, "target", triple, "release", `aw-manual${exe}`);
}

const exe = target.includes("windows") ? ".exe" : "";
const dest = join(root, "crates", "daemon", "binaries", `aw-manual-${target}${exe}`);
mkdirSync(dirname(dest), { recursive: true });
if (target === "universal-apple-darwin") {
  // macOS universal binary: build both architectures and merge them.
  const parts = ["aarch64-apple-darwin", "x86_64-apple-darwin"].map(build);
  execFileSync("lipo", ["-create", "-output", dest, ...parts], { stdio: "inherit" });
} else {
  copyFileSync(build(target), dest);
}
console.log(`sidecar: ${dest}`);
