// The browser-only scenarios (OPFS in a dedicated Web Worker), in headless Chrome driven over
// the DevTools protocol. Skipped when Chrome is not installed (set CHROME to its binary).
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, normalize } from "node:path";
import { test } from "node:test";
import { nativeBin, sleep, tempDir, workspace } from "./helpers.mjs";

const CHROME = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".wasm": "application/wasm", ".json": "application/json" };

function serve() {
  const server = createServer((req, res) => {
    const path = normalize(decodeURIComponent(new URL(req.url, "http://x").pathname)).replace(/^(\.\.[/\\])+/, "");
    const file = join(workspace, path);
    if (!file.startsWith(workspace) || !existsSync(file)) {
      res.writeHead(404).end();
      return;
    }
    res.writeHead(200, { "content-type": TYPES[extname(file)] ?? "application/octet-stream" });
    res.end(readFileSync(file));
  });
  return new Promise((r) => server.listen(0, "127.0.0.1", () => r(server)));
}

async function chrome() {
  const profile = tempDir();
  const proc = spawn(CHROME, ["--headless=new", "--remote-debugging-port=0", `--user-data-dir=${profile}`, "--no-first-run", "--no-default-browser-check", "about:blank"], { stdio: "ignore" });
  const portFile = join(profile, "DevToolsActivePort");
  for (let i = 0; i < 200 && !existsSync(portFile); i++) await sleep(50);
  const port = readFileSync(portFile, "utf8").split("\n")[0];
  const pages = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const ws = new WebSocket(pages.find((p) => p.type === "page").webSocketDebuggerUrl);
  await new Promise((r, j) => ((ws.onopen = r), (ws.onerror = j)));
  let next = 1;
  const pending = new Map();
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id && pending.has(msg.id)) {
      pending.get(msg.id)(msg);
      pending.delete(msg.id);
    }
  };
  const send = (method, params = {}) =>
    new Promise((r) => {
      const id = next++;
      pending.set(id, r);
      ws.send(JSON.stringify({ id, method, params }));
    });
  const evaluate = async (expression) => {
    const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (r.result?.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails));
    return r.result?.result?.value;
  };
  const open = async (url) => {
    await send("Page.navigate", { url });
    for (let i = 0; i < 200; i++) {
      if (await evaluate("window.vertepReady === true").catch(() => false)) return;
      await sleep(50);
    }
    throw new Error(`${url} did not load`);
  };
  return { evaluate, open, close: () => (ws.close(), proc.kill()) };
}

test("browser: OPFS storage in a dedicated worker", { skip: !existsSync(CHROME) && "Chrome is not installed" }, async (t) => {
  const server = await serve();
  const url = `http://127.0.0.1:${server.address().port}/packages/web/test/browser/index.html`;
  const b = await chrome();
  try {
    await b.open(url);
    const path = `/vertep/app-${Date.now()}.db`;

    // @tg: verifies:: [[openspec:wasm-bindings#Browser Journal Storage#Durable journal survives reload]] {level: e2e}
    await t.test("durable journal survives reload", async () => {
      const before = await b.evaluate(`vertepTests.write(${JSON.stringify(path)})`);
      assert.equal(before.build, "journal");
      assert.deepEqual(before.snapshot.configuration, ["busy"]);
      await b.open(url); // reload the page
      const after = await b.evaluate(`vertepTests.read(${JSON.stringify(path)})`);
      assert.deepEqual(after.snapshot, before.snapshot);
    });

    // @tg: verifies:: [[openspec:wasm-bindings#Browser Journal Storage#Second writer is refused]] {level: e2e}
    await t.test("second writer is refused", async () => {
      const r = await b.evaluate(`vertepTests.secondWriter(${JSON.stringify(path + ".w")})`);
      assert.equal(r.second.code, "StoreLocked", JSON.stringify(r));
      assert.equal(r.first.context.n, 2, "the first worker's journal is unaffected");
    });

    // @tg: verifies:: [[openspec:wasm-bindings#Conformance Suite In WebAssembly#Traces equal native traces]] {level: e2e}
    await t.test("differential traces in headless Chrome equal native traces", async () => {
      const cases = execFileSync(nativeBin, ["gen-diff", "20", "11"], { encoding: "utf8", maxBuffer: 256 << 20 }).trim();
      const native = execFileSync(nativeBin, ["diff"], { input: cases, encoding: "utf8", maxBuffer: 256 << 20 }).trimEnd();
      const inChrome = await b.evaluate(`vertepTests.diff(${JSON.stringify(cases)})`);
      assert.equal(inChrome, native);
      assert.ok(JSON.parse(inChrome).filter((r) => r.error === null).length > 10);
    });
  } finally {
    b.close();
    server.close();
  }
});
