// Browser JS: a Vertep actor system from JavaScript, run here with Node.js.
//
// `@vertep/web` starts the actor system in a worker (a Web Worker in a browser, a worker
// thread in Node.js): the Vertep core compiled to WebAssembly, a shell on the JavaScript event
// loop, and a journal. This main thread only holds a thin typed client; every call returns a
// Promise. The same file runs in a browser page unchanged, with `storage: "opfs"` for a durable
// journal instead of `"memory"`.
//
// Illustrates the wasm-bindings capability (Worker-Hosted Actor System, Actor Creation From
// Chart Sources, Event Sending From JavaScript, Snapshot And Transition Subscriptions, Journal
// Inspection And Time Travel, Node.js Support).
//
// Run with `npm install && npm start` in this directory, after building the JavaScript packages
// (see examples/README.md).
import { createActorSystem } from "@vertep/web";

// A cart on the CEL datamodel: `add` carries a quantity, `checkout` needs a non-empty cart.
const CART = `<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0"
       datamodel="cel" name="cart" initial="empty">
  <datamodel><data id="items" expr="0"/></datamodel>
  <state id="empty">
    <transition event="add" target="filled"><assign location="items" expr="ctx.items + ev.data.qty"/></transition>
  </state>
  <state id="filled">
    <transition event="add"><assign location="items" expr="ctx.items + ev.data.qty"/></transition>
    <transition event="checkout" cond="ctx.items > 0" target="ordered"/>
  </state>
  <state id="ordered"/>
</scxml>`;

const system = await createActorSystem({ storage: "memory" });
try {
  // The chart is validated and content-addressed exactly as natively. Registered under a type
  // IRI and spawned with a key, the actor's address is `<type>/<key>`.
  const cart = await system.spawn(CART, { type: "https://example.org/charts/cart", key: "alice" });
  console.log("actor", await cart.address);

  // One snapshot per macrostep that changes the configuration or the context.
  const seen = [];
  const stop = cart.subscribe((s) => {
    seen.push(s.position);
    console.log(`snapshot @${s.position}: ${JSON.stringify(s.configuration)} items=${s.context.items}`);
  });

  // An event is its name as `type` plus its payload's fields.
  await cart.send({ type: "add", qty: 2 });
  await cart.send({ type: "add", qty: 1 });
  await cart.send({ type: "pay" }); // no transition takes it: journaled, no new snapshot
  await cart.send({ type: "checkout" });
  await until(() => seen.includes(4));
  stop();

  const now = await cart.getSnapshot();
  console.log("now:", JSON.stringify(now.configuration), "matches('ordered') =", now.matches("ordered"));

  // Every accepted event is a journal entry, with the transitions it took. Transitions are
  // IRIs inside the chart version: `<version>#s.<state>:t<index>`.
  console.log("journal:");
  for (const e of await cart.journal()) {
    const taken = (e.transitions ?? []).map((t) => t.split("#")[1]);
    console.log(`  ${e.position} ${e.event.name.padEnd(8)} ${JSON.stringify(e.event.data).padEnd(9)} -> ${JSON.stringify(taken)}`);
  }

  // Time travel: the state after entry 1, rebuilt from the journal.
  const then = await cart.asOf({ position: 1 });
  console.log("as of entry 1:", JSON.stringify(then.configuration), "items =", then.context.items);
} finally {
  await system.terminate();
}

/** Waits until `ready()` holds (subscriptions are delivered asynchronously). */
async function until(ready, timeoutMs = 5000) {
  const deadline = Date.now() + timeoutMs;
  while (!ready()) {
    if (Date.now() > deadline) throw new Error("timed out waiting for snapshots");
    await new Promise((r) => setTimeout(r, 10));
  }
}
