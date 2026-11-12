// The core in WebAssembly against the same core natively: identical traces.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { COUNTER, CONFLICT, ORDER, loadBuild, loadTestBuild, nativeBin } from "./helpers.mjs";

const HISTORY = `<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="on" datamodel="cel">
  <datamodel><data id="log" expr="[]"/></datamodel>
  <state id="on" initial="a">
    <history id="h" type="deep"/>
    <transition event="off" target="off"/>
    <state id="a"><onentry><assign location="ctx.log" expr="ctx.log + ['a']"/></onentry><transition event="next" target="b"/></state>
    <state id="b"><onentry><raise event="entered.b"/></onentry><transition event="entered.b" target="c"/></state>
    <state id="c"><transition cond="size(ctx.log) &gt; 0" target="d"/></state>
    <state id="d"/>
  </state>
  <state id="off"><transition event="on" target="h"/></state>
</scxml>`;

const YAML = readFileSync(new URL("./fixtures/order.yaml", import.meta.url), "utf8");

const CASES = [
  { name: "order", chart: ORDER, start_ms: 1000, inputs: [{ name: "pay", now_ms: 1100, seed: 7 }, { name: "ship", now_ms: 1400, seed: 8 }] },
  {
    name: "counter",
    chart: COUNTER,
    inputs: [
      { name: "inc", now_ms: 1, seed: 1 },
      { name: "add", data: { by: 5 }, now_ms: 2, seed: 2 },
      { name: "add", data: { by: 2.5 }, now_ms: 3, seed: 3 },
      { name: "go", now_ms: 4, seed: 4 },
      { name: "unknown", now_ms: 5, seed: 5 },
    ],
  },
  { name: "conflict", chart: CONFLICT, inputs: [{ name: "e" }] },
  { name: "history", chart: HISTORY, inputs: [{ name: "next" }, { name: "off" }, { name: "on" }] },
  { name: "yaml", chart: YAML, format: "yaml", inputs: [{ name: "start" }, { name: "pay", data: { amount: 12.5 } }, { name: "authorized" }] },
];

// @tg: verifies:: [[openspec:wasm-bindings#Core Builds For WebAssembly Unchanged#Native and WebAssembly steps agree]] {level: integration}
test("native and WebAssembly steps agree", async () => {
  // the test build: the same core and datamodels as the published builds, plus `traceCases`
  const build = await loadTestBuild();
  const input = JSON.stringify(CASES);
  const wasm = build.traceCases(input);
  const nat = execFileSync(nativeBin, ["trace"], { input, encoding: "utf8" }).trimEnd();
  const traces = JSON.parse(wasm);
  for (const t of traces) {
    // every case ran: a chart that fails to load would compare equal but prove nothing
    assert.equal(t.error, null, `${t.name}: ${t.error}`);
  }
  assert.equal(traces.find((t) => t.name === "counter").steps.length, 6);
  assert.equal(traces.find((t) => t.name === "yaml").steps.at(-1).terminated, true, "paid is a top-level final state");
  assert.equal(wasm, nat, "byte-for-byte equal traces");
});

// @tg: verifies:: [[openspec:wasm-bindings#Core Builds For WebAssembly Unchanged#Core compiles for wasm32 without changes]] {level: integration}
test("the core build for wasm32 loads and runs", async () => {
  const core = await loadBuild("core");
  assert.equal(core.version(), "0.1.0");
  assert.equal(core.hasJournal(), false);
  assert.equal(typeof core.traceCases, "undefined", "test-only exports stay out of the published build");
  const system = await core.ActorSystem.create(JSON.stringify({ storage: "memory" }), null);
  try {
    const run = (op, args) => JSON.parse(system.command(op, JSON.stringify(args)));
    const { address } = run("spawn", { source: ORDER, key: "o1" });
    run("send", { to: address, event: { type: "pay" } });
    assert.deepEqual(run("snapshot", { address }).configuration, ["paid"]);
  } finally {
    system.close();
  }
});
