// Shared fixtures and helpers of the @vertep/web tests (run with `npm test` after `npm run build`).
import { execFileSync, spawn } from "node:child_process";
import { createInterface } from "node:readline";
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

/**
 * Loads the test build (`.native/wasm-test`: every datamodel plus the test-only exports
 * `traceCases`, `differentialRecords` and `conformanceRun`, which the published builds leave
 * out) in this thread.
 */
export async function loadTestBuild() {
  const dir = new URL("../../../.native/wasm-test/", import.meta.url);
  const glue = await import(new URL("vertep_wasm.js", dir).href);
  await glue.default({ module_or_path: readFileSync(new URL("vertep_wasm_bg.wasm", dir)) });
  return glue;
}

/**
 * The W3C IRP gate on a run's verdicts: an allow-listed variant that passes, or a mandatory
 * variant that fails without an allow-list entry, fails the run (as the native gate does).
 */
export function gateFailures(input, verdicts) {
  const allowed = new Set(input.allow.map(([t, d]) => `${t}/${d}`));
  const classes = new Map(input.cases.map((c) => [`${c.test}/${c.datamodel}`, c.classification]));
  return verdicts
    .filter((v) => {
      const k = `${v.test}/${v.datamodel}`;
      return v.verdict === "pass" ? allowed.has(k) : !allowed.has(k) && classes.get(k) === "mandatory";
    })
    .map((v) => `${v.test}/${v.datamodel}: ${v.verdict === "pass" ? "unexpected pass" : `unexpected failure (${v.reason})`}`);
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

/**
 * Starts the native WebSocket server of the end-to-end tests (`vertep-wasm-native ws-server`):
 * a native actor system hosting `SERVER_TYPE` that accepts the credential `secret` only.
 * `cmd({op, args})` runs a bridge command on it, `ctl(name)` a fault control (`hold`,
 * `release`, `dup`, `nodup`, `drop`, `pending`); both resolve with the reply's `ok` value.
 */
export async function startWsServer() {
  const proc = spawn(nativeBin, ["ws-server"], { stdio: ["pipe", "pipe", "inherit"] });
  const lines = createInterface({ input: proc.stdout });
  const waiting = [];
  lines.on("line", (l) => waiting.shift()?.(JSON.parse(l)));
  const next = () => new Promise((r) => waiting.push(r));
  const { port } = await next();
  const send = async (o) => {
    const reply = next();
    proc.stdin.write(`${JSON.stringify(o)}\n`);
    const r = await reply;
    if ("err" in r) throw new Error(JSON.stringify(r.err));
    return r.ok;
  };
  return {
    port,
    url: `ws://127.0.0.1:${port}/vertep`,
    cmd: (op, args = {}) => send({ op, args }),
    ctl: (name) => send({ ctl: name }),
    close: () => proc.kill(),
  };
}

export const SERVER_TYPE = "https://server.example/charts/server";
export const SERVER_ACTOR = `${SERVER_TYPE}/s1`;
export const WS_PROCESSOR = "https://w3id.org/vertep/vt#WebSocketEventProcessor";
export const CLIENT_TYPE = "https://client.example/charts/client";

/** A browser-side chart: `go` sends `ping` to the server actor, `pong` replies are counted. */
export const CLIENT = `<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="c" datamodel="cel">
  <datamodel><data id="n" expr="0"/><data id="ot" expr="''"/></datamodel>
  <state id="c">
    <transition event="go"><send type="websocket" event="ping" target="${SERVER_ACTOR}"/></transition>
    <transition event="pong"><assign location="ctx.n" expr="ctx.n + 1"/><assign location="ctx.ot" expr="ev.origintype"/></transition>
  </state>
</scxml>`;
