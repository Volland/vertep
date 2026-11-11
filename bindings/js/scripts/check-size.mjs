#!/usr/bin/env node
// The bundle size budget of @vertep/web, measured on the built artifacts with gzip -9:
//   storage-free  core WebAssembly + its glue + the client and worker JavaScript   <= 600 KB
//   journal       journal WebAssembly + its glue + the client and worker JavaScript <= 2.5 MB
// Exits 1 and reports the size and the budget when a build exceeds its budget. KB = 1000 bytes.
// VERTEP_BUDGET_CORE_KB and VERTEP_BUDGET_JOURNAL_KB override the budgets (tests).
// @tg: implements:: [[openspec:wasm-bindings#Bundle Size Budget]]
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";

const web = fileURLToPath(new URL("../packages/web/", import.meta.url));

export const BUDGETS = {
  core: Number(process.env.VERTEP_BUDGET_CORE_KB ?? 600) * 1000,
  journal: Number(process.env.VERTEP_BUDGET_JOURNAL_KB ?? 2500) * 1000,
};

const gz = (file) => gzipSync(readFileSync(file), { level: 9 }).length;

/** The gzip sizes of the two builds, in bytes. */
export function measure(dir = web) {
  const js = readdirSync(join(dir, "dist"))
    .filter((f) => f.endsWith(".js"))
    .reduce((n, f) => n + gz(join(dir, "dist", f)), 0);
  const build = (kind) => gz(join(dir, "wasm", kind, "vertep_wasm_bg.wasm")) + gz(join(dir, "wasm", kind, "vertep_wasm.js")) + js;
  return { core: build("core"), journal: build("journal") };
}

/** The budget violations of `sizes`. */
export function violations(sizes, budgets = BUDGETS) {
  return Object.entries(budgets)
    .filter(([k, b]) => sizes[k] > b)
    .map(([k, b]) => `${k}: ${(sizes[k] / 1000).toFixed(1)} KB gzip exceeds the budget of ${(b / 1000).toFixed(0)} KB`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const sizes = measure();
  for (const [k, n] of Object.entries(sizes)) {
    console.log(`${k}: ${(n / 1000).toFixed(1)} KB gzip (budget ${(BUDGETS[k] / 1000).toFixed(0)} KB)`);
  }
  const bad = violations(sizes);
  for (const v of bad) console.error(v);
  process.exit(bad.length ? 1 : 0);
}
