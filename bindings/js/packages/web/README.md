# @vertep/web

W3C SCXML statecharts as journaled actors in the browser and in Node.js. The Vertep core runs in a worker, compiled to WebAssembly; this package is the thin typed client on the main thread, and every call returns a Promise.

- Charts in SCXML, YAML or RDF, validated and content-addressed exactly as in the Rust runtime
- One journal entry per accepted event, with the transitions it took; time travel with `asOf`
- Storage in memory or, in browsers, in the origin private file system (`storage: "opfs"`)
- Datamodels: `null` and CEL built in; ECMAScript from [`@vertep/dm-ecma`](https://www.npmjs.com/package/@vertep/dm-ecma)

## Install

```sh
npm install @vertep/web
```

Node.js 22 or later, or a browser with Web Workers and WebAssembly.

## Use

```js
import { createActorSystem } from "@vertep/web";

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
const cart = await system.spawn(CART, { type: "https://example.org/charts/cart", key: "alice" });

const stop = cart.subscribe((s) => console.log(s.position, s.configuration, s.context.items));
await cart.send({ type: "add", qty: 2 });
await cart.send({ type: "checkout" });

const now = await cart.getSnapshot();
console.log(now.matches("ordered"));                 // true
console.log(await cart.journal());                   // every accepted event, with its transitions
console.log(await cart.asOf({ position: 1 }));       // the state after entry 1, rebuilt by replay

stop();
await system.terminate();
```

Typed charts get generated TypeScript types with `vt gen ts` from the Vertep command line.

## Documentation

Source, documentation and runnable examples: <https://github.com/Volland/vertep> (see `site/` and `examples/browser-js`).

## License

MIT OR Apache-2.0, at your option.
