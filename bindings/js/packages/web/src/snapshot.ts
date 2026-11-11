/** Snapshots, as subscribers and `getSnapshot` deliver them. */

import type { SnapshotData } from "./protocol.js";

/** Any JSON value: the type of untyped payloads and contexts. */
export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };

/** An actor's state after a macrostep. */
export interface Snapshot<C = JsonValue, S extends string = string> {
  /** The actor's address IRI. */
  readonly address: string;
  /** The journal position the state is after (0 before the first entry). */
  readonly position: number;
  /** The chart version IRI. */
  readonly version: string;
  /** Active state ids in document order. */
  readonly configuration: readonly S[];
  /** The datamodel's context. */
  readonly context: C;
  /** Whether the session terminated. */
  readonly terminated: boolean;
  /** Whether state `id` is active. */
  matches(id: S): boolean;
}

/** A snapshot with `matches`. */
// @tg: implements:: [[openspec:wasm-bindings#SCXML Semantics In JavaScript]] {confidence: partial, note: "matches on the configuration"}
export function snapshot<C, S extends string>(d: SnapshotData): Snapshot<C, S> {
  const configuration = d.configuration as S[];
  return Object.freeze({
    address: d.address,
    position: d.position,
    version: d.version,
    configuration,
    context: d.context as C,
    terminated: d.terminated,
    matches: (id: S) => configuration.includes(id),
  });
}
