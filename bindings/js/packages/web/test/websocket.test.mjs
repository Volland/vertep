// WebSocket interop end to end: an actor system in a Node.js worker thread (the core build in
// WebAssembly) and a native actor system (`vertep-wasm-native ws-server`, the WebSocket
// endpoint of vertep-runtime) exchange events across forced disconnects.
import assert from "node:assert/strict";
import { test } from "node:test";
import { createActorSystem } from "../dist/index.js";
import { CLIENT, CLIENT_TYPE, SERVER_ACTOR, WS_PROCESSOR, startWsServer, sleep, until } from "./helpers.mjs";

const chart = { source: CLIENT, format: "scxml", type: CLIENT_TYPE };

async function system(url, credential) {
  return createActorSystem({
    storage: "memory",
    charts: [chart],
    transports: {
      websocket: { url, credentials: async () => credential, targets: ["https://server.example/"], reconnectMs: 50 },
    },
  });
}

const n = async (actor) => (await actor.getSnapshot()).context.n;

test("WebSocket interop with a native server", async (t) => {
  const server = await startWsServer();
  const s = await system(server.url, "secret");
  try {
    const c1 = s.virtual(chart, "c1");
    const address = await c1.address;

    // @tg: verifies:: [[openspec:wasm-bindings#Server Interop Over WebSocket#Browser actor sends to a server actor]] {level: e2e}
    await t.test("a worker actor sends to a server actor", async () => {
      await c1.send({ type: "go" });
      await until(async () => (await server.cmd("snapshot", { address: SERVER_ACTOR })).context.n === 1, 10_000);
      const snap = await server.cmd("snapshot", { address: SERVER_ACTOR });
      assert.equal(snap.context.ot, WS_PROCESSOR, "_event.origintype names the WebSocket processor");
      assert.equal(snap.context.from, address, "_event.origin is the worker actor's IRI");
    });

    // @tg: verifies:: [[openspec:wasm-bindings#Server Interop Over WebSocket#Server actor replies to a browser actor]] {level: e2e}
    await t.test("the server actor replies to the worker actor", async () => {
      await until(async () => (await n(c1)) === 1, 10_000);
      assert.equal((await c1.getSnapshot()).context.ot, WS_PROCESSOR);
    });

    // @tg: verifies:: [[openspec:wasm-bindings#Server Interop Over WebSocket#Reconnect redelivers the outbox]] {level: e2e}
    await t.test("a send journaled before a dropped acknowledgement is redelivered and processed once", async () => {
      await server.ctl("hold"); // the server journals the ping but sends no ack
      await c1.send({ type: "go" });
      await until(async () => (await server.cmd("snapshot", { address: SERVER_ACTOR })).context.n === 2, 10_000);
      await server.ctl("release");
      await server.ctl("drop"); // the connection drops before the ack went out
      await until(async () => (await n(c1)) === 2, 10_000);
      // every server message is sent twice: the worker actor processes each once
      await server.ctl("dup");
      await c1.send({ type: "go" });
      await until(async () => (await n(c1)) === 3, 10_000);
      await sleep(300);
      await server.ctl("nodup");
      assert.equal(await n(c1), 3);
      const serverJournal = await server.cmd("journal", { address: SERVER_ACTOR });
      assert.equal(serverJournal.length, 3, "three pings, each journaled once");
      assert.equal(new Set(serverJournal.map((e) => e.messageId)).size, 3);
      const clientJournal = await c1.journal();
      assert.equal(clientJournal.filter((e) => e.event?.name === "pong").length, 3, "three pongs, each journaled once");
    });

    await t.test("a send from JavaScript to a server actor resolves once acknowledged", async () => {
      await s.actor(SERVER_ACTOR).send({ type: "hit" });
      assert.equal((await server.cmd("snapshot", { address: SERVER_ACTOR })).context.hits, 1);
    });
  } finally {
    await s.terminate();
    server.close();
  }
});

// @tg: verifies:: [[openspec:wasm-bindings#Server Interop Over WebSocket#Connection authentication is pluggable]] {level: e2e}
test("a refused credential rejects pending sends with TransportRefused", async () => {
  const server = await startWsServer();
  const s = await system(server.url, "wrong");
  try {
    await assert.rejects(s.actor(SERVER_ACTOR).send({ type: "hit" }), (e) => e.code === "TransportRefused");
    await assert.rejects(s.actor(SERVER_ACTOR).send({ type: "hit" }), (e) => e.code === "TransportRefused");
    assert.equal((await server.cmd("snapshot", { address: SERVER_ACTOR })).context.hits, 0);
  } finally {
    await s.terminate();
    server.close();
  }
});
