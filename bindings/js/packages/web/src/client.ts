/**
 * The main-thread client: a typed API over the worker that hosts the actor system. No call
 * blocks the page: every call returns a Promise or delivers results through a subscription.
 */
// @tg: implements:: [[openspec:wasm-bindings#Worker-Hosted Actor System]]
// @tg: implements:: [[openspec:wasm-bindings#Node.js Support]] {confidence: partial, note: "the same client on worker_threads"}

import { VertepError, fromWire } from "./errors.js";
import type { BuildRef, EventData, FromWorker, InitOptions, Notification, SnapshotData, ToWorker, TransitionData } from "./protocol.js";
import { type JsonValue, type Snapshot, snapshot } from "./snapshot.js";

/** An event: its name as `type`, and its payload's fields (or `data` for a non-record payload). */
export type AnyEvent = { type: string; [field: string]: unknown };

/** A chart source with the TypeScript types `vt gen ts` generated for it. */
export interface ChartSpec<E extends AnyEvent = AnyEvent, C = JsonValue, S extends string = string> {
  /** The chart source. */
  readonly source: string;
  /** Its surface. */
  readonly format: "scxml" | "yaml" | "rdf";
  /** The chart type IRI to register it under; its version IRI when absent. */
  readonly type?: string;
  /** Phantom field carrying the types; never set at runtime. */
  readonly __types?: { event: E; context: C; state: S };
}

/** Options of `createActorSystem`. */
export interface SystemOptions {
  /** Durable OPFS storage or volatile memory; there is no default and no fallback. */
  storage: "opfs" | "memory";
  /** The store path (the tiramemsu file name). */
  path?: string;
  /** For `memory`: keep a tiramemsu journal file (exportable). `opfs` always does. */
  journal?: boolean;
  /** The implementations module: its `guards` and `actions` exports become named functions. */
  implementations?: URL | string;
  /** Development mode: every macrostep is replayed and impure guards or actions reported. */
  dev?: boolean;
  /** Called with each impurity report in development mode. */
  onImpurity?: (report: { address: string; position: number; message: string }) => void;
  /** Idle deactivation of virtual actors, in milliseconds. */
  idleTimeoutMs?: number;
  /** A journal file to start from (with `journal: true`). */
  import?: Uint8Array;
  /**
   * Charts to register before the system's first scheduling round, which recovers what an
   * earlier run left: overdue durable timers of these charts' actors fire once, right after.
   */
  charts?: (ChartSpec<any, any, any> | { source: string; format?: string; type?: string })[];
  /** Another worker build, for example `ecmascript` from `@vertep/dm-ecma`. */
  datamodel?: BuildRef;
  /** Transports (see `WebSocketTransportOptions`). */
  transports?: { websocket?: unknown };
  /** The worker entry; the bundled one by default. */
  workerUrl?: URL | string;
}

/** A transition taken by a macrostep. */
export interface TransitionInfo {
  readonly source: string;
  readonly targets: readonly string[];
  readonly iri: string;
}

/** A transition notification. */
export interface TransitionNotice {
  readonly address: string;
  readonly position: number;
  readonly event: EventData;
  readonly transitions: readonly TransitionInfo[];
}

/** A journal entry. */
export interface JournalEntry {
  readonly position: number;
  readonly kind: "input" | "migration" | "fork";
  readonly version: string;
  readonly messageId: string;
  readonly timestamp: number;
  readonly digest: string;
  readonly event?: EventData;
  readonly transitions?: readonly string[];
  readonly timer?: string;
}

/** SPARQL results: `{variables, rows}`, `{boolean}` or `{triples}`. */
export type QueryResults =
  | { variables: string[]; rows: Record<string, string>[] }
  | { boolean: boolean }
  | { triples: string[] };

/** Stops a subscription. */
export type Unsubscribe = () => void;

interface Port {
  post(m: ToWorker, transfer?: Transferable[]): void;
  terminate(): void;
}

const isNode = typeof process !== "undefined" && !!process.versions?.node && typeof (globalThis as { window?: unknown }).window === "undefined";

async function startWorker(
  url: URL,
  onMessage: (m: FromWorker) => void,
  onDeath: (why: string) => void,
): Promise<Port> {
  if (!isNode && typeof Worker === "function") {
    const w = new Worker(url, { type: "module" });
    w.onmessage = (e: MessageEvent) => onMessage(e.data as FromWorker);
    w.onerror = (e: ErrorEvent) => onDeath(e.message || "the worker failed");
    return {
      post: (m, transfer) => w.postMessage(m, transfer ?? []),
      terminate: () => w.terminate(),
    };
  }
  const { Worker: NodeWorker } = await import("node:worker_threads");
  const w = new NodeWorker(url);
  w.on("message", (m: FromWorker) => onMessage(m));
  w.on("error", (e: Error) => onDeath(e.message));
  w.on("exit", (code: number) => onDeath(`the worker exited with code ${code}`));
  return {
    post: (m, transfer) => w.postMessage(m, (transfer ?? []) as never),
    terminate: () => void w.terminate(),
  };
}

/** Converts a client event to the wire form. */
function wireEvent(e: AnyEvent): AnyEvent {
  if (!e || typeof e.type !== "string" || e.type === "") {
    throw new VertepError("InvalidArgument", "an event has a non-empty type");
  }
  return e;
}

let messageCounter = 0;

/** A fresh message id. */
function messageId(): string {
  const rand =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `${Date.now().toString(36)}-${(messageCounter++).toString(36)}-${Math.random().toString(36).slice(2)}`;
  return `urn:uuid:${rand}`;
}

/** The actor system running in a worker. */
export class ActorSystem {
  #port: Port;
  #next = 1;
  #pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: unknown) => void }>();
  #subs = new Map<number, (n: Notification) => void>();
  #dead: VertepError | null = null;
  #registered = new Map<string, Promise<string>>();
  #onImpurity?: SystemOptions["onImpurity"];
  /** Which WebAssembly build the worker loaded: `core` (no SQLite), `journal` or `custom`. */
  readonly build: "core" | "journal" | "custom";

  /** @internal */
  constructor(port: Port, build: "core" | "journal" | "custom", onImpurity?: SystemOptions["onImpurity"]) {
    this.#port = port;
    this.build = build;
    this.#onImpurity = onImpurity;
  }

  /** @internal */
  _receive(m: FromWorker): void {
    if (m.t === "reply") {
      const p = this.#pending.get(m.id);
      if (!p) return;
      this.#pending.delete(m.id);
      if (m.ok) p.resolve(m.value);
      else p.reject(fromWire(m.error));
    } else if (m.t === "notify") {
      const n = m.payload;
      if (n.kind === "impurity") {
        if (this.#onImpurity) this.#onImpurity(n);
        else console.warn(`vertep: ${n.message} (${n.address} at ${n.position})`);
        return;
      }
      this.#subs.get(n.sub)?.(n);
    }
  }

  /** @internal */
  _die(why: string): void {
    if (this.#dead) return;
    this.#dead = new VertepError("WorkerTerminated", `the actor system's worker terminated: ${why}`);
    for (const p of this.#pending.values()) p.reject(this.#dead);
    this.#pending.clear();
    this.#subs.clear();
  }

  /** @internal Sends a command to the worker. */
  _call<T>(op: string, args: unknown, transfer?: Transferable[]): Promise<T> {
    if (this.#dead) return Promise.reject(this.#dead);
    const id = this.#next++;
    return new Promise<T>((resolve, reject) => {
      this.#pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      this.#port.post({ t: "call", id, op, args }, transfer);
    });
  }

  /** @internal */
  _subscribe(address: Promise<string>, kind: "snapshot" | "transitions", f: (n: Notification) => void): Unsubscribe {
    let id: number | null = null;
    let stopped = false;
    const ready = address
      .then((a) => this._call<{ id: number }>("subscribe", { address: a, kind }))
      .then((r) => {
        id = r.id;
        if (stopped) void this._call("unsubscribe", { id }).catch(() => undefined);
        else this.#subs.set(r.id, f);
      });
    ready.catch(() => undefined);
    return () => {
      stopped = true;
      if (id !== null) {
        this.#subs.delete(id);
        void this._call("unsubscribe", { id }).catch(() => undefined);
      }
    };
  }

  /** Registers a chart under a type (its version IRI by default); resolves to the type. */
  async register(chart: ChartSpec<any, any, any> | string, options: { format?: string; type?: string } = {}): Promise<{ type: string; version: string }> {
    const spec = typeof chart === "string" ? { source: chart, format: options.format, type: options.type } : chart;
    return this._call("register", { source: spec.source, format: spec.format, type: options.type ?? spec.type });
  }

  #typeOf(chart: ChartSpec<any, any, any> | string): Promise<string> {
    if (typeof chart === "string") return Promise.resolve(chart);
    let p = this.#registered.get(chart.source);
    if (!p) {
      p = this.register(chart).then((r) => r.type);
      this.#registered.set(chart.source, p);
    }
    return p;
  }

  /**
   * Starts an actor from a chart source (SCXML, YAML or RDF). The chart is validated and
   * content-addressed exactly as natively; an invalid chart rejects with `ChartInvalid`.
   */
  async spawn<E extends AnyEvent = AnyEvent, C = JsonValue, S extends string = string>(
    chart: ChartSpec<E, C, S> | string,
    options: { format?: "scxml" | "yaml" | "rdf"; type?: string; key?: string } = {},
  ): Promise<ActorRef<E, C, S>> {
    const spec = typeof chart === "string" ? { source: chart, format: options.format } : chart;
    const r = await this._call<{ address: string; snapshot: SnapshotData }>("spawn", {
      source: spec.source,
      format: spec.format,
      type: options.type ?? (spec as ChartSpec).type,
      key: options.key,
    });
    return new ActorRef<E, C, S>(this, Promise.resolve(r.address));
  }

  /**
   * The virtual actor of `chart` with `key`: it is activated by its first message, and every
   * later call with the same type and key reaches the same actor.
   */
  virtual<E extends AnyEvent = AnyEvent, C = JsonValue, S extends string = string>(
    chart: ChartSpec<E, C, S> | string,
    key: string,
  ): ActorRef<E, C, S> {
    const address = this.#typeOf(chart).then((type) =>
      this._call<{ address: string }>("virtual", { type, key }).then((r) => r.address),
    );
    address.catch(() => undefined);
    return new ActorRef<E, C, S>(this, address);
  }

  /** The actor at an address IRI. */
  actor<E extends AnyEvent = AnyEvent, C = JsonValue, S extends string = string>(address: string): ActorRef<E, C, S> {
    return new ActorRef<E, C, S>(this, Promise.resolve(address));
  }

  /** Runs a SPARQL query on the graph store, now or as believed at `time` (epoch ms). */
  query(sparql: string, options: { time?: number } = {}): Promise<QueryResults> {
    return this._call("query", { sparql, time: options.time });
  }

  /** The journal file (journaled stores), to download or to open natively. */
  exportJournal(): Promise<Uint8Array> {
    return this._call("exportJournal", {});
  }

  /** Every actor with a journal. */
  actors(): Promise<string[]> {
    return this._call("actors", {});
  }

  /** Stops the worker; later calls reject with `WorkerTerminated`. */
  async terminate(): Promise<void> {
    if (this.#dead) return;
    await this._call("close", {}).catch(() => undefined);
    this.#port.terminate();
    this._die("terminated by the application");
  }
}

/** A reference to one actor. Every method returns a Promise or a subscription. */
export class ActorRef<E extends AnyEvent = AnyEvent, C = JsonValue, S extends string = string> {
  readonly #system: ActorSystem;
  /** The actor's address IRI. */
  readonly address: Promise<string>;

  /** @internal */
  constructor(system: ActorSystem, address: Promise<string>) {
    this.#system = system;
    this.address = address;
  }

  /**
   * Sends an event. It carries `messageId` (a fresh one by default), so a duplicate delivery
   * of the same id is journaled and processed once.
   */
  async send(event: E, options: { messageId?: string } = {}): Promise<{ messageId: string }> {
    const to = await this.address;
    return this.#system._call("send", { to, event: wireEvent(event), messageId: options.messageId ?? messageId() });
  }

  /**
   * Sends an event and resolves with the chart's `vt:reply`; rejects with `RequestTimeout`
   * after `timeoutMs` (a late reply is dropped).
   */
  async request(event: E, options: { timeoutMs?: number } = {}): Promise<EventData> {
    const to = await this.address;
    return this.#system._call("request", { to, event: wireEvent(event), timeoutMs: options.timeoutMs ?? 30_000 });
  }

  /** The current snapshot. */
  async getSnapshot(): Promise<Snapshot<C, S>> {
    const address = await this.address;
    return snapshot<C, S>(await this.#system._call<SnapshotData>("snapshot", { address }));
  }

  /**
   * Whether the actor accepts `event` now: it enables a transition or an active state defers
   * it. Nothing changes and nothing is journaled.
   */
  async can(event: E): Promise<boolean> {
    const address = await this.address;
    const r = await this.#system._call<{ can: boolean }>("can", { address, event: wireEvent(event) });
    return r.can;
  }

  /** The journal entries from position `from`. */
  async journal(options: { from?: number } = {}): Promise<JournalEntry[]> {
    const address = await this.address;
    return this.#system._call("journal", { address, from: options.from ?? 0 });
  }

  /** The snapshot after a past journal position, or as of a transaction time; read-only. */
  async asOf(at: { position: number } | { time: number }): Promise<Snapshot<C, S>> {
    const address = await this.address;
    return snapshot<C, S>(await this.#system._call<SnapshotData>("asOf", { address, ...at }));
  }

  /** Calls `f` with one snapshot per macrostep that changes the configuration or context. */
  subscribe(f: (s: Snapshot<C, S>) => void): Unsubscribe {
    return this.#system._subscribe(this.address, "snapshot", (n) => {
      if (n.kind === "snapshot") f(snapshot<C, S>(n.snapshot));
    });
  }

  /** Calls `f` with the triggering event and the transitions taken, per macrostep. */
  onTransition(f: (t: TransitionNotice) => void): Unsubscribe {
    return this.#system._subscribe(this.address, "transitions", (n) => {
      if (n.kind === "transition") {
        f({
          address: n.address,
          position: n.position,
          event: n.event,
          transitions: n.transitions.map((t: TransitionData) => ({ source: t.source, targets: t.targets, iri: t.iri })),
        });
      }
    });
  }
}

/**
 * Starts an actor system in a dedicated worker (a Web Worker in browsers, a worker thread in
 * Node.js) and resolves once its store is open.
 */
// @tg: implements:: [[openspec:wasm-bindings#Worker-Hosted Actor System]]
export async function createActorSystem(options: SystemOptions): Promise<ActorSystem> {
  if (options.storage !== "opfs" && options.storage !== "memory") {
    throw new VertepError("InvalidArgument", 'storage must be chosen explicitly: "opfs" or "memory"');
  }
  const url = options.workerUrl ? new URL(String(options.workerUrl)) : new URL("./worker.js", import.meta.url);
  let system: ActorSystem | null = null;
  let settle: { resolve: (s: ActorSystem) => void; reject: (e: unknown) => void } | null = null;
  const started = new Promise<ActorSystem>((resolve, reject) => {
    settle = { resolve, reject };
  });
  const port = await startWorker(
    url,
    (m) => {
      if (m.t === "ready") {
        system = new ActorSystem(port, m.build, options.onImpurity);
        settle?.resolve(system);
      } else if (m.t === "fatal") {
        settle?.reject(fromWire(m.error));
        port.terminate();
      } else {
        system?._receive(m);
      }
    },
    (why) => {
      if (system) system._die(why);
      else settle?.reject(new VertepError("WorkerTerminated", `the actor system's worker terminated: ${why}`));
    },
  );
  const init: InitOptions = {
    storage: options.storage,
    path: options.path,
    journal: options.journal,
    dev: options.dev,
    idleTimeoutMs: options.idleTimeoutMs,
    implementations: options.implementations ? String(options.implementations) : undefined,
    import: options.import,
    build: options.datamodel,
    charts: options.charts?.map((c) => ({ source: c.source, format: c.format, type: c.type })),
  };
  port.post({ t: "init", options: init });
  return started;
}
