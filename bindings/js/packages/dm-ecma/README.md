# @vertep/dm-ecma

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

Source and documentation: <https://github.com/Volland/vertep>.

## License

MIT OR Apache-2.0, at your option.
