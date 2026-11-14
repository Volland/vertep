/**
 * `@vertep/web`: Vertep statecharts as durable, event-sourced actors in browsers and Node.js.
 *
 * The actor system (the Vertep core compiled to WebAssembly, a shell on the JavaScript event
 * loop, and a tiramemsu journal in OPFS or memory) runs in a dedicated worker; this module is
 * the thin typed client on the main thread. SCXML semantics are normative: there is no mode
 * that changes how charts execute.
 *
 * ```ts
 * import { createActorSystem } from "@vertep/web";
 * const system = await createActorSystem({ storage: "memory" });
 * const order = await system.spawn(scxmlText);
 * order.subscribe((s) => console.log(s.configuration));
 * await order.send({ type: "pay" });
 * ```
 */

export { ActorRef, ActorSystem, createActorSystem } from "./client.js";
export type {
  AnyEvent,
  ChartSpec,
  JournalEntry,
  QueryResults,
  SystemOptions,
  TransitionInfo,
  TransitionNotice,
  Unsubscribe,
  WebSocketTransportOptions,
} from "./client.js";
export { VertepError } from "./errors.js";
export type { ErrorCode, Violation } from "./errors.js";
export type { BuildRef, EventData } from "./protocol.js";
export type { JsonValue, Snapshot } from "./snapshot.js";
export { lockName, takeStoreLock } from "./locks.js";

/** The version of the package, equal to the Rust crates it was built from. */
export const VERSION = "0.1.5";
