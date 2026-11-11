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

window.vertepTests = {
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
    const glue = await import("../../../dm-ecma/wasm/vertep_wasm.js");
    await glue.default({ module_or_path: new URL("../../../dm-ecma/wasm/vertep_wasm_bg.wasm", import.meta.url) });
    return glue.differentialRecords(cases);
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
