// Node.js runs the same program as a browser and as the native runtime.
import assert from "node:assert/strict";
import { test } from "node:test";
import { COUNTER, ORDER_TYPE, nativeScript } from "./helpers.mjs";
import { createActorSystem } from "../dist/index.js";

const EVENTS = [{ type: "inc" }, { type: "add", by: 4 }, { type: "go" }, { type: "done" }, { type: "inc" }];
const entry = (e) => ({ position: e.position, event: e.event.name, messageId: e.messageId, version: e.version, digest: e.digest, transitions: e.transitions });
const snap = (s) => ({ configuration: [...s.configuration], context: s.context, position: s.position, version: s.version, terminated: s.terminated });

// @tg: verifies:: [[openspec:wasm-bindings#Node.js Support#Same program runs in Node.js]] {level: e2e}
test("the same program runs in Node.js", async () => {
  const addr = `${ORDER_TYPE}/same`;
  const system = await createActorSystem({ storage: "memory" });
  let wasm;
  try {
    const a = system.virtual({ source: COUNTER, format: "scxml", type: ORDER_TYPE }, "same");
    const snapshots = [];
    a.subscribe((s) => snapshots.push(snap(s)));
    await new Promise((r) => setTimeout(r, 20));
    for (const [i, e] of EVENTS.entries()) await a.send(e, { messageId: `urn:m:${i}` });
    await new Promise((r) => setTimeout(r, 30));
    wasm = { snapshot: snap(await a.getSnapshot()), journal: (await a.journal()).map(entry), snapshots };
  } finally {
    await system.terminate();
  }
  const steps = [
    { op: "register", args: { source: COUNTER, type: ORDER_TYPE } },
    { op: "subscribe", args: { address: addr } },
    ...EVENTS.map((e, i) => ({ op: "send", args: { to: addr, event: e, messageId: `urn:m:${i}` } })),
    { op: "snapshot", args: { address: addr } },
    { op: "journal", args: { address: addr } },
  ];
  const nat = nativeScript({ options: { storage: "memory" }, steps });
  const results = nat.results.map((r) => r.ok);
  assert.deepEqual(wasm.snapshot, snap(results.at(-2)));
  assert.deepEqual(wasm.journal, results.at(-1).map(entry), "the same journal entries");
  assert.deepEqual(wasm.snapshots, nat.notifications.map((n) => snap(n.snapshot)), "the same snapshots");
  assert.equal(wasm.snapshots.length, 5);
});
