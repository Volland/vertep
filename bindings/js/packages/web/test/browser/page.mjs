// Browser-side halves of browser.test.mjs, called through the DevTools protocol.
import { createActorSystem } from "../../dist/index.js";

const COUNTER = `<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="idle" datamodel="cel">
  <datamodel><data id="n" expr="0"/></datamodel>
  <state id="idle">
    <transition event="inc"><assign location="ctx.n" expr="ctx.n + 1"/></transition>
    <transition event="go" target="busy"/>
  </state>
  <state id="busy"><transition event="done" target="idle"/></state>
</scxml>`;
const TYPE = "https://example.org/charts/counter";
const chart = { source: COUNTER, format: "scxml", type: TYPE };
const plain = (s) => ({ configuration: [...s.configuration], context: s.context, position: s.position, version: s.version });
const code = (e) => ({ code: e.code ?? null, message: String(e.message) });

// the test build (every datamodel plus the test-only exports), not part of the packages
async function testBuild() {
  const glue = await import("../../../../.native/wasm-test/vertep_wasm.js");
  await glue.default({ module_or_path: new URL("../../../../.native/wasm-test/vertep_wasm_bg.wasm", import.meta.url) });
  return glue;
}

// the WebSocket interop scenarios (the server is `vertep-wasm-native ws-server`)
const SERVER_ACTOR = "https://server.example/charts/server/s1";
const CLIENT_TYPE = "https://client.example/charts/client";
const CLIENT = `<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="c" datamodel="cel">
  <datamodel><data id="n" expr="0"/><data id="ot" expr="''"/></datamodel>
  <state id="c">
    <transition event="go"><send type="websocket" event="ping" target="${SERVER_ACTOR}"/></transition>
    <transition event="pong"><assign location="ctx.n" expr="ctx.n + 1"/><assign location="ctx.ot" expr="ev.origintype"/></transition>
  </state>
</scxml>`;
const wsSystem = (url, credential) =>
  createActorSystem({
    storage: "memory",
    charts: [{ source: CLIENT, format: "scxml", type: CLIENT_TYPE }],
    transports: { websocket: { url, credentials: async () => credential, targets: ["https://server.example/"], reconnectMs: 50 } },
  });
let ws = null;

window.vertepTests = {
  async wsStart(url) {
    ws = await wsSystem(url, "secret");
    return ws.virtual(CLIENT_TYPE, "c1").address;
  },
  async wsGo() {
    await ws.virtual(CLIENT_TYPE, "c1").send({ type: "go" });
    return true;
  },
  async wsClient() {
    const a = ws.virtual(CLIENT_TYPE, "c1");
    const journal = await a.journal();
    return { context: (await a.getSnapshot()).context, pongs: journal.filter((e) => e.event?.name === "pong").length };
  },
  async wsHit() {
    await ws.actor(SERVER_ACTOR).send({ type: "hit" });
    return true;
  },
  async wsStop() {
    await ws?.terminate();
    ws = null;
    return true;
  },
  async wsRefused(url) {
    const s = await wsSystem(url, "wrong");
    try {
      await s.actor(SERVER_ACTOR).send({ type: "hit" });
      return { code: null };
    } catch (e) {
      return code(e);
    } finally {
      await s.terminate();
    }
  },
  async write(path) {
    const system = await createActorSystem({ storage: "opfs", path, charts: [chart] });
    try {
      const a = system.virtual(TYPE, "k");
      for (const type of ["inc", "inc", "go"]) await a.send({ type });
      return { build: system.build, snapshot: plain(await a.getSnapshot()) };
    } finally {
      await system.terminate();
    }
  },
  async read(path) {
    const system = await createActorSystem({ storage: "opfs", path, charts: [chart] });
    try {
      return { snapshot: plain(await system.virtual(TYPE, "k").getSnapshot()) };
    } finally {
      await system.terminate();
    }
  },
  async secondWriter(path) {
    const first = await createActorSystem({ storage: "opfs", path, charts: [chart] });
    let second = null;
    try {
      await first.virtual(TYPE, "w").send({ type: "inc" });
      try {
        const s = await createActorSystem({ storage: "opfs", path, charts: [chart] });
        await s.terminate();
        second = { opened: true };
      } catch (e) {
        second = code(e);
      }
      await first.virtual(TYPE, "w").send({ type: "inc" });
      return { second, first: plain(await first.virtual(TYPE, "w").getSnapshot()) };
    } finally {
      await first.terminate();
    }
  },
  async diff(cases) {
    return (await testBuild()).differentialRecords(cases);
  },
  async conformance(input) {
    return (await testBuild()).conformanceRun(input);
  },
  async mainThreadOpfs() {
    // OPFS sync access handles exist only in dedicated workers: the page itself cannot use
    // them, which the worker reports when asked for an OPFS store with a page-only build
    try {
      return { ok: typeof window.FileSystemSyncAccessHandle };
    } catch (e) {
      return code(e);
    }
  },
};
window.vertepReady = true;
