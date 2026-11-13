// The TypeScript API over a worker thread in Node.js: the same packages a browser loads.
import assert from "node:assert/strict";
import { test } from "node:test";
import { COUNTER, CONFLICT, ORDER, ORDER_TYPE, PRICER, RUST_CHART, sleep, until } from "./helpers.mjs";
import { VertepError, createActorSystem } from "../dist/index.js";

const impl = (name) => new URL(`./fixtures/${name}.mjs`, import.meta.url);

async function rejects(p, code) {
  await assert.rejects(p, (e) => {
    assert.ok(e instanceof VertepError, `expected a VertepError, got ${e}`);
    assert.equal(e.code, code, e.message);
    return true;
  });
}

// @tg: verifies:: [[openspec:wasm-bindings#Worker-Hosted Actor System#Actor system starts in a worker]] {level: integration}
test("actor system starts in a worker", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    const order = await system.spawn(ORDER);
    const pending = order.getSnapshot();
    assert.ok(pending instanceof Promise, "every call returns a Promise");
    const s = await pending;
    assert.deepEqual(s.configuration, ["open"]);
    const sending = order.send({ type: "pay" });
    assert.ok(sending instanceof Promise);
    await sending;
    assert.equal(typeof order.subscribe(() => {}), "function", "subscriptions return an unsubscribe");
    assert.equal(system.build, "core");
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Worker-Hosted Actor System#Implementations load inside the worker]] {level: integration}
test("implementations load inside the worker", async () => {
  const system = await createActorSystem({ storage: "memory", implementations: impl("impl") });
  try {
    const a = await system.spawn(RUST_CHART);
    await a.send({ type: "pay", amount: 0 });
    assert.deepEqual((await a.getSnapshot()).configuration, ["idle"], "guard false");
    await a.send({ type: "pay", amount: 12 });
    const s = await a.getSnapshot();
    // the action set the context and raised `receipt`, processed in the same macrostep
    assert.equal(s.terminated, true);
    assert.equal(s.context.total, 12);
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Worker-Hosted Actor System#Worker failure is reported]] {level: integration}
test("worker failure is reported", async () => {
  const system = await createActorSystem({ storage: "memory", implementations: impl("crash") });
  const a = await system.spawn(RUST_CHART);
  const snapshot = a.getSnapshot();
  const send = a.send({ type: "pay", amount: 1 });
  await snapshot;
  await rejects(send, "WorkerTerminated");
  await rejects(a.getSnapshot(), "WorkerTerminated");
  await rejects(system.spawn(ORDER), "WorkerTerminated");
});

test("development mode reports impure guards", async () => {
  const reports = [];
  const system = await createActorSystem({
    storage: "memory",
    implementations: impl("impure"),
    dev: true,
    onImpurity: (r) => reports.push(r),
  });
  try {
    const a = await system.spawn(RUST_CHART);
    await a.send({ type: "pay", amount: 1 });
    await until(() => reports.length > 0);
    assert.match(reports[0].message, /impure/);
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Actor Creation From Chart Sources#Create from SCXML text]] {level: e2e}
test("create from SCXML text", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    const a = await system.spawn(ORDER, { key: "1" });
    const s = await a.getSnapshot();
    assert.deepEqual(s.configuration, ["open"]);
    assert.match(s.version, /^urn:vertep:chart:sha256:[0-9a-f]{64}$/);
    assert.equal(s.matches("open"), true);
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:scxml-format#Layout Namespace#Layout does not change the chart version]] {level: e2e}
test("SCXML layout does not change the chart version", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    const laidOut = ORDER.replace(
      'version="1.0"',
      'xmlns:vl="https://w3id.org/vertep/layout#" version="1.0"',
    )
      .replace('<state id="open">', '<state id="open" vl:x="40" vl:y="40" vl:w="120" vl:h="60" vl:collapsed="false">')
      .replace('target="paid">', 'target="paid" vl:route="160,70 220,70" vl:label="190,60">');
    assert.notEqual(laidOut, ORDER);
    const plain = await (await system.spawn(ORDER, { key: "plain" })).getSnapshot();
    const layout = await (await system.spawn(laidOut, { key: "layout" })).getSnapshot();
    assert.deepEqual(layout.configuration, ["open"]);
    assert.equal(layout.version, plain.version);
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:yaml-format#Layout Block#Layout does not change the chart version]] {level: e2e}
test("YAML layout does not change the chart version", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    const yaml = "vertep: 1\nname: door\nstates:\n  idle:\n    on: { go: done }\n  done: { type: final }\n";
    const laidOut = `${yaml}layout:
  states:
    idle: { x: 40, y: 40, w: 120, h: 60, collapsed: false }
  transitions:
    "idle#0": { route: [[160, 70], [240, 70]], label: [200, 60] }
`;
    const plain = await (await system.spawn(yaml, { key: "plain" })).getSnapshot();
    const layout = await (await system.spawn(laidOut, { key: "layout" })).getSnapshot();
    assert.deepEqual(layout.configuration, ["idle"]);
    assert.equal(layout.version, plain.version);
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Actor Creation From Chart Sources#Invalid chart is rejected]] {level: e2e}
test("invalid chart is rejected", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    const bad = `<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
  <state id="a"><transition event="x" target="missing"/></state>
</scxml>`;
    await assert.rejects(system.spawn(bad), (e) => {
      assert.equal(e.code, "ChartInvalid");
      assert.ok(e.violations.length >= 1);
      assert.ok(e.violations[0].line > 0, JSON.stringify(e.violations));
      return true;
    });
    assert.deepEqual(await system.actors(), []);
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Actor Creation From Chart Sources#Virtual actor addressed by type and key]] {level: e2e}
test("virtual actor addressed by type and key", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    await system.register(ORDER, { type: ORDER_TYPE });
    const a = system.virtual(ORDER_TYPE, "k1");
    assert.deepEqual(await system.actors(), []);
    await a.send({ type: "pay" });
    const b = system.virtual(ORDER_TYPE, "k1");
    assert.equal(await b.address, await a.address);
    assert.deepEqual((await b.getSnapshot()).configuration, ["paid"]);
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Event Sending From JavaScript#Duplicate send is deduplicated]] {level: e2e}
test("duplicate send is deduplicated", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    await system.register(COUNTER, { type: ORDER_TYPE });
    const a = system.virtual(ORDER_TYPE, "d");
    const r1 = await a.send({ type: "inc" }, { messageId: "urn:msg:7" });
    const r2 = await a.send({ type: "inc" }, { messageId: "urn:msg:7" });
    assert.equal(r1.messageId, r2.messageId);
    assert.equal((await a.journal()).length, 1);
    assert.equal((await a.getSnapshot()).context.n, 1);
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Event Sending From JavaScript#Request resolves with the reply]] {level: e2e}
test("request resolves with the reply", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    const p = await system.spawn(PRICER);
    const reply = await p.request({ type: "price.get" }, { timeoutMs: 2000 });
    assert.equal(reply.name, "vt:reply");
    assert.deepEqual(reply.data, { price: 42 });
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Event Sending From JavaScript#Request times out]] {level: e2e}
test("request times out", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    const p = await system.spawn(PRICER);
    await rejects(p.request({ type: "price.slow" }, { timeoutMs: 100 }), "RequestTimeout");
    // the late reply arrives at 400 ms and is not delivered to anyone waiting
    await sleep(450);
    const j = await p.journal();
    assert.equal(j.length, 1);
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Snapshot And Transition Subscriptions#Snapshot after each macrostep]] {level: e2e}
test("snapshot after each macrostep", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    const a = await system.spawn(COUNTER);
    const seen = [];
    a.subscribe((s) => seen.push(s));
    await sleep(20);
    await a.send({ type: "inc" });
    await a.send({ type: "add", by: 4 });
    await a.send({ type: "done" }); // changes nothing: no snapshot
    await until(() => seen.length >= 2);
    await sleep(30);
    assert.deepEqual(
      seen.map((s) => [s.position, s.context.n]),
      [
        [1, 1],
        [2, 5],
      ],
    );
    assert.match(seen[0].version, /^urn:vertep:chart:/);
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Snapshot And Transition Subscriptions#Transition notification lists taken transitions]] {level: e2e}
test("transition notification lists taken transitions", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    const a = await system.spawn(ORDER);
    const seen = [];
    a.onTransition((t) => seen.push(t));
    await sleep(20);
    await a.send({ type: "pay" });
    await until(() => seen.length >= 2, 2000); // pay, then the timer's ship
    assert.equal(seen[0].event.name, "pay");
    assert.deepEqual(seen[0].transitions.map((t) => [t.source, t.targets]), [["open", ["paid"]]]);
    assert.equal(seen[1].event.name, "ship");
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Snapshot And Transition Subscriptions#Unsubscribe stops notifications]] {level: e2e}
test("unsubscribe stops notifications", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    const a = await system.spawn(COUNTER);
    const seen = [];
    const stop = a.subscribe((s) => seen.push(s));
    await sleep(20);
    await a.send({ type: "inc" });
    await until(() => seen.length === 1);
    stop();
    await sleep(20);
    await a.send({ type: "inc" });
    await sleep(30);
    assert.equal(seen.length, 1);
    assert.equal((await a.getSnapshot()).context.n, 2);
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Journal Inspection And Time Travel#Read journal entries]] {level: e2e}
test("read journal entries", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    const a = await system.spawn(COUNTER);
    await a.send({ type: "inc" }, { messageId: "urn:m:1" });
    await a.send({ type: "go" }, { messageId: "urn:m:2" });
    const j = await a.journal({ from: 1 });
    assert.deepEqual(
      j.map((e) => [e.position, e.event.name, e.messageId]),
      [
        [1, "inc", "urn:m:1"],
        [2, "go", "urn:m:2"],
      ],
    );
    for (const e of j) {
      assert.equal(typeof e.timestamp, "number");
      assert.match(e.version, /^urn:vertep:chart:/);
    }
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Journal Inspection And Time Travel#Snapshot as of a past position]] {level: e2e}
test("snapshot as of a past position", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    const a = await system.spawn(COUNTER);
    for (let i = 0; i < 3; i++) await a.send({ type: "inc" });
    const then = await a.asOf({ position: 1 });
    assert.equal(then.context.n, 1);
    assert.equal((await a.getSnapshot()).context.n, 3);
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Journal Inspection And Time Travel#Graph query over history]] {level: e2e}
test("graph query over history", async () => {
  const system = await createActorSystem({ storage: "memory", journal: true });
  try {
    assert.equal(system.build, "journal");
    const a = await system.spawn(ORDER);
    const addr = await a.address;
    const before = Date.now();
    await sleep(15);
    await a.send({ type: "pay" });
    await sleep(15);
    const between = Date.now();
    await until(async () => (await a.getSnapshot()).matches("shipped"));
    const q = `SELECT ?s WHERE { GRAPH <${addr}> { <${addr}> <https://w3id.org/vertep/vt#inState> ?s } }`;
    const states = async (time) => (await system.query(q, { time })).rows.map((r) => r.s.replace(/^.*#s\.|>$/g, ""));
    assert.deepEqual(await states(undefined), ["shipped"]);
    assert.deepEqual(await states(between), ["paid"]);
    assert.deepEqual(await states(before), []);
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Browser Runtime Shell#Timer fires through the event loop]] {level: e2e}
test("timer fires through the event loop", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    const a = await system.spawn(ORDER);
    const sent = Date.now();
    await a.send({ type: "pay" });
    const [, ship] = await until(async () => {
      const j = await a.journal();
      return j.length === 2 && j;
    });
    assert.equal(ship.event.name, "ship");
    assert.ok(Date.now() - sent >= 300, "no earlier than the delay");
    const pay = (await a.journal())[0];
    assert.ok(ship.timestamp >= pay.timestamp + 300, "the journaled due time");
    assert.ok(ship.timer, "delivered as a timer input");
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#SCXML Semantics In JavaScript#Document order decides conflicts]] {level: e2e}
test("document order decides conflicts", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    const a = await system.spawn(CONFLICT);
    await a.send({ type: "e" });
    assert.deepEqual((await a.getSnapshot()).configuration, ["x"]);
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#SCXML Semantics In JavaScript#Can reflects enabled transitions]] {level: e2e}
test("can reflects enabled transitions", async () => {
  const system = await createActorSystem({ storage: "memory" });
  try {
    const a = await system.spawn(COUNTER);
    await a.send({ type: "inc" });
    assert.equal(await a.can({ type: "go" }), true);
    assert.equal(await a.can({ type: "done" }), false);
    const before = (await a.journal()).length;
    await a.can({ type: "go" });
    assert.equal((await a.journal()).length, before, "nothing journaled");
    assert.deepEqual((await a.getSnapshot()).configuration, ["idle"], "nothing changed");
  } finally {
    await system.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#SCXML Semantics In JavaScript#Matches checks active states]] {level: e2e}
test("matches checks active states", async () => {
  const nested = `<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="checkout">
  <state id="checkout" initial="payment"><state id="payment"/></state>
  <state id="done"/>
</scxml>`;
  const system = await createActorSystem({ storage: "memory" });
  try {
    const a = await system.spawn(nested);
    const s = await a.getSnapshot();
    assert.equal(s.matches("checkout"), true);
    assert.equal(s.matches("payment"), true);
    assert.equal(s.matches("done"), false);
  } finally {
    await system.terminate();
  }
});

test("storage must be chosen explicitly", async () => {
  await rejects(createActorSystem({}), "InvalidArgument");
});
