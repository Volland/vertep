// The browser-only scenarios (OPFS in a dedicated Web Worker), in headless Chrome driven over
// the DevTools protocol. Skipped when Chrome is not installed (set CHROME to its binary).
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, normalize } from "node:path";
import { test } from "node:test";
import { SERVER_ACTOR, WS_PROCESSOR, gateFailures, nativeBin, sleep, startWsServer, tempDir, workspace } from "./helpers.mjs";

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
  // CI runners (Ubuntu 24.04) restrict unprivileged user namespaces, which Chrome's sandbox
  // needs; the page under test is our own, so the sandbox is dropped there.
  const sandbox = process.env.CI ? ["--no-sandbox"] : [];
  const proc = spawn(CHROME, ["--headless=new", ...sandbox, "--remote-debugging-port=0", `--user-data-dir=${profile}`, "--no-first-run", "--no-default-browser-check", "about:blank"], { stdio: ["ignore", "ignore", "pipe"] });
  let stderr = "";
  proc.stderr.on("data", (d) => (stderr = (stderr + d).slice(-4000)));
  const portFile = join(profile, "DevToolsActivePort");
  for (let i = 0; i < 400 && !existsSync(portFile) && proc.exitCode === null; i++) await sleep(50);
  if (!existsSync(portFile)) {
    proc.kill("SIGKILL");
    throw new Error(`Chrome did not start (exit ${proc.exitCode}): ${stderr}`);
  }
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
  let b;
  try {
    b = await chrome();
  } catch (e) {
    server.close();
    throw e;
  }
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

    // @tg: verifies:: [[openspec:wasm-bindings#Conformance Suite In WebAssembly#Suite passes in headless Chrome]] {level: e2e}
    await t.test("the W3C IRP suite in headless Chrome gives the native verdicts and passes the gate", async () => {
      const input = execFileSync(nativeBin, ["conformance-input"], { encoding: "utf8", maxBuffer: 256 << 20 });
      const native = JSON.parse(execFileSync(nativeBin, ["conformance-native"], { encoding: "utf8", maxBuffer: 256 << 20 }));
      const verdicts = JSON.parse(await b.evaluate(`vertepTests.conformance(${JSON.stringify(input)})`));
      assert.deepEqual(verdicts, native, "every verdict and failure reason equals the native run's");
      assert.deepEqual(gateFailures(JSON.parse(input), verdicts), [], "no unexpected pass or failure");
    });

    await t.test("WebSocket interop with a native server, across forced disconnects", async (t) => {
      const ws = await startWsServer();
      const serverState = async () => (await ws.cmd("snapshot", { address: SERVER_ACTOR })).context;
      const client = () => b.evaluate("vertepTests.wsClient()");
      const waitFor = async (what, f) => {
        for (let i = 0; i < 500; i++) {
          if (await f()) return;
          await sleep(20);
        }
        throw new Error(`timed out waiting for ${what}`);
      };
      try {
        const address = await b.evaluate(`vertepTests.wsStart(${JSON.stringify(ws.url)})`);

        // @tg: verifies:: [[openspec:wasm-bindings#Server Interop Over WebSocket#Browser actor sends to a server actor]] {level: e2e}
        await t.test("a browser actor sends to a server actor", async () => {
          await b.evaluate("vertepTests.wsGo()");
          await waitFor("the ping", async () => (await serverState()).n === 1);
          assert.equal((await serverState()).ot, WS_PROCESSOR);
          assert.equal((await serverState()).from, address);
        });

        // @tg: verifies:: [[openspec:wasm-bindings#Server Interop Over WebSocket#Server actor replies to a browser actor]] {level: e2e}
        await t.test("the server actor replies to the browser actor", async () => {
          await waitFor("the pong", async () => (await client()).context.n === 1);
          assert.equal((await client()).context.ot, WS_PROCESSOR);
        });

        // @tg: verifies:: [[openspec:wasm-bindings#Server Interop Over WebSocket#Reconnect redelivers the outbox]] {level: e2e}
        await t.test("a send whose ack was lost is redelivered after reconnect and processed once", async () => {
          await ws.ctl("hold");
          await b.evaluate("vertepTests.wsGo()");
          await waitFor("the held ping", async () => (await serverState()).n === 2);
          await ws.ctl("release");
          await ws.ctl("drop");
          await waitFor("the second pong", async () => (await client()).context.n === 2);
          await ws.ctl("dup");
          await b.evaluate("vertepTests.wsGo()");
          await waitFor("the third pong", async () => (await client()).context.n === 3);
          await sleep(300);
          await ws.ctl("nodup");
          const c = await client();
          assert.deepEqual([c.context.n, c.pongs], [3, 3], "each pong processed once");
          assert.equal((await ws.cmd("journal", { address: SERVER_ACTOR })).length, 3, "each ping processed once");
          await b.evaluate("vertepTests.wsHit()");
          assert.equal((await serverState()).hits, 1);
          await b.evaluate("vertepTests.wsStop()");
        });

        // @tg: verifies:: [[openspec:wasm-bindings#Server Interop Over WebSocket#Connection authentication is pluggable]] {level: e2e}
        await t.test("a refused credential rejects pending sends with TransportRefused", async () => {
          const refused = await b.evaluate(`vertepTests.wsRefused(${JSON.stringify(ws.url)})`);
          assert.equal(refused.code, "TransportRefused", JSON.stringify(refused));
        });
      } finally {
        ws.close();
      }
    });
  } finally {
    b.close();
    server.close();
  }
});
