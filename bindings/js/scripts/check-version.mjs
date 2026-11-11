#!/usr/bin/env node
// Every npm package must carry the version of the Rust crates it was built from: the
// workspace version in Cargo.toml. Fails the release otherwise.
// @tg: implements:: [[openspec:wasm-bindings#npm Package Distribution]] {confidence: partial, note: "the release version gate"}
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const js = fileURLToPath(new URL("..", import.meta.url));
const root = join(js, "..", "..");

/** The workspace crate version from Cargo.toml. */
export function crateVersion(cargoToml = readFileSync(join(root, "Cargo.toml"), "utf8")) {
  const pkg = cargoToml.split("[workspace.package]")[1] ?? "";
  const m = pkg.match(/^version\s*=\s*"([^"]+)"/m);
  if (!m) throw new Error("no [workspace.package] version in Cargo.toml");
  return m[1];
}

/** The mismatches between the crate version and the packages. */
export function mismatches(version, packages) {
  const out = [];
  for (const p of packages) {
    if (p.version !== version) out.push(`${p.name} is ${p.version}, the crates are ${version}`);
    for (const [dep, range] of Object.entries(p.peerDependencies ?? {})) {
      if (dep.startsWith("@vertep/") && range !== version) out.push(`${p.name} needs ${dep}@${range}, the crates are ${version}`);
    }
  }
  return out;
}

export function packages() {
  return ["web", "dm-ecma"].map((d) => JSON.parse(readFileSync(join(js, "packages", d, "package.json"), "utf8")));
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const version = crateVersion();
  const index = readFileSync(join(js, "packages", "web", "src", "index.ts"), "utf8");
  const bad = mismatches(version, packages());
  if (!index.includes(`VERSION = "${version}"`)) bad.push(`@vertep/web's VERSION constant is not ${version}`);
  for (const b of bad) console.error(b);
  if (!bad.length) console.log(`all packages are at ${version}`);
  process.exit(bad.length ? 1 : 0);
}
