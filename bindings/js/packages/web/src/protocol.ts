/**
 * The messages between the main-thread client and the worker that hosts the actor system.
 * Every call carries an id and gets exactly one reply; notifications carry no id.
 */
// @tg: implements:: [[openspec:wasm-bindings#Worker-Hosted Actor System]] {confidence: partial, note: "the command protocol"}

import type { WireError } from "./errors.js";

/** A WebAssembly build the worker loads. */
export interface BuildRef {
  glue: string;
  wasm: string;
  datamodels: readonly string[];
  journal: boolean;
}

/** The options the worker starts the system with. */
export interface InitOptions {
  storage: "memory" | "opfs";
  path?: string;
  journal?: boolean;
  dev?: boolean;
  idleTimeoutMs?: number;
  /** URL of the implementations module (named guards and actions). */
  implementations?: string;
  /** A journal file to start from (journaled stores). */
  import?: Uint8Array;
  /** A build to load instead of the bundled ones (for example `@vertep/dm-ecma`). */
  build?: BuildRef;
  /** Charts registered before the first scheduling round. */
  charts?: { source: string; format?: string; type?: string }[];
}

/** Main thread to worker. */
export type ToWorker =
  | { t: "init"; options: InitOptions }
  | { t: "call"; id: number; op: string; args: unknown };

/** Worker to main thread. */
export type FromWorker =
  | { t: "ready"; build: "core" | "journal" | "custom"; version: string }
  | { t: "fatal"; error: WireError }
  | { t: "reply"; id: number; ok: true; value: unknown }
  | { t: "reply"; id: number; ok: false; error: WireError }
  | { t: "notify"; payload: Notification };

/** A notification from the actor system. */
export type Notification =
  | { kind: "snapshot"; sub: number; snapshot: SnapshotData }
  | {
      kind: "transition";
      sub: number;
      address: string;
      position: number;
      event: EventData;
      transitions: TransitionData[];
    }
  | { kind: "impurity"; address: string; position: number; message: string };

/** A snapshot as the worker sends it. */
export interface SnapshotData {
  address: string;
  position: number;
  version: string;
  configuration: string[];
  context: unknown;
  terminated: boolean;
}

/** An SCXML event as the worker sends it. */
export interface EventData {
  name: string;
  data: unknown;
  origin?: string;
  origintype?: string;
  sendid?: string;
  invokeid?: string;
}

/** A taken transition. */
export interface TransitionData {
  iri: string;
  source: string;
  index: number | null;
  targets: string[];
}
