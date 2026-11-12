// Conformance in WebAssembly (the test build, with every datamodel):
// - the W3C SCXML IRP suite, its documents passed in as text, gives the native harness's
//   verdicts on every non-manual variant and passes the gate with the shared allow-list;
// - the differential test's generated charts and inputs give identical oracle records, which
//   are the records the native test hands the Lean model.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { test } from "node:test";
import { gateFailures, loadTestBuild, nativeBin } from "./helpers.mjs";

const run = (args) => execFileSync(nativeBin, args, { encoding: "utf8", maxBuffer: 256 << 20 });

// @tg: verifies:: [[openspec:wasm-bindings#Conformance Suite In WebAssembly#Suite passes in headless Chrome]] {level: integration}
test("the W3C IRP suite in WebAssembly gives the native verdicts and passes the gate", async () => {
  const wasm = await loadTestBuild();
  const input = run(["conformance-input"]);
  const native = JSON.parse(run(["conformance-native"]));
  const verdicts = JSON.parse(wasm.conformanceRun(input));
  assert.equal(verdicts.length, native.length);
  assert.deepEqual(verdicts, native, "every verdict and failure reason equals the native run's");
  assert.deepEqual(gateFailures(JSON.parse(input), verdicts), [], "no unexpected pass or failure");
  assert.ok(verdicts.filter((v) => v.verdict === "pass").length > 300);
});

// @tg: verifies:: [[openspec:wasm-bindings#Conformance Suite In WebAssembly#Traces equal native traces]] {level: integration}
test("differential traces in WebAssembly equal native traces", async () => {
  const wasm = await loadTestBuild();
  let compared = 0;
  for (const seed of [1, 2]) {
    const cases = run(["gen-diff", "25", String(seed)]).trim();
    const native = execFileSync(nativeBin, ["diff"], { input: cases, encoding: "utf8", maxBuffer: 256 << 20 }).trimEnd();
    const inWasm = wasm.differentialRecords(cases);
    assert.equal(inWasm, native, `seed ${seed}: identical records`);
    compared += JSON.parse(inWasm).filter((r) => r.error === null).length;
  }
  assert.ok(compared > 30, `${compared} runs compared`);
});
