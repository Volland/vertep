/**
 * The worker entry: hosts the actor system (core, shell, storage and the application's
 * implementations) in one dedicated worker and serves the client's commands.
 *
 * - It loads the WebAssembly build the storage needs: `core` (no SQLite) for plain `memory`
 *   stores, `journal` (tiramemsu on SQLite in WebAssembly) the first time a journaled store
 *   opens, or a build the application passes (such as `@vertep/dm-ecma`).
 * - For `opfs` it probes for OPFS sync access handles (`StorageUnavailable` without them, and
 *   no memory store is opened in their place) and takes the store's Web Lock (`StoreLocked`
 *   for a second holder) before opening the file.
 * - It imports the implementations module and binds its `guards` and `actions` exports as
 *   named functions, which steps call synchronously in this thread.
 * - With `transports.websocket` it connects the WebSocket Event I/O Processor, asking the main
 *   thread's credential provider before each connection; a `send` to a server actor replies
 *   once the server acknowledged it, or fails with `TransportRefused`.
 */
// @tg: implements:: [[openspec:wasm-bindings#Worker-Hosted Actor System]]
// @tg: implements:: [[openspec:wasm-bindings#Browser Journal Storage]] {confidence: partial, note: "storage choice, OPFS probe and Web Lock in the worker"}
// @tg: implements:: [[openspec:wasm-bindings#Bundle Size Budget]] {confidence: partial, note: "the SQLite build is loaded only for journaled stores"}
// @tg: implements:: [[openspec:wasm-bindings#Server Interop Over WebSocket]] {confidence: partial, note: "credential provider round trip and acknowledged sends"}

import { VertepError, fromWire, toWire } from "./errors.js";
import type { BuildRef, FromWorker, InitOptions, ToWorker } from "./protocol.js";
import { type Release, takeStoreLock } from "./locks.js";

interface WasmActorSystem {
  command(op: string, args: string): string;
  request(to: string, event: string, timeoutMs: number): Promise<string>;
  exportJournal(): Uint8Array;
  onNotify(f: (n: string) => void): void;
  connect(credentials?: () => Promise<string | null>): void;
  close(): void;
}

interface Glue {
  default(input: { module_or_path: unknown }): Promise<unknown>;
  ActorSystem: {
    create(options: string, implementations: unknown, importBytes?: Uint8Array): Promise<WasmActorSystem>;
  };
  version(): string;
  hasJournal(): boolean;
}

/**
 * Node's `worker_threads`, not a Web Worker. Electron hosts with Node integration in workers
 * (Obsidian, for one) set `process.versions.node` inside Web Workers too, so a Web Worker scope
 * wins over `process`.
 */
const isNode =
  typeof process !== "undefined" &&
  !!process.versions?.node &&
  typeof (globalThis as { window?: unknown }).window === "undefined" &&
  typeof (globalThis as { WorkerGlobalScope?: unknown }).WorkerGlobalScope === "undefined";

type Post = (m: FromWorker, transfer?: Transferable[]) => void;

async function port(): Promise<{ post: Post; listen: (f: (m: ToWorker) => void) => void }> {
  if (isNode) {
    const { parentPort } = await import("node:worker_threads");
    if (!parentPort) throw new Error("@vertep/web/worker runs inside a worker thread");
    return {
      post: (m, transfer) => parentPort.postMessage(m, (transfer ?? []) as never),
      listen: (f) => parentPort.on("message", f),
    };
  }
  const scope = globalThis as unknown as {
    postMessage(m: unknown, transfer?: Transferable[]): void;
    addEventListener(t: "message", f: (e: MessageEvent) => void): void;
  };
  return {
    post: (m, transfer) => scope.postMessage(m, transfer ?? []),
    listen: (f) => scope.addEventListener("message", (e) => f(e.data as ToWorker)),
  };
}

/** The bundled builds. */
function bundled(kind: "core" | "journal"): BuildRef {
  return {
    glue: new URL(`../wasm/${kind}/vertep_wasm.js`, import.meta.url).href,
    wasm: new URL(`../wasm/${kind}/vertep_wasm_bg.wasm`, import.meta.url).href,
    datamodels: [],
    journal: kind === "journal",
  };
}

async function load(build: BuildRef): Promise<Glue> {
  const glue = (await import(build.glue)) as Glue;
  if (isNode) {
    const { readFile } = await import("node:fs/promises");
    const bytes = await readFile(new URL(build.wasm));
    await glue.default({ module_or_path: bytes });
  } else {
    await glue.default({ module_or_path: new URL(build.wasm) });
  }
  return glue;
}

/** Whether OPFS sync access handles exist here (a browser's dedicated worker). */
function opfsAvailable(): boolean {
  const g = globalThis as {
    navigator?: { storage?: { getDirectory?: unknown } };
    FileSystemSyncAccessHandle?: unknown;
    FileSystemFileHandle?: { prototype?: { createSyncAccessHandle?: unknown } };
  };
  return (
    !isNode &&
    typeof g.navigator?.storage?.getDirectory === "function" &&
    typeof g.FileSystemSyncAccessHandle === "function" &&
    typeof g.FileSystemFileHandle?.prototype?.createSyncAccessHandle === "function"
  );
}

/** The application's event as it reaches guards and actions: `{type, ...fields}` or `{type, data}`. */
function userEvent(e: { name: string; data: unknown; [k: string]: unknown }): Record<string, unknown> {
  const { name, data, ...meta } = e;
  const base: Record<string, unknown> =
    data !== null && typeof data === "object" && !Array.isArray(data) ? { ...(data as object) } : data === null ? {} : { data };
  return { type: name, ...base, _meta: meta };
}

type Guard = (args: { context: any; event: any }) => unknown;
type Action = (args: {
  context: any;
  event: any;
  raise(event: { type: string; [k: string]: unknown }): void;
  send(event: { type: string; [k: string]: unknown }, options?: { target?: string; delay?: number }): void;
}) => unknown;

function payload(e: { type: string; [k: string]: unknown }): unknown {
  const { type: _t, ...rest } = e;
  if (Object.keys(rest).length === 1 && "data" in rest) return rest.data;
  return Object.keys(rest).length === 0 ? null : rest;
}

/** Wraps an implementations module for the Rust side: two functions over JSON text. */
async function implementations(url: string): Promise<{ guards: string[]; actions: string[]; wrapper: unknown }> {
  const m = (await import(url)) as { guards?: Record<string, Guard>; actions?: Record<string, Action> };
  const guards = m.guards ?? {};
  const actions = m.actions ?? {};
  const wrapper = {
    guard(iri: string, ctx: string, ev: string): boolean {
      const g = guards[iri];
      if (!g) throw new Error(`no guard ${iri} in the implementations module`);
      return !!g({ context: JSON.parse(ctx), event: userEvent(JSON.parse(ev)) });
    },
    action(iri: string, ctx: string, ev: string): string {
      const a = actions[iri];
      if (!a) throw new Error(`no action ${iri} in the implementations module`);
      const context = JSON.parse(ctx);
      const raise: unknown[] = [];
      const send: unknown[] = [];
      const r = a({
        context,
        event: userEvent(JSON.parse(ev)),
        raise: (e) => raise.push({ name: e.type, data: payload(e) }),
        send: (e, o = {}) => send.push({ name: e.type, data: payload(e), target: o.target, delay: o.delay ?? 0 }),
      });
      const next = r && typeof r === "object" && !Array.isArray(r) ? r : context;
      return JSON.stringify({ context: next, raise, send });
    },
  };
  return { guards: Object.keys(guards), actions: Object.keys(actions), wrapper };
}

async function main(): Promise<void> {
  const { post, listen } = await port();
  let system: WasmActorSystem | null = null;
  let release: Release | null = null;
  // sends to server actors waiting for the server's acknowledgement, by message id
  const deliveries = new Map<string, { resolve: () => void; reject: (e: unknown) => void }[]>();
  // credential requests to the main thread, by id
  const credentials = new Map<number, { resolve: (v: string | null) => void; reject: (e: unknown) => void }>();
  let nextCredential = 1;
  const askCredential = (): Promise<string | null> =>
    new Promise((resolve, reject) => {
      const id = nextCredential++;
      credentials.set(id, { resolve, reject });
      post({ t: "credential", id });
    });
  const notify = (text: string): void => {
    const n = JSON.parse(text);
    if (n.kind === "delivery") {
      const waiting = deliveries.get(n.messageId) ?? [];
      deliveries.delete(n.messageId);
      for (const w of waiting) {
        if (n.ok) w.resolve();
        else w.reject(fromWire(n.error));
      }
      return;
    }
    post({ t: "notify", payload: n });
  };
  const queue: ToWorker[] = [];
  let starting = true;

  const handle = async (m: ToWorker): Promise<void> => {
    if (m.t === "init") {
      try {
        const o: InitOptions = m.options;
        const journaled = o.storage === "opfs" || !!o.journal || !!o.import;
        if (o.storage === "opfs") {
          if (!opfsAvailable()) {
            throw new VertepError(
              "StorageUnavailable",
              "OPFS sync access handles are unavailable here (they exist only in a browser's dedicated worker); no memory store was opened instead",
            );
          }
          release = await takeStoreLock(o.path ?? "vertep.db");
        }
        const build = o.build ?? bundled(journaled ? "journal" : "core");
        if (journaled && !build.journal) {
          throw new VertepError("StorageUnavailable", "the selected build has no journal storage");
        }
        const glue = await load(build);
        const impl = o.implementations ? await implementations(o.implementations) : null;
        const options = {
          storage: o.storage,
          path: o.path ?? "vertep.db",
          journal: journaled,
          dev: !!o.dev,
          idleTimeoutMs: o.idleTimeoutMs,
          guards: impl?.guards ?? [],
          actions: impl?.actions ?? [],
          websocket: o.websocket
            ? { url: o.websocket.url, node: o.websocket.node, targets: o.websocket.targets, hosts: o.websocket.hosts, reconnectMs: o.websocket.reconnectMs }
            : undefined,
        };
        system = await glue.ActorSystem.create(JSON.stringify(options), impl?.wrapper ?? null, o.import);
        system.onNotify(notify);
        if (o.websocket) system.connect(o.websocket.credentials ? askCredential : undefined);
        // the first command runs the first scheduling round, which fires overdue timers
        for (const c of o.charts ?? []) system.command("register", JSON.stringify(c));
        post({ t: "ready", build: o.build ? "custom" : journaled ? "journal" : "core", version: glue.version() });
      } catch (e) {
        void release?.();
        post({ t: "fatal", error: toWire(e) });
      }
      return;
    }
    if (m.t !== "call") return;
    const { id, op, args } = m;
    try {
      if (!system) throw new VertepError("InvalidArgument", "the actor system is not started");
      let value: unknown;
      if (op === "request") {
        const a = args as { to: string; event: unknown; timeoutMs: number };
        value = JSON.parse(await system.request(a.to, JSON.stringify(a.event), a.timeoutMs));
      } else if (op === "exportJournal") {
        value = system.exportJournal();
      } else if (op === "send") {
        const r = JSON.parse(system.command(op, JSON.stringify(args ?? {}))) as { messageId: string; pending?: boolean };
        if (r.pending) {
          // a server actor: done once the server acknowledged it
          await new Promise<void>((resolve, reject) => {
            const list = deliveries.get(r.messageId) ?? [];
            list.push({ resolve, reject });
            deliveries.set(r.messageId, list);
          });
        }
        value = { messageId: r.messageId };
      } else if (op === "close") {
        system.close();
        await release?.();
        release = null;
        value = null;
      } else {
        value = JSON.parse(system.command(op, JSON.stringify(args ?? {})));
      }
      post({ t: "reply", id, ok: true, value });
    } catch (e) {
      post({ t: "reply", id, ok: false, error: toWire(e) });
    }
  };

  listen((m) => {
    if (m.t === "credential") {
      const c = credentials.get(m.id);
      credentials.delete(m.id);
      if (c && m.ok) c.resolve(m.value);
      else if (c && !m.ok) c.reject(fromWire(m.error));
      return;
    }
    if (starting && m.t !== "init") {
      queue.push(m);
      return;
    }
    if (m.t === "init") {
      void handle(m).then(() => {
        starting = false;
        for (const q of queue.splice(0)) void handle(q);
      });
      return;
    }
    void handle(m);
  });
}

void main();
