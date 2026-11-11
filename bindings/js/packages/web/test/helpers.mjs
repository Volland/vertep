// Shared fixtures and helpers of the @vertep/web tests (run with `npm test` after `npm run build`).
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const pkg = fileURLToPath(new URL("..", import.meta.url));
export const workspace = fileURLToPath(new URL("../../..", import.meta.url));
export const nativeBin = join(workspace, ".native", "vertep-wasm-native");

/** Runs the native helper (`trace` with stdin, or `script <file>`). */
export function native(args, input) {
  const out = execFileSync(nativeBin, args, { input, encoding: "utf8", maxBuffer: 64 << 20 });
  return JSON.parse(out);
}

/** Runs a command script natively (see src/bin/vertep-wasm-native.rs). */
export function nativeScript(script) {
  const dir = tempDir();
  const file = join(dir, "script.json");
  writeFileSync(file, JSON.stringify(script));
  return native(["script", file]);
}

export function tempDir() {
  return mkdtempSync(join(tmpdir(), "vertep-js-"));
}

/** Loads a wasm-bindgen build in this thread (for the pure trace function). */
export async function loadBuild(kind) {
  const glue = await import(new URL(`../wasm/${kind}/vertep_wasm.js`, import.meta.url).href);
  await glue.default({ module_or_path: readFileSync(new URL(`../wasm/${kind}/vertep_wasm_bg.wasm`, import.meta.url)) });
  return glue;
}

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Waits until `f()` is truthy, polling. */
export async function until(f, ms = 3000) {
  const end = Date.now() + ms;
  for (;;) {
    const v = await f();
    if (v) return v;
    if (Date.now() > end) throw new Error("timed out waiting");
    await sleep(10);
  }
}

export const ORDER_TYPE = "https://example.org/charts/order";

export const ORDER = `<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="open">
  <state id="open">
    <transition event="pay" target="paid"><send event="ship" delay="300ms"/></transition>
  </state>
  <state id="paid"><transition event="ship" target="shipped"/></state>
  <state id="shipped"/>
</scxml>`;

export const COUNTER = `<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="idle" datamodel="cel">
  <datamodel><data id="n" expr="0"/></datamodel>
  <state id="idle">
    <transition event="inc"><assign location="ctx.n" expr="ctx.n + 1"/></transition>
    <transition event="add"><assign location="ctx.n" expr="ctx.n + ev.data.by"/></transition>
    <transition event="go" target="busy"/>
  </state>
  <state id="busy"><transition event="done" target="idle"/></state>
</scxml>`;

export const PRICER = `<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="s" datamodel="cel">
  <state id="s">
    <transition event="price.get">
      <send event="vt:reply" targetexpr="ev.origin"><content expr="{'price': 42}"/></send>
    </transition>
    <transition event="price.slow">
      <send event="vt:reply" targetexpr="ev.origin" delay="400ms"><content expr="{'price': 7}"/></send>
    </transition>
  </state>
</scxml>`;

export const CONFLICT = `<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="p">
  <parallel id="p">
    <state id="a"><transition event="e" target="x"/></state>
    <state id="b"><transition event="e" target="y"/></state>
  </parallel>
  <state id="x"/><state id="y"/>
</scxml>`;

export const RUST_CHART = `<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="idle" datamodel="rust">
  <datamodel><data id="total" expr="0"/></datamodel>
  <state id="idle">
    <transition event="pay" cond="vt:guard/order/positive" target="paid">
      <foreach array="vt:action/order/record" item="_vt_emit">
        <send eventexpr="_vt_emit.event" targetexpr="_vt_emit.target" typeexpr="_vt_emit.type" delayexpr="_vt_emit.delay"><content expr="_vt_emit.data"/></send>
      </foreach>
    </transition>
  </state>
  <state id="paid"><transition event="receipt" target="done"/></state>
  <final id="done"/>
</scxml>`;
