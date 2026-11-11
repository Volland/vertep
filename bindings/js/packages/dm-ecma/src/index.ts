/**
 * The ECMAScript datamodel for `@vertep/web`. The boa engine is too large for the main
 * package's size budget, so it ships here as a separate build of the worker's WebAssembly
 * module (core, shell, journal and boa). Pass it to `createActorSystem`:
 *
 * ```ts
 * import { createActorSystem } from "@vertep/web";
 * import { ecmascript } from "@vertep/dm-ecma";
 * const system = await createActorSystem({ storage: "memory", datamodel: ecmascript });
 * ```
 */
// @tg: implements:: [[openspec:wasm-bindings#npm Package Distribution]] {confidence: partial, note: "the separate ECMAScript datamodel package"}

/** A WebAssembly build the worker can load instead of the default ones. */
export interface WasmBuild {
  /** The wasm-bindgen glue module URL. */
  readonly glue: string;
  /** The WebAssembly module URL. */
  readonly wasm: string;
  /** Datamodels the build adds. */
  readonly datamodels: readonly string[];
  /** Whether the build contains the tiramemsu journal. */
  readonly journal: boolean;
}

/** The worker build with the ECMAScript datamodel (and the journal). */
export const ecmascript: WasmBuild = {
  glue: new URL("../wasm/vertep_wasm.js", import.meta.url).href,
  wasm: new URL("../wasm/vertep_wasm_bg.wasm", import.meta.url).href,
  datamodels: ["ecmascript"],
  journal: true,
};
