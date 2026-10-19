#!/usr/bin/env node
// The app version lives in one place: `workspace.package.version` in the root
// Cargo.toml. Everything else derives from it (crates, Tauri bundles, Nix).
// This script keeps it that way and guards releases (see docs/release.md).
//
// Usage:
//   node scripts/version.mjs                       print the version
//   node scripts/version.mjs check [--tag <tag>]   verify the single source of
//                                                  truth (and that a release
//                                                  tag matches it)
//   node scripts/version.mjs assets <file|dir>...  verify that release assets
//                                                  carry the version
//   node scripts/version.mjs bump <x.y.z>          set the version (Cargo.toml
//                                                  and Cargo.lock)
//   node scripts/version.mjs tag [--push]          tag the version on main
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const CARGO_TOML = join(root, "Cargo.toml");
const CARGO_LOCK = join(root, "Cargo.lock");
// Installers (MSI in particular) only accept numeric versions.
const SEMVER = /^(\d+)\.(\d+)\.(\d+)$/;
// Files that must not declare a version of their own: a second copy is what
// shipped 0.1.0 installers for v0.2.0 and v0.2.1 (issue #27).
const NO_VERSION = ["crates/daemon/tauri.conf.json", "crates/daemon/tauri.release.conf.json", "ui/package.json"];
const ASSET_EXT = [".deb", ".dmg", ".msi", ".exe", ".AppImage", ".rpm"];

class Failure extends Error {}

function workspaceSection(toml) {
  const m = /^\[workspace\.package\][^\S\n]*\n([\s\S]*?)(?=^\[|(?![\s\S]))/m.exec(toml);
  if (!m) throw new Failure("Cargo.toml has no [workspace.package] section");
  return { start: m.index + m[0].length - m[1].length, body: m[1] };
}

function readVersion() {
  const { body } = workspaceSection(readFileSync(CARGO_TOML, "utf8"));
  const m = /^version\s*=\s*"([^"]+)"/m.exec(body);
  if (!m) throw new Failure("Cargo.toml: workspace.package.version is missing");
  return m[1];
}

function workspaceCrates() {
  const toml = readFileSync(CARGO_TOML, "utf8");
  const members = /^members\s*=\s*\[([^\]]*)\]/m.exec(toml)?.[1] ?? "";
  return [...members.matchAll(/"([^"]+)"/g)].map(([, dir]) => {
    const manifest = readFileSync(join(root, dir, "Cargo.toml"), "utf8");
    return { dir, manifest, name: /^name\s*=\s*"([^"]+)"/m.exec(manifest)[1] };
  });
}

function lockedVersions() {
  const lock = readFileSync(CARGO_LOCK, "utf8");
  return new Map(
    [...lock.matchAll(/^\[\[package\]\]\nname = "([^"]+)"\nversion = "([^"]+)"/gm)].map(([, n, v]) => [n, v]),
  );
}

// All problems with the single source of truth, as messages.
function problems(version) {
  const out = [];
  if (!SEMVER.test(version)) out.push(`Cargo.toml: version "${version}" is not of the form x.y.z`);
  const locked = lockedVersions();
  for (const { dir, manifest, name } of workspaceCrates()) {
    if (!/^version\.workspace\s*=\s*true/m.test(manifest)) {
      out.push(`${dir}/Cargo.toml: use \`version.workspace = true\` instead of an own version`);
    }
    if (locked.get(name) !== version) {
      out.push(`Cargo.lock: ${name} is ${locked.get(name) ?? "missing"}, expected ${version} (run \`cargo check\`)`);
    }
  }
  for (const file of NO_VERSION) {
    const path = join(root, file);
    if (existsSync(path) && "version" in JSON.parse(readFileSync(path, "utf8"))) {
      out.push(`${file}: remove "version", it is taken from Cargo.toml`);
    }
  }
  return out;
}

function check(args) {
  const version = readVersion();
  const out = problems(version);
  const ti = args.indexOf("--tag");
  const tag = ti !== -1 ? args[ti + 1] : null;
  if (tag && tag !== `v${version}`) {
    out.push(`tag ${tag} does not match Cargo.toml version ${version}: bump the version before tagging (node scripts/version.mjs bump <x.y.z>)`);
  }
  if (out.length) throw new Failure(out.join("\n"));
  console.log(`version ${version}: ok${tag ? ` (tag ${tag})` : ""}`);
}

function listAssets(paths) {
  return paths.filter((p) => existsSync(p)).flatMap((p) =>
    statSync(p).isDirectory() ? listAssets(readdirSync(p).map((f) => join(p, f))) : [p],
  ).filter((p) => ASSET_EXT.some((ext) => p.endsWith(ext)));
}

function assets(paths) {
  const version = readVersion();
  const files = listAssets(paths);
  if (!files.length) throw new Failure(`no release assets found in ${paths.join(", ")}`);
  const out = [];
  for (const file of files) {
    // Tauri names bundles <product>_<version>_<arch>…, e.g. aw-manual_0.2.1_amd64.deb.
    if (!basename(file).includes(`_${version}_`)) out.push(`${file}: file name does not carry version ${version}`);
    if (file.endsWith(".deb")) {
      let pkgVersion;
      try {
        pkgVersion = execFileSync("dpkg-deb", ["-f", file, "Version"], { encoding: "utf8" }).trim();
      } catch (e) {
        // Without dpkg-deb (macOS, Windows) the name check still applies.
        if (e.code === "ENOENT") continue;
        out.push(`${file}: cannot read the package version (${e.message.split("\n")[0]})`);
        continue;
      }
      if (pkgVersion !== version) out.push(`${file}: package version is ${pkgVersion}, expected ${version}`);
    }
  }
  if (out.length) throw new Failure(out.join("\n"));
  files.forEach((f) => console.log(`${basename(f)}: ok`));
}

function git(...args) {
  return execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
}

function newer(a, b) {
  const [pa, pb] = [a, b].map((v) => SEMVER.exec(v).slice(1).map(Number));
  for (let i = 0; i < 3; i++) if (pa[i] !== pb[i]) return pa[i] > pb[i];
  return false;
}

function bump(args) {
  const version = args[0];
  const current = readVersion();
  if (!version || !SEMVER.test(version)) throw new Failure("usage: node scripts/version.mjs bump <x.y.z>");
  if (!newer(version, current)) throw new Failure(`${version} is not newer than the current version ${current}`);
  const toml = readFileSync(CARGO_TOML, "utf8");
  const { start, body } = workspaceSection(toml);
  const bumped = body.replace(/^(version\s*=\s*")[^"]+"/m, `$1${version}"`);
  writeFileSync(CARGO_TOML, toml.slice(0, start) + bumped + toml.slice(start + body.length));
  // Only the workspace crates change; no other dependency is touched.
  const names = new Set(workspaceCrates().map((c) => c.name));
  const lock = readFileSync(CARGO_LOCK, "utf8").replace(
    /^(\[\[package\]\]\nname = "([^"]+)"\nversion = ")[^"]+"/gm,
    (m, head, name) => (names.has(name) ? `${head}${version}"` : m),
  );
  writeFileSync(CARGO_LOCK, lock);
  const left = problems(version);
  if (left.length) throw new Failure(left.join("\n"));
  console.log(`version ${current} -> ${version}: commit Cargo.toml and Cargo.lock and merge to main, then run \`node scripts/version.mjs tag\``);
}

// Tags the version that is on main, so tag and version cannot disagree.
function tag(args) {
  const version = readVersion();
  const out = problems(version);
  if (out.length) throw new Failure(out.join("\n"));
  if (git("status", "--porcelain")) throw new Failure("the working tree has uncommitted changes");
  const branch = git("rev-parse", "--abbrev-ref", "HEAD");
  if (branch !== "main") throw new Failure(`releases are tagged on main, not ${branch}`);
  git("fetch", "--quiet", "--tags", "origin", "main");
  if (git("rev-parse", "HEAD") !== git("rev-parse", "origin/main")) {
    throw new Failure("main is not in sync with origin/main: pull first");
  }
  if (git("tag", "--list", `v${version}`)) {
    throw new Failure(`v${version} is already released: bump the version first (node scripts/version.mjs bump <x.y.z>)`);
  }
  const older = git("tag", "--list", "v*").split("\n").map((t) => t.slice(1)).filter((v) => SEMVER.test(v));
  const latest = older.find((v) => !older.some((o) => newer(o, v)));
  if (latest && !newer(version, latest)) throw new Failure(`${version} is not newer than the latest release v${latest}`);
  git("tag", "--annotate", `v${version}`, "-m", `v${version}`);
  if (args.includes("--push")) {
    git("push", "origin", `v${version}`);
    console.log(`tagged and pushed v${version}; CI builds the draft release`);
  } else {
    console.log(`tagged v${version}; push with: git push origin v${version}`);
  }
}

try {
  const [cmd = "print", ...args] = process.argv.slice(2);
  if (cmd === "print") console.log(readVersion());
  else if (cmd === "check") check(args);
  else if (cmd === "assets") assets(args);
  else if (cmd === "bump") bump(args);
  else if (cmd === "tag") tag(args);
  else throw new Failure(`unknown command ${cmd}`);
} catch (e) {
  if (!(e instanceof Failure)) throw e;
  console.error(e.message.split("\n").map((l) => `error: ${l}`).join("\n"));
  process.exit(1);
}
