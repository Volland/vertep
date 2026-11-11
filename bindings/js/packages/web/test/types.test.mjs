// Generated TypeScript types: `vt gen ts`, checked by the TypeScript compiler.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { ORDER, workspace } from "./helpers.mjs";
import { createActorSystem } from "../dist/index.js";

const vt = join(workspace, ".native", "vt");
const fixture = (f) => fileURLToPath(new URL(`./fixtures/${f}`, import.meta.url));
const tsc = join(workspace, "node_modules", "typescript", "bin", "tsc");

function project(files) {
  const base = join(workspace, ".test-tmp");
  mkdirSync(base, { recursive: true });
  const dir = mkdtempSync(join(base, "types-"));
  writeFileSync(
    join(dir, "tsconfig.json"),
    JSON.stringify({
      compilerOptions: {
        target: "ES2022",
        module: "ES2022",
        moduleResolution: "Bundler",
        lib: ["ES2022", "DOM"],
        types: [],
        strict: true,
        noEmit: true,
        skipLibCheck: true,
      },
      include: ["*.ts"],
    }),
  );
  for (const [name, text] of Object.entries(files)) writeFileSync(join(dir, name), text);
  const r = spawnSync(process.execPath, [tsc, "-p", dir], { encoding: "utf8" });
  return { ok: r.status === 0, out: r.stdout + r.stderr };
}

const gen = (file, ...args) => execFileSync(vt, ["gen", "ts", file, ...args], { encoding: "utf8" });

// @tg: verifies:: [[openspec:wasm-bindings#Generated TypeScript Types#Ill-typed event is a compile error]] {level: integration}
test("ill-typed events are compile errors", () => {
  const usage = `import type { ActorSystem } from "@vertep/web";
import { createShopActor } from "./shop.chart";
declare const system: ActorSystem;
export async function use() {
  const a = createShopActor(system, "k");
  await a.send({ type: "ping" });
  await a.send({ type: "count", data: 3 });
  await a.send({ type: "pay", data: { tag: "Cash" } });
  await a.send({ type: "submit", id: "o1", lines: [{ sku: "A", qty: 2 }], total: "12.50", pay: { tag: "Card", value: { last4: "4242" } } });
  // @ts-expect-error an event name outside the union
  await a.send({ type: "refund" });
  // @ts-expect-error a payload of the wrong type
  await a.send({ type: "count", data: "three" });
  // @ts-expect-error a missing payload field
  await a.send({ type: "submit", id: "o1", lines: [], total: 1 });
  // @ts-expect-error an undeclared payload field
  await a.send({ type: "ping", extra: 1 });
  // @ts-expect-error a wrong union tag
  await a.send({ type: "pay", data: { tag: "Cheque" } });
  const s = await a.getSnapshot();
  s.matches("payment");
  // @ts-expect-error a state that does not exist
  s.matches("nowhere");
  const total: number | string = s.context.total;
  // @ts-expect-error a context field that does not exist
  s.context.missing;
  return total;
}
`;
  const r = project({ "shop.chart.ts": gen(fixture("shop.scxml")), "usage.ts": usage });
  assert.ok(r.ok, r.out);
  // and the same file without the expectations does fail, at each of those lines
  const bare = project({ "shop.chart.ts": gen(fixture("shop.scxml")), "usage.ts": usage.replaceAll(/\/\/ @ts-expect-error.*\n/g, "") });
  assert.equal(bare.ok, false);
  assert.equal((bare.out.match(/error TS/g) ?? []).length, 7, bare.out);
});

// @tg: verifies:: [[openspec:wasm-bindings#Generated TypeScript Types#Untyped chart falls back to dynamic values]] {level: integration}
test("untyped chart falls back to dynamic values", async () => {
  const dir = mkdtempSync(join(workspace, ".test-tmp", "untyped-"));
  const file = join(dir, "order.scxml");
  writeFileSync(file, ORDER);
  const ts = gen(file, "--name", "Order");
  assert.match(ts, /export type OrderEvent = \{ type: string; \[field: string\]: JsonValue \};/);
  assert.match(ts, /export type OrderContext = JsonValue;/);
  const r = project({
    "order.chart.ts": ts,
    "usage.ts": `import type { ActorSystem, JsonValue } from "@vertep/web";
import { createOrderActor } from "./order.chart";
declare const system: ActorSystem;
export async function use(): Promise<JsonValue> {
  const a = createOrderActor(system, "k");
  await a.send({ type: "anything", amount: 3, nested: { ok: true } });
  return (await a.getSnapshot()).context;
}
`,
  });
  assert.ok(r.ok, r.out);
  // and the chart still runs
  const system = await createActorSystem({ storage: "memory" });
  try {
    const a = await system.spawn(ORDER);
    await a.send({ type: "pay" });
    assert.deepEqual((await a.getSnapshot()).configuration, ["paid"]);
  } finally {
    await system.terminate();
  }
});

// ---- the TypeScript and JSON Schema projections accept the same values -----------------

/** A small JSON Schema 2020-12 validator for the keywords the projection uses. */
function valid(schema, v, root = schema) {
  if (schema.$ref) return valid(root.$defs[schema.$ref.replace("#/$defs/", "")], v, root);
  if (schema.anyOf && !schema.anyOf.some((s) => valid(s, v, root))) return false;
  if (schema.allOf && !schema.allOf.every((s) => valid(s, v, root))) return false;
  if ("const" in schema && v !== schema.const) return false;
  const t = schema.type;
  if (t === "null" && v !== null) return false;
  if (t === "boolean" && typeof v !== "boolean") return false;
  if (t === "string" && typeof v !== "string") return false;
  if (t === "number" && typeof v !== "number") return false;
  if (t === "integer" && !Number.isInteger(v)) return false;
  if (t === "array" && !Array.isArray(v)) return false;
  if (t === "object" && (v === null || typeof v !== "object" || Array.isArray(v))) return false;
  if (typeof v === "string" && schema.pattern && !new RegExp(schema.pattern, "u").test(v)) return false;
  if (typeof v === "number" && "minimum" in schema && v < schema.minimum) return false;
  if (typeof v === "number" && "exclusiveMaximum" in schema && v >= schema.exclusiveMaximum) return false;
  if (t === "array" && schema.items && !v.every((x) => valid(schema.items, x, root))) return false;
  if (t === "object") {
    for (const r of schema.required ?? []) if (!(r in v)) return false;
    for (const [k, x] of Object.entries(v)) {
      const p = schema.properties?.[k];
      if (!p) {
        if (schema.additionalProperties === false) return false;
      } else if (!valid(p, x, root)) return false;
    }
  }
  return true;
}

/** A seeded generator (mulberry32). */
function rng(seed) {
  return () => {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/**
 * Random values near the types: well-typed ones, then mutations (a field dropped, an extra
 * field, a wrong scalar, a wrong tag, null). Scalars respect the refinements TypeScript cannot
 * state (integers are integers, decimal strings and date-times match their patterns), which
 * are checked at runtime.
 */
function generator(rand) {
  const pick = (xs) => xs[Math.floor(rand() * xs.length)];
  const scalar = {
    string: () => pick(["", "a", "sku-1", "x y"]),
    int: () => pick([0, 1, -7, 42]),
    decimal: () => pick([0, 1.5, "12.50", "-3", 7]),
    datetime: () => pick(["2026-11-11T10:00:00Z", "2026-01-01T00:00:00+02:00"]),
    value: () => pick([null, 1, "s", [1, 2], { k: true }]),
  };
  // a value of another JSON kind (a string that breaks a pattern is a refinement, not structure)
  const wrong = () => pick([true, 3, null, [], {}]);
  const line = () => {
    const o = { sku: scalar.string(), qty: scalar.int() };
    if (rand() < 0.5) o.note = pick([null, "fragile"]);
    return o;
  };
  const payment = () => (rand() < 0.5 ? { tag: "Card", value: { last4: pick(["4242", "0000"]) } } : { tag: "Cash" });
  const order = () => {
    const o = { id: scalar.string(), lines: Array.from({ length: Math.floor(rand() * 3) }, line), total: scalar.decimal(), pay: payment() };
    if (rand() < 0.5) o.meta = scalar.value();
    if (rand() < 0.5) o.when = pick([null, scalar.datetime()]);
    return o;
  };
  const mutate = (v) => {
    if (v === null || typeof v !== "object") return wrong();
    const c = structuredClone(v);
    if (Array.isArray(c)) {
      if (c.length === 0) return [wrong()];
      c[0] = mutate(c[0]);
      return c;
    }
    const keys = Object.keys(c);
    const r = rand();
    if (r < 0.25 && keys.length) delete c[pick(keys)];
    else if (r < 0.45) c[pick(["extra", "zz"])] = 1;
    else if (r < 0.6 && "tag" in c) c.tag = "Cheque";
    else if (keys.length) {
      const k = pick(keys);
      c[k] = rand() < 0.5 ? mutate(c[k]) : wrong();
    }
    return c;
  };
  const gens = { Order: order, Payment: payment, Line: line };
  return (type) => (rand() < 0.4 ? gens[type]() : mutate(gens[type]()));
}

// @tg: verifies:: [[openspec:wasm-bindings#Generated TypeScript Types#Generated types match the other projections]] {level: integration}
test("TypeScript and JSON Schema projections accept the same values", () => {
  const schemas = JSON.parse(execFileSync(vt, ["schema", fixture("shop.scxml"), "--format", "jsonschema"], { encoding: "utf8" }));
  const order = schemas.events.submit;
  const payment = schemas.events.pay;
  const lineSchema = { ...order, $ref: "#/$defs/Line" };
  const schemaOf = { Order: order, Payment: payment, Line: lineSchema };
  const next = generator(rng(20261111));
  let lines = "";
  let accepted = 0;
  let rejected = 0;
  for (let i = 0; i < 300; i++) {
    const type = ["Order", "Payment", "Line"][i % 3];
    const v = next(type);
    const ok = valid(schemaOf[type], v);
    if (ok) accepted++;
    else rejected++;
    // TypeScript must reject exactly the values the schema rejects
    if (!ok) lines += "// @ts-expect-error\n";
    lines += `export const v${i}: ${type} = ${JSON.stringify(v)};\n`;
  }
  assert.ok(accepted > 50 && rejected > 50, `a balanced sample: ${accepted} accepted, ${rejected} rejected`);
  const r = project({
    "shop.chart.ts": gen(fixture("shop.scxml")),
    "values.ts": `import type { Line, Order, Payment } from "./shop.chart";\n${lines}`,
  });
  assert.ok(r.ok, r.out);
});
