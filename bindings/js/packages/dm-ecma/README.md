<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">@vertep/dm-ecma</h1>

<p align="center">The ECMAScript datamodel for @vertep/web</p>

<p align="center">
  <a href="https://www.npmjs.com/package/@vertep/dm-ecma"><img src="https://img.shields.io/npm/v/@vertep/dm-ecma.svg" alt="npm"></a>
  <a href="https://github.com/Volland/vertep"><img src="https://img.shields.io/badge/github-Volland%2Fvertep-black.svg" alt="GitHub"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/npm/l/@vertep/dm-ecma.svg" alt="license"></a>
</p>

The ECMAScript datamodel for [`@vertep/web`](https://www.npmjs.com/package/@vertep/web). The ECMAScript engine (boa) is too large for the main package's size budget, so it ships here as a separate build of the worker's WebAssembly module: core, shell, journal and the engine.

## Install

```sh
npm install @vertep/web @vertep/dm-ecma
```

## Use

Pass the build to `createActorSystem`, and charts with `datamodel="ecmascript"` run in the worker:

```js
import { createActorSystem } from "@vertep/web";
import { ecmascript } from "@vertep/dm-ecma";

const system = await createActorSystem({ storage: "memory", datamodel: ecmascript });
```

Charts on the `null` and CEL datamodels do not need this package.

## Documentation

- Website and guide: <https://volland.github.io/vertep/>
- Datamodels and types: <https://volland.github.io/vertep/docs/types.html>
- Browser and JavaScript guide: <https://volland.github.io/vertep/docs/browser.html>
- Source: <https://github.com/Volland/vertep> (`bindings/js/packages/dm-ecma`)

## License

MIT OR Apache-2.0, at your option.
