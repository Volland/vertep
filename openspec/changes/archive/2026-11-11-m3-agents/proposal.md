## Why

Vertep's main reason to put agents inside statecharts is that the chart becomes the safety boundary. An agent's action space is the set of enabled transitions, every model output is a journaled event, and a run can be replayed, audited and forked. M2 provides journaled actors, but nothing yet lets an LLM, a human or another actor choose what happens next. M3 adds that choice point, LLM and tool actors, and LLM-written plans, without giving up determinism.

## What Changes

- **`vt:decide` states.** A new kind of state that, on entry, computes the currently enabled events and hands them to a pluggable decider. The decision comes back as a journaled event, and only an enabled event is accepted.
- **Pluggable deciders.** A `Decider` contract with four kinds: deterministic guards, an LLM, a human in the loop, and another actor. It includes deadlines and fallback deciders, re-deciding after the context changes, and typed payload schemas generated from the type system.
- **Decision annotations.** Rationale, confidence, model and decider identity are recorded as RDF 1.2 triple-term annotations on the journaled decision.
- **LLM actors.** `<invoke type="vt:llm">` with prompt templates rendered from the context, a provider contract and structured output. Each result is journaled as an `llm.result` input, so replay never calls the model again. Token and cost accounting goes into the agent ontology, retries are expressed in the chart, and streamed partial results are an optional feature.
- **Tool actors.** Tools are invoked actors whose argument and result schemas come from chart types.
- **Capability grants.** An invoking state grants send targets to LLM, tool and plan actors. A send to a target that wasn't granted fails with `error.communication`.
- **LLM as main driver.** A ReAct-style agent loop is written as an ordinary chart pattern: one `vt:decide` loop with a wide action space.
- **The `vt-agent:` ontology module.** Deciders, decisions, LLM calls, tools, grants and plans, aligned with PROV-O agents and activities, with SHACL shapes.
- **Plan charts.** Chart fragments written by an LLM are validated with SHACL and the IR checks, restricted to the CEL datamodel, content-addressed, and run as sandboxed invoked children. They get granted capabilities and step and resource bounds, and their plan lineage is recorded in the graph. An agent can change its own chart only through a new chart version plus a migration.

## Capabilities

### New Capabilities

- `deciders`: `vt:decide` state semantics, the decider contract and kinds, enabled-event handling, deadlines and fallbacks, typed decision schemas, and journaled, annotated decisions.
- `llm-actors`: LLM invocation as an actor, prompt templates, providers, structured output, journaled results, accounting, tool actors, capability grants, the ReAct agent pattern, and the `vt-agent:` ontology module.
- `plan-charts`: LLM-produced chart fragments, their validation, the CEL-only restriction, content addressing, sandboxed execution as invoked children, bounds, result return, lineage, and self-modification through versioning.

### Modified Capabilities

(none)

## Impact

- **Code:** new crate `vertep-agent` ([[components#vertep-agent]]), which uses `vertep-runtime`, `vertep-dm-cel` and `vertep-rdf`. The `vt-agent:` ontology module and its SHACL shapes ship with `vertep-rdf`. The Lean package `vertep-lean` gains theorems for decider safety and grant enforcement ([[components#vertep-lean]]).
- **tg sections touched:** [[capabilities#Deciders]], [[capabilities#LLM actors]], [[capabilities#Plan charts]], [[roadmap#M3 Agents]], [[glossary#Decider]].
- **Binding decisions:**
  - [[decisions#Deciders at choice points]]: the primary decision this change realizes.
  - [[decisions#Event-sourced actors]]: model results are journaled inputs.
  - [[decisions#Pluggable datamodel]]: plans use CEL only.
  - [[decisions#One IR type language]]: decision and tool schemas.
  - [[decisions#Content-addressed chart versions]]: plan and self-modification versions.
  - [[decisions#Modular vt ontology on RDF 1.2]]: the `vt-agent:` module and triple-term annotations.
  - [[decisions#Lean verification bridge]]: decider safety theorem.
  - [[decisions#Non-blocking request and deferred events]]: deciders never block the chart.
- **Depends on capabilities from earlier milestones:** `actor-model`, `messaging`, `event-journal`, `chart-versioning`, `runtime-ontology` (M2), `type-system` and `cel-datamodel` (M1), and `chart-model`, `scxml-interpretation` and `chart-rdf` (M0).
- **External dependencies:** an HTTP client for model providers, behind a provider trait and cargo features. No provider is a hard dependency.
