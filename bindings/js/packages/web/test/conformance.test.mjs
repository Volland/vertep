// Conformance in WebAssembly: the differential test's generated charts and inputs, run in
// WebAssembly (the build with every datamodel, from @vertep/dm-ecma) and natively, give
// identical oracle records, which are the records the native test hands the Lean model.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { nativeBin } from "./helpers.mjs";

async function loadEcma() {
  const glue = await import(new URL("../../dm-ecma/wasm/vertep_wasm.js", import.meta.url).href);
  await glue.default({ module_or_path: readFileSync(new URL("../../dm-ecma/wasm/vertep_wasm_bg.wasm", import.meta.url)) });
  return glue;
}

// @tg: verifies:: [[openspec:wasm-bindings#Conformance Suite In WebAssembly#Traces equal native traces]] {level: integration}
test("differential traces in WebAssembly equal native traces", async () => {
  const wasm = await loadEcma();
  let compared = 0;
  for (const seed of [1, 2]) {
    const cases = execFileSync(nativeBin, ["gen-diff", "25", String(seed)], { encoding: "utf8", maxBuffer: 256 << 20 }).trim();
    const native = execFileSync(nativeBin, ["diff"], { input: cases, encoding: "utf8", maxBuffer: 256 << 20 }).trimEnd();
    const inWasm = wasm.differentialRecords(cases);
    assert.equal(inWasm, native, `seed ${seed}: identical records`);
    compared += JSON.parse(inWasm).filter((r) => r.error === null).length;
  }
  assert.ok(compared > 30, `${compared} runs compared`);
});
