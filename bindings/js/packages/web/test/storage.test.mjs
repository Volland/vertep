// Storage: explicit kinds, exclusivity, journal files that move between WebAssembly and native.
import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { Worker } from "node:worker_threads";
import { COUNTER, ORDER, ORDER_TYPE, nativeScript, sleep, tempDir, until } from "./helpers.mjs";
import { VertepError, createActorSystem, takeStoreLock } from "../dist/index.js";

const strip = (s) => ({ configuration: s.configuration, context: s.context, position: s.position, version: s.version, terminated: s.terminated });

// @tg: verifies:: [[openspec:wasm-bindings#Node.js Support#OPFS requested in Node.js]] {level: e2e}
test("OPFS requested in Node.js", async () => {
  await assert.rejects(createActorSystem({ storage: "opfs", path: "/vertep/app.db" }), (e) => {
    assert.ok(e instanceof VertepError);
    assert.equal(e.code, "StorageUnavailable");
    return true;
  });
});

// @tg: verifies:: [[openspec:wasm-bindings#Browser Journal Storage#Unavailable OPFS is an error]] {level: e2e}
test("unavailable OPFS is an error, never a memory store", async () => {
  let opened = null;
  try {
    opened = await createActorSystem({ storage: "opfs", path: "/vertep/x.db" });
  } catch (e) {
    assert.equal(e.code, "StorageUnavailable");
    assert.match(e.message, /no memory store/);
  }
  assert.equal(opened, null);
});

// @tg: verifies:: [[openspec:wasm-bindings#Browser Journal Storage#Second writer is refused]] {level: integration}
test("a second holder of a store's Web Lock is refused", async () => {
  const path = `/vertep/locked-${Date.now()}.db`;
  const release = await takeStoreLock(path);
  await assert.rejects(takeStoreLock(path), (e) => e.code === "StoreLocked");
  // another thread (a worker) is refused too while the first holds the lock
  const other = new Worker(
    `import { takeStoreLock } from ${JSON.stringify(new URL("../dist/locks.js", import.meta.url).href)};
     import { parentPort } from "node:worker_threads";
     takeStoreLock(${JSON.stringify(path)}).then(() => parentPort.postMessage("taken"), (e) => parentPort.postMessage(e.code));`,
    { eval: true, type: "module" },
  );
  const answer = await new Promise((r) => other.once("message", r));
  await other.terminate();
  assert.equal(answer, "StoreLocked");
  await release();
  const again = await takeStoreLock(path);
  await again();
});

// @tg: verifies:: [[openspec:wasm-bindings#Browser Journal Storage#Journal file round-trips with native]] {level: e2e}
test("journal file round-trips with native", async () => {
  // WebAssembly -> native
  const system = await createActorSystem({ storage: "memory", journal: true, path: "rt.db" });
  const a = system.virtual({ source: COUNTER, format: "scxml", type: ORDER_TYPE }, "c1");
  for (const e of [{ type: "inc" }, { type: "add", by: 3 }, { type: "go" }]) await a.send(e);
  const browser = strip(await a.getSnapshot());
  const bytes = await system.exportJournal();
  await system.terminate();
  assert.equal(new TextDecoder().decode(bytes.slice(0, 15)), "SQLite format 3");
  const dir = tempDir();
  const file = join(dir, "from-wasm.db");
  writeFileSync(file, bytes);
  const addr = `${ORDER_TYPE}/c1`;
  const nat = nativeScript({
    options: { storage: "native", path: file },
    steps: [
      { op: "register", args: { source: COUNTER, type: ORDER_TYPE } },
      { op: "snapshot", args: { address: addr } },
    ],
  });
  assert.deepEqual(strip(nat.results[1].ok), browser, "native replay of the browser journal");

  // native -> WebAssembly
  const file2 = join(dir, "native.db");
  const written = nativeScript({
    options: { storage: "native", path: file2 },
    steps: [
      { op: "register", args: { source: COUNTER, type: ORDER_TYPE } },
      { op: "send", args: { to: addr, event: { type: "add", by: 10 } } },
      { op: "send", args: { to: addr, event: { type: "inc" } } },
      { op: "snapshot", args: { address: addr } },
    ],
  });
  const native = strip(written.results[3].ok);
  const imported = await createActorSystem({
    storage: "memory",
    journal: true,
    path: "imported.db",
    import: readFileSync(file2),
    charts: [{ source: COUNTER, type: ORDER_TYPE }],
  });
  try {
    assert.deepEqual(strip(await imported.actor(addr).getSnapshot()), native);
  } finally {
    await imported.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Node.js Support#Persist through export]] {level: e2e}
test("persist through export", async () => {
  const first = await createActorSystem({ storage: "memory", journal: true });
  const a = first.virtual({ source: COUNTER, format: "scxml", type: ORDER_TYPE }, "p");
  await a.send({ type: "add", by: 2 });
  await a.send({ type: "go" });
  const before = strip(await a.getSnapshot());
  const file = await first.exportJournal();
  await first.terminate();

  const later = await createActorSystem({ storage: "memory", journal: true, import: file, charts: [{ source: COUNTER, type: ORDER_TYPE }] });
  try {
    const b = later.virtual(ORDER_TYPE, "p");
    assert.deepEqual(strip(await b.getSnapshot()), before);
    await b.send({ type: "done" });
    assert.deepEqual((await b.getSnapshot()).configuration, ["idle"], "it resumes");
  } finally {
    await later.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Browser Runtime Shell#Due timers fire after reopening]] {level: e2e}
test("due timers fire after reopening", async () => {
  const first = await createActorSystem({ storage: "memory", journal: true });
  const a = first.virtual({ source: ORDER, format: "scxml", type: ORDER_TYPE }, "t");
  await a.send({ type: "pay" }); // arms `ship` in 300 ms
  const file = await first.exportJournal();
  await first.terminate(); // closed while the timer is pending
  await sleep(400); // reopened after its due time
  const later = await createActorSystem({ storage: "memory", journal: true, import: file, charts: [{ source: ORDER, type: ORDER_TYPE }] });
  try {
    const b = later.virtual(ORDER_TYPE, "t");
    await until(async () => (await b.getSnapshot()).matches("shipped"), 2000);
    await sleep(100);
    const ships = (await b.journal()).filter((e) => e.event?.name === "ship");
    assert.equal(ships.length, 1, "fired once");
  } finally {
    await later.terminate();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Browser Runtime Shell#Step never reads the clock]] {level: e2e}
test("a browser-recorded journal replays under another clock", async () => {
  const system = await createActorSystem({ storage: "memory", journal: true });
  const a = system.virtual({ source: COUNTER, format: "scxml", type: ORDER_TYPE }, "clock");
  for (const e of [{ type: "inc" }, { type: "add", by: 5 }]) await a.send(e);
  const live = strip(await a.getSnapshot());
  const bytes = await system.exportJournal();
  await system.terminate();
  const file = join(tempDir(), "clock.db");
  writeFileSync(file, bytes);
  // the native helper replays on a virtual clock set decades later
  const nat = nativeScript({
    options: { storage: "native", path: file },
    start: 4_000_000_000_000,
    steps: [
      { op: "register", args: { source: COUNTER, type: ORDER_TYPE } },
      { op: "snapshot", args: { address: `${ORDER_TYPE}/clock` } },
      { op: "asOf", args: { address: `${ORDER_TYPE}/clock`, position: 1 } },
    ],
  });
  assert.deepEqual(strip(nat.results[1].ok), live);
  assert.equal(nat.results[2].ok.context.n, 1);
});

// @tg: verifies:: [[openspec:wasm-bindings#Bundle Size Budget#Storage-free build excludes SQLite]] {level: e2e}
test("storage-free build excludes SQLite", async () => {
  const core = readFileSync(new URL("../wasm/core/vertep_wasm_bg.wasm", import.meta.url));
  const journal = readFileSync(new URL("../wasm/journal/vertep_wasm_bg.wasm", import.meta.url));
  const has = (bytes, text) => bytes.includes(Buffer.from(text));
  assert.equal(has(core, "SQLite format 3"), false, "the core build holds no SQLite");
  assert.equal(has(journal, "SQLite format 3"), true);
  const system = await createActorSystem({ storage: "memory" });
  try {
    assert.equal(system.build, "core", "memory actors without a journal load the core build only");
    const a = await system.spawn(ORDER);
    await a.send({ type: "pay" });
    assert.deepEqual((await a.getSnapshot()).configuration, ["paid"]);
  } finally {
    await system.terminate();
  }
});
