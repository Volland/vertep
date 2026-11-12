<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">@vertep/web</h1>

<p align="center">W3C SCXML statecharts as journaled actors, in the browser and in Node.js</p>

<p align="center">
  <a href="https://www.npmjs.com/package/@vertep/web"><img src="https://img.shields.io/npm/v/@vertep/web.svg" alt="npm"></a>
  <a href="https://github.com/Volland/vertep"><img src="https://img.shields.io/badge/github-Volland%2Fvertep-black.svg" alt="GitHub"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/npm/l/@vertep/web.svg" alt="license"></a>
</p>

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

- Website and guide: <https://volland.github.io/vertep/>
- Browser and JavaScript guide: <https://volland.github.io/vertep/docs/browser.html>
- The example above, complete with its output: <https://volland.github.io/vertep/examples/browser-js.html>
- All examples: <https://volland.github.io/vertep/examples/>
- Why deterministic agents: <https://volland.github.io/vertep/articles/deterministic-agents.html>
- Source: <https://github.com/Volland/vertep> (`bindings/js/packages/web`)

## License

MIT OR Apache-2.0, at your option.
