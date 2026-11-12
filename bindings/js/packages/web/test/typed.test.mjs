// Typed CEL charts in the worker: instantiated with their projected CEL type environment, as
// natively, refused with `ChartInvalid` when they have CEL type errors, and untyped CEL charts
// stay dynamic.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createActorSystem } from "../dist/index.js";

const ORDER = readFileSync(new URL("../../../../../examples/typed-order/order.yaml", import.meta.url), "utf8");

const item = (qty) => ({ type: "add", sku: "BOOK-1", qty, price: "12.50" });
const card = { tag: "Card", value: { last4: "4242" } };

// @tg: verifies:: [[openspec:cel-datamodel#Static Type Checking of CEL]] {level: e2e}
test("typed CEL chart runs typed in the worker", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    const order = await system.spawn(ORDER, { key: "o1" });
    await order.send(item(2));
    let s = await order.getSnapshot();
    // a decimal, not an int plus a string: 2 x 12.50
    assert.equal(Number(s.context.total), 25);
    await order.send({ type: "checkout" });
    // `Card` without its `value` fails the declared payload type and is not delivered
    await order.send({ type: "pay", amount: "25", method: { tag: "Card" } });
    assert.deepEqual((await order.getSnapshot()).configuration, ["shopping", "payment"]);
    await order.send({ type: "pay", amount: "20", method: card });
    assert.deepEqual((await order.getSnapshot()).configuration, ["shopping", "payment"]);
    await order.send({ type: "pay", amount: "25", method: card });
    s = await order.getSnapshot();
    assert.deepEqual(s.configuration, ["fulfilment"]);
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:cel-datamodel#Static Type Checking of CEL#Assignment type mismatch]] {level: e2e}
test("CEL-ill-typed chart is rejected with ChartInvalid", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    const bad = `<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" datamodel="cel" initial="a">
  <datamodel><data id="count" vt:type="int" expr="0"/></datamodel>
  <state id="a"><onentry><assign location="ctx.count" expr="'many'"/></onentry></state>
</scxml>`;
    await assert.rejects(system.spawn(bad), (e) => {
      assert.equal(e.code, "ChartInvalid", e.message);
      const all = e.violations.map((v) => v.message).join("\n");
      assert.match(all, /ctx\.count/);
      assert.match(all, /int/);
      assert.match(all, /string/);
      return true;
    });
    assert.deepEqual(await system.actors(), []);
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:cel-datamodel#Static Type Checking of CEL#Untyped chart checks dynamically]] {level: e2e}
test("untyped CEL chart runs dynamically in the worker", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    const tally = `<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="cel" initial="s">
  <datamodel><data id="n" expr="0"/></datamodel>
  <state id="s">
    <transition event="add"><assign location="n" expr="ctx.n + ev.data.k"/></transition>
    <transition event="error.execution" target="failed"/>
  </state>
  <state id="failed"/>
</scxml>`;
    const a = await system.spawn(tally);
    await a.send({ type: "add", k: 2 });
    await a.send({ type: "add", k: 3 });
    assert.equal((await a.getSnapshot()).context.n, 5);
    await a.send({ type: "add", k: "x" });
    assert.deepEqual((await a.getSnapshot()).configuration, ["failed"]);
  } finally {
    await system.terminate();
  }
});
