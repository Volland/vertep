# Vertep examples

Small, runnable programs that show one idea each. Every Rust example is a binary of the
`vertep-examples` package, and `cargo test -p vertep-examples` runs them all and compares what
they print with the `expected-output.txt` next to them, so the outputs below are the real ones.

| Example | Shows | Capabilities |
|---|---|---|
| [traffic-light](#traffic-light) | An SCXML chart driven through the pure core interpreter; `vt validate`, `vt hash`, `vt convert` | scxml-interpretation, scxml-format, chart-rdf, yaml-format |
| [typed-order](#typed-order) | A typed YAML chart on CEL; `vt typecheck`, `vt schema`, `vt gen ts`, a type error | type-system, yaml-format, cel-datamodel |
| [rust-macro](#rust-macro) | A chart written with `statechart!`, guards and actions as Rust functions | rust-dsl |
| [durable-actor](#durable-actor) | A virtual actor on a tiramemsu journal: replay after restart, a durable timer | actor-model, event-journal, durable-timers |
| [decider-agent](#decider-agent) | `vt:decide` choice points, a scripted decider, replay with no decider | deciders |
| [browser-js](#browser-js) | `@vertep/web` from Node.js: spawn, send, subscribe, journal, time travel | wasm-bindings |

All commands run from the repository root.

## Setup

The Rust examples need nothing beyond the workspace toolchain. For the `vt` commands, install
the CLI from this checkout, or use cargo as an alias:

```sh
cargo install --path crates/vertep-cli        # puts `vt` on your PATH
# or
alias vt='cargo run -q -p vertep-cli --bin vt --'
```

`browser-js` needs the JavaScript packages built once (the wasm32 target and the `wasm-bindgen`
CLI at the version pinned in `crates/vertep-wasm/Cargo.toml`, 0.2.128 here):

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128
cd bindings/js && npm install && npm run build   # builds the WebAssembly modules and dist/
```

## traffic-light

`traffic-light/traffic-light.scxml` is a traffic light with a pedestrian crossing: a parallel
state whose `light` region cycles red, green and yellow on delayed `<send>`s, and whose
`crossing` region follows it with eventless `In('red')` transitions. `traffic-light/main.rs` is
the smallest host for the pure core: `session::start` and `session::step` return the next
session plus effects ("deliver `tick` in 30 s", "cancel `light-timer`"), and the program keeps
the timers on a virtual clock and feeds them back as events. A pedestrian `button` cuts green
short, which cancels the pending timer; `fault` leaves the parallel state entirely.

```sh
cargo run -p vertep-examples --bin traffic-light
```

```text
[   0s] (start)  -> operating light red crossing walk
          schedule tick in 30s
[  30s] tick     -> operating light green crossing dont_walk
          schedule tick in 25s
[  40s] button   -> operating light yellow crossing dont_walk
          cancel light-timer
          schedule tick in 5s
[  45s] tick     -> operating light red crossing walk
          schedule tick in 30s
[  50s] fault    -> flashing
          cancel light-timer
[  60s] reset    -> operating light red crossing walk
          schedule tick in 30s
...
[ 120s] tick     -> operating light red crossing walk
          schedule tick in 30s
stopped at 120s; still pending: tick due at 150s
```

The same chart through `vt`. Every surface of a chart has the same content-addressed version
IRI, so the converted YAML and Turtle hash to the same value as the SCXML:

```sh
vt validate examples/traffic-light/traffic-light.scxml
vt hash examples/traffic-light/traffic-light.scxml
vt convert --to yaml examples/traffic-light/traffic-light.scxml
vt convert --to turtle examples/traffic-light/traffic-light.scxml
```

```text
examples/traffic-light/traffic-light.scxml: ok: urn:vertep:chart:sha256:46431c540d155bee9016c61ee7a8f734890a92342ed9f91188aea25a602f5731
urn:vertep:chart:sha256:46431c540d155bee9016c61ee7a8f734890a92342ed9f91188aea25a602f5731
```

The YAML (`traffic-light/vt/traffic-light.yaml`, excerpt):

```yaml
vertep: 1
name: traffic-light
datamodel: "null"
initial: operating
states:
  operating:
    type: parallel
    on:
      fault: flashing
    states:
      light:
        initial: red
        states:
          red:
            entry:
              - send:
                  event: tick
                  id: light-timer
                  delay: "30s"
            exit:
              - cancel:
                  sendid: light-timer
            on:
              tick: green
```

The Turtle (`traffic-light/vt/traffic-light.ttl`, 272 lines, excerpt): every state,
transition and target is a node under the version IRI.

```turtle
@prefix c: <urn:vertep:chart:sha256:46431c540d155bee9016c61ee7a8f734890a92342ed9f91188aea25a602f5731#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix vt: <https://w3id.org/vertep/vt#> .
c:chart a vt:Chart ;
	vt:binding "early" ;
	vt:bindingExplicit false ;
	vt:child c:s.flashing , c:s.operating ;
	vt:datamodel "null" ;
	vt:datamodelExplicit true ;
	vt:initial c:chart:ia0 ;
	vt:name "traffic-light" .
c:s.dont_walk:t0 a vt:Transition ;
	vt:cond "In('red')"^^vt:nullExpr ;
	vt:order 0 ;
	vt:source c:s.dont_walk ;
	vt:target c:s.dont_walk:t0:tg0 ;
	vt:transitionType "external" ;
	vt:transitionTypeExplicit false .
```

## typed-order

`typed-order/order.yaml` is an order chart in YAML on the CEL datamodel with a `types` block
(records, a union, `optional` and `list`), a payload type for every event and a type for every
context field. `vt typecheck` checks every guard and assignment against those declarations;
`vt schema` projects the types to JSON Schema, SHACL or the CEL type environment; `vt gen ts`
writes a TypeScript module for `@vertep/web`.

```sh
vt typecheck examples/typed-order/order.yaml
vt schema --format cel examples/typed-order/order.yaml
vt schema --format jsonschema examples/typed-order/order.yaml   # typed-order/vt/schema.json
vt schema --format shacl examples/typed-order/order.yaml        # typed-order/vt/schema-shacl.ttl
vt gen ts examples/typed-order/order.yaml                       # typed-order/vt/order.ts
```

```text
examples/typed-order/order.yaml: ok: 3 named types, 6 typed events, 3 context fields
ctx.items: list(Item)
ctx.total: decimal
ctx.limit: decimal
ev.data[add]: Item
ev.data[checkout]: null_type
ev.data[pay]: Pay
ev.data[approve]: null_type
ev.data[ship]: Ship
ev.data[cancel]: null_type
```

The generated TypeScript (excerpt; the full module also embeds the chart source):

```ts
export type Item = { sku: string; qty: number; price: number | string };
export type Pay = { amount: number | string; method: { tag: "Card"; value: { last4: string } } | { tag: "Invoice" } };
export type Ship = { carrier: string; tracking?: string | null };

export type OrderEvent =
  | { type: "add" } & Item
  | { type: "checkout" }
  | { type: "pay" } & Pay
  | { type: "approve" }
  | { type: "ship" } & Ship
  | { type: "cancel" };
export interface OrderContext {
  items: Item[];
  total: number | string;
  limit: number | string;
}
export type OrderStateId = "shopping" | "cart" | "payment" | "review" | "fulfilment" | "shipped" | "cancelled";
```

`typed-order/order-ill-typed.yaml` assigns the item's `sku` (a string) to `ctx.total` (a
decimal). The error names the file, line and column, the path in the document and the state:

```sh
vt typecheck examples/typed-order/order-ill-typed.yaml   # exit code 1
```

```text
examples/typed-order/order-ill-typed.yaml:44:19: states.shopping.states.cart.on.add[0].actions[1]: state "cart", transition 0 (on add), action 1: the value assigned to `ctx.total` has type string, expected decimal
```

## rust-macro

`rust-macro/main.rs` writes a coin-operated turnstile with `statechart!` on the `rust`
datamodel. The macro checks the chart at compile time and generates `Ctx`, `Event`, `State`,
`VERSION` and `Machine`; guards (`fn(&Ctx, &P) -> bool`) and actions
(`fn(&mut Ctx, &P, &mut Emit)` or `fn(&mut Ctx, &P)`) are ordinary functions bound by name. A
targetless `on coin / add_credit` counts coins and an eventless `always if paid => unlocked`
opens the turnstile once there is enough credit.

```sh
cargo run -p vertep-examples --bin rust-macro
```

```text
chart version urn:vertep:chart:sha256:0b1614ef361262f9f6fec5189fa5834756f37808a3a466f81a9f83d6ed20e868
(start)                -> [Service, Locked]    credit=0   passes=0
Coin { cents: 20 }     -> [Service, Locked]    credit=20  passes=0
Push                   -> [Service, Locked]    credit=20  passes=0
Coin { cents: 50 }     -> [Service, Unlocked]  credit=70  passes=0
Push                   -> [Service, Locked]    credit=20  passes=1
Push                   -> [Service, Locked]    credit=20  passes=1
Shutdown               -> []                   credit=20  passes=1
```

`off` is a top-level final state, so `Shutdown` ends the session and the configuration is empty.

## durable-actor

`durable-actor/order.scxml` is an order on CEL whose `awaiting_payment` state schedules
`payment.timeout` 900 s ahead. `durable-actor/main.rs` sends three events to the virtual actor
`https://example.org/charts/order/42` on a tiramemsu store, then drops the node and the store as
if the process stopped. A second node opens the same directory ten minutes later: reading the
actor replays its journal, and five minutes after that the timer the first node scheduled fires
as an ordinary journaled input. Both nodes run on a virtual clock, so the output is stable.

```sh
cargo run -p vertep-examples --bin durable-actor
```

```text
actor https://example.org/charts/order/42
process 1: ["awaiting_payment"] context={"items":2}
  entry 1 at +0s: add              timers scheduled: []
  entry 2 at +0s: add              timers scheduled: []
  entry 3 at +0s: checkout         timers scheduled: ["payment.timeout"]
  pending timer payment.timeout due at +900s
process 1 stops
process 2: ["awaiting_payment"] context={"items":2} (replayed 3 entries)
process 2 at +900s: ["expired"] context={"items":2}
  entry 1 at +0s: add              timers scheduled: []
  entry 2 at +0s: add              timers scheduled: []
  entry 3 at +0s: checkout         timers scheduled: ["payment.timeout"]
  entry 4 at +900s: payment.timeout  timers scheduled: []
as of entry 1: ["cart"] context={"items":1}
```

A server runs the same node on the system clock with `Shell::tokio()` and `node.run().await`;
`crates/vertep/examples/quickstart.rs` shows that, plus a time-travel query in tiramemsu.

## decider-agent

`decider-agent/refund.scxml` has two decide states: `vt:decide="policy"` hands the choice
among their events to whatever decider the host binds to `policy`. The chart still owns the
rules: with an amount of 250, the guard `ctx.amount <= 100` keeps `approve` out of the offer in
`review`. `decider-agent/main.rs` binds a scripted `Decider` (a stand-in for an `LlmDecider`,
so no network and no API key), runs a request, and prints the journal: each decision is an entry
with its offer's correlation id and rationale. A second `AgentHost` on the same journal with no
decider bound replays it to the same states without asking anyone.

```sh
cargo run -p vertep-examples --bin decider-agent
```

```text
run 1, decider bound to `policy`:
  decider asked in `review`: offered ["reject", "escalate"] -> escalate
  decider asked in `manager`: offered ["approve", "reject"] -> approve
  final: ["approved"] context={"amount":250}
journal:
  1 start
  2 event     request
  3 decision  escalate (offer https://example.org/agents/refund/7/decision/2.0, rationale "amount is above my approval limit")
  4 decision  approve (offer https://example.org/agents/refund/7/decision/3.0, rationale "manager approved: item returned unopened")
run 2, same journal, no decider bound:
  final: ["approved"] context={"amount":250}
  as of entry 2: ["review"]
  as of entry 3: ["manager"]
  as of entry 4: ["approved"]
  entries replayed: 4, decider calls: 0
```

For an LLM-driven agent with tools and a fork from the journal, see
`crates/vertep-agent/examples/react_support_agent.rs` and `fork_from_journal.rs`.

## browser-js

`browser-js/main.mjs` is an ES module that uses `@vertep/web`, the same package a browser
loads: the actor system runs in a worker (a worker thread in Node.js) with the core compiled to
WebAssembly. It spawns a cart actor from SCXML text with in-memory storage, subscribes to
snapshots, sends events (`{ type, ...payload }`), reads the journal with the transitions each
entry took, and reads the state as of an earlier entry. An event no transition takes (`pay`) is
still journaled, but produces no snapshot. `package.json` links `@vertep/web` to the local build
in `bindings/js/packages/web`.

```sh
cd examples/browser-js
npm install     # links the local @vertep/web; no download
npm start       # node main.mjs; `npm test` compares with expected-output.txt
```

```text
actor https://example.org/charts/cart/alice
snapshot @1: ["filled"] items=2
snapshot @2: ["filled"] items=3
snapshot @4: ["ordered"] items=3
now: ["ordered"] matches('ordered') = true
journal:
  1 add      {"qty":2} -> ["s.empty:t0"]
  2 add      {"qty":1} -> ["s.filled:t0"]
  3 pay      null      -> []
  4 checkout null      -> ["s.filled:t1"]
as of entry 1: ["filled"] items = 2
```

In a browser, pass `storage: "opfs"` for a durable journal in the origin private file system.

## Keeping the examples honest

- `tests/run_examples.rs` runs each binary and compares its output with `expected-output.txt`;
  it also runs `browser-js` when Node.js and `examples/browser-js/node_modules` are present,
  and runs the typed-order chart through the core on a typed CEL datamodel.
- `tests/vt_commands.rs` runs the `vt` commands above through the `vertep-cli` library and
  compares them with the files under `traffic-light/vt/` and `typed-order/vt/`.
- After an intended change, re-record with `VERTEP_BLESS=1 cargo test -p vertep-examples` and
  review the diff.
