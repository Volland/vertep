// The npm packages: contents, installation, the separate ECMAScript package, versions, budget.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { ORDER, workspace } from "./helpers.mjs";
import { createActorSystem } from "../dist/index.js";
import { ecmascript } from "../../dm-ecma/dist/index.js";
import { crateVersion, mismatches, packages } from "../../../scripts/check-version.mjs";
import { measure, violations } from "../../../scripts/check-size.mjs";

const npm = (args, cwd) => execFileSync("npm", args, { cwd, encoding: "utf8", env: { ...process.env, npm_config_audit: "false", npm_config_fund: "false" } });

const ECMA_CHART = `<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a" datamodel="ecmascript">
  <datamodel><data id="n" expr="1"/></datamodel>
  <state id="a"><transition event="go" cond="n === 1" target="b"><assign location="n" expr="n + 41"/></transition></state>
  <state id="b"/>
</scxml>`;

// @tg: verifies:: [[openspec:wasm-bindings#npm Package Distribution#Install and import]] {level: e2e}
test("install and import", () => {
  const base = join(workspace, ".test-tmp");
  mkdirSync(base, { recursive: true });
  const dir = mkdtempSync(join(base, "install-"));
  const pack = JSON.parse(npm(["pack", "--json", "--pack-destination", dir, "-w", "@vertep/web"], workspace))[0];
  const files = pack.files.map((f) => f.path);
  for (const f of ["dist/index.js", "dist/index.d.ts", "dist/worker.js", "wasm/core/vertep_wasm_bg.wasm", "wasm/core/vertep_wasm.js", "wasm/journal/vertep_wasm_bg.wasm"]) {
    assert.ok(files.includes(f), `${f} is in the package`);
  }
  assert.ok(!files.some((f) => f.startsWith("src/") || f.startsWith("test/")), "only build output ships");
  const app = join(dir, "app");
  mkdirSync(app);
  writeFileSync(join(app, "package.json"), JSON.stringify({ name: "app", private: true, type: "module" }));
  npm(["install", "--offline", "--no-save", join(dir, pack.filename)], app);
  writeFileSync(
    join(app, "main.js"),
    `import { createActorSystem } from "@vertep/web";
const system = await createActorSystem({ storage: "memory" });
const a = await system.spawn(${JSON.stringify(ORDER)});
await a.send({ type: "pay" });
console.log(JSON.stringify((await a.getSnapshot()).configuration));
await system.terminate();
`,
  );
  const r = spawnSync(process.execPath, ["main.js"], { cwd: app, encoding: "utf8" });
  assert.equal(r.status, 0, r.stderr);
  assert.equal(r.stdout.trim(), '["paid"]');
});

// @tg: verifies:: [[openspec:wasm-bindings#npm Package Distribution#ECMAScript datamodel is a separate package]] {level: e2e}
test("the ECMAScript datamodel is a separate package", async () => {
  const pack = JSON.parse(npm(["pack", "--dry-run", "--json", "-w", "@vertep/web"], workspace))[0];
  assert.ok(!pack.files.some((f) => f.path.includes("dm-ecma") || f.path.includes("ecma")), "no engine in the main package");
  const plain = await createActorSystem({ storage: "memory" });
  try {
    await assert.rejects(plain.spawn(ECMA_CHART), (e) => e.code === "Activation" && /ecmascript/i.test(e.message));
  } finally {
    await plain.terminate();
  }
  const withEcma = await createActorSystem({ storage: "memory", datamodel: ecmascript });
  try {
    assert.equal(withEcma.build, "custom");
    const a = await withEcma.spawn(ECMA_CHART);
    await a.send({ type: "go" });
    const s = await a.getSnapshot();
    assert.deepEqual(s.configuration, ["b"]);
  } finally {
    await withEcma.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#npm Package Distribution#Version matches the crates]] {level: unit}
test("version matches the crates", () => {
  const version = crateVersion();
  assert.deepEqual(mismatches(version, packages()), []);
  // a package at another version fails the release
  const bumped = packages().map((p, i) => (i === 0 ? { ...p, version: "9.9.9" } : p));
  assert.equal(mismatches(version, bumped).length, 1);
  const script = join(workspace, "scripts", "check-version.mjs");
  assert.equal(spawnSync(process.execPath, [script]).status, 0);
});

// @tg: verifies:: [[openspec:wasm-bindings#Bundle Size Budget#Budget exceeded fails CI]] {level: integration}
test("budget exceeded fails CI", () => {
  const sizes = measure();
  assert.ok(sizes.core > 0 && sizes.journal > sizes.core);
  const v = violations(sizes, { core: 1000, journal: sizes.journal + 1 });
  assert.equal(v.length, 1);
  assert.match(v[0], /^core: [\d.]+ KB gzip exceeds the budget of 1 KB$/);
  const script = join(workspace, "scripts", "check-size.mjs");
  const r = spawnSync(process.execPath, [script], { encoding: "utf8", env: { ...process.env, VERTEP_BUDGET_CORE_KB: "1" } });
  assert.equal(r.status, 1);
  assert.match(r.stderr, /core: [\d.]+ KB gzip exceeds the budget of 1 KB/);
});
