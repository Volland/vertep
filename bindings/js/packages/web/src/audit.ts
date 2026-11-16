/**
 * Decision records for an auditor: verify an agent's explanation bundle offline and read the
 * explanations it recomputes, or replay a verified bundle under another chart. No actor system,
 * worker, decider or network is involved; everything comes from the bundle, which an agent host
 * exports (`AgentHost::export_bundle` in Rust, `vt verify-bundle` on the command line).
 *
 * The check runs in the calling thread on the `journal` WebAssembly build (loaded once, on first
 * use); the `core` build has no room for it in its size budget.
 *
 * ```ts
 * import { verifyBundle } from "@vertep/web";
 * const { explanations } = await verifyBundle(await (await fetch("/bundle.json")).text());
 * for (const x of explanations) console.log(x.position, x.state_before, "->", x.state_after);
 * ```
 */
// @tg: implements:: [[openspec:decision-records#Explanation Bundle]] {note: "the TypeScript API"}

import { fromWire, toWire } from "./errors.js";

/** The decider's own account of its choice: a claim, never established by the system. */
export interface Claim {
  readonly claimed_by: string;
  readonly rationale: string | null;
  readonly confidence: number | null;
  /** Always `"unverified claim by the decider"`. */
  readonly label: string;
}

/** What an entry says about a decision or a refusal. */
export interface DecisionRecord {
  readonly correlation: string;
  /** The events offered, in offer order; empty when the entry predates offers. */
  readonly offered: readonly string[];
  readonly offered_known: boolean;
  /** The event chosen; `null` for a refusal. */
  readonly chosen: string | null;
  /** `accepted`, `refused`, ... or `unknown` for an entry written before dispositions. */
  readonly disposition: string;
  readonly decider: string | null;
  readonly decider_kind: string | null;
  readonly fallback: boolean;
  /** The transitions the chosen event took in its first microstep: `[source state, index]`. */
  readonly taken: readonly (readonly [string, number])[];
  readonly claim: Claim | null;
  /** The derivation a rule decider's choice rests on (re-runnable, not a claim). */
  readonly derivation: unknown;
}

/**
 * The explanation of one journal entry, in the canonical JSON of `vertep-agent` (the text the
 * bundle's digests cover, and what `vt explain --json` prints).
 */
export interface Explanation {
  readonly actor: string;
  readonly position: number;
  /** When the input was accepted, in milliseconds since the epoch. */
  readonly ts: number;
  /** The kind of input: `start`, `event`, `decision`, `refusal`, `timer`, ... */
  readonly input: string;
  readonly chart_version: string;
  /** Present, as `"recorded"`, when erased data was replayed: not a full verification. */
  readonly replay?: "recorded";
  readonly state_before: readonly string[];
  readonly state_after: readonly string[];
  readonly entered: readonly string[];
  readonly exited: readonly string[];
  /** What the entry asked the host to do, one line each. */
  readonly effects: readonly string[];
  readonly decision: DecisionRecord | null;
}

/** What a bundle that verified explains. */
export interface VerifiedBundle {
  readonly actor: string;
  /** The first position explained. */
  readonly from: number;
  /** The last position explained. */
  readonly to: number;
  /** How many deciders verification called: always 0. */
  readonly deciderCalls: number;
  /** The recomputed explanations, positions `from` to `to`. */
  readonly explanations: readonly Explanation[];
}

/** One journal position under the recorded chart and the chart tried. */
export interface CounterfactualStep {
  readonly position: number;
  readonly input: string;
  readonly original_state: readonly string[];
  readonly counterfactual_state: readonly string[];
  readonly recorded_disposition: string | null;
  readonly counterfactual_disposition: string | null;
  readonly state_diverged: boolean;
  readonly disposition_changed: boolean;
}

/** The report of a counterfactual replay of a bundle. */
export interface CounterfactualReport {
  readonly actor: string;
  readonly original_version: string;
  readonly counterfactual_version: string;
  /** Where the answers came from: always `"recorded"` for a bundle. */
  readonly decisions: "recorded" | "supplied";
  readonly steps: readonly CounterfactualStep[];
  readonly first_divergence: number | null;
  readonly changed_dispositions: readonly number[];
}

/** A bundle: its JSON text, or the parsed object. */
export type BundleInput = string | Uint8Array | object;

interface AuditGlue {
  default(input: { module_or_path: unknown }): Promise<unknown>;
  verifyBundle?(bundle: string): string;
  counterfactualBundle?(bundle: string, chart: string): string;
}

const isNode =
  typeof process !== "undefined" &&
  !!process.versions?.node &&
  typeof (globalThis as { window?: unknown }).window === "undefined" &&
  typeof (globalThis as { WorkerGlobalScope?: unknown }).WorkerGlobalScope === "undefined";

let glue: Promise<AuditGlue> | null = null;

function build(): Promise<AuditGlue> {
  glue ??= (async () => {
    const g = (await import(new URL("../wasm/journal/vertep_wasm.js", import.meta.url).href)) as AuditGlue;
    const wasm = new URL("../wasm/journal/vertep_wasm_bg.wasm", import.meta.url);
    if (isNode) {
      const { readFile } = await import("node:fs/promises");
      await g.default({ module_or_path: await readFile(wasm) });
    } else {
      await g.default({ module_or_path: wasm });
    }
    return g;
  })();
  return glue;
}

function text(bundle: BundleInput): string {
  if (typeof bundle === "string") return bundle;
  if (bundle instanceof Uint8Array) return new TextDecoder().decode(bundle);
  return JSON.stringify(bundle);
}

function call<T>(f: (() => string) | undefined): T {
  if (!f) throw fromWire({ code: "InvalidArgument", message: "this build cannot verify bundles" });
  try {
    return JSON.parse(f()) as T;
  } catch (e) {
    throw fromWire(toWire(e));
  }
}

/**
 * Verifies an explanation bundle offline and resolves with the explanations it recomputed.
 *
 * Every chart is re-hashed, every entry checked against the manifest, the journal replayed and
 * each explanation recomputed and compared with its digest; no decider is called. Rejects with a
 * `VertepError` of code `BundleInvalid` naming the altered entry, the missing chart or the wrong
 * digest, or `InvalidArgument` for text that is not JSON.
 */
export async function verifyBundle(bundle: BundleInput): Promise<VerifiedBundle> {
  const g = await build();
  return call<VerifiedBundle>(g.verifyBundle && (() => g.verifyBundle!(text(bundle))));
}

/**
 * Verifies a bundle, then replays its journal under `chart` (a chart source, its format guessed
 * unless given) with the decisions the journal recorded, and resolves with the report of where
 * the states and dispositions differ. The bundle is only read.
 */
// @tg: implements:: [[openspec:decision-records#Counterfactual Replay]] {confidence: partial, note: "the TypeScript API, from a bundle"}
export async function counterfactualBundle(
  bundle: BundleInput,
  chart: string | { source: string; format?: string },
): Promise<CounterfactualReport> {
  const g = await build();
  const spec = typeof chart === "string" ? { source: chart } : chart;
  return call<CounterfactualReport>(
    g.counterfactualBundle && (() => g.counterfactualBundle!(text(bundle), JSON.stringify(spec))),
  );
}
