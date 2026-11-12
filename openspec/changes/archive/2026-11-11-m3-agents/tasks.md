## 1. Crate and ontology setup

- [x] 1.1 Create the `vertep-agent` crate in the workspace, with dependencies on `vertep-runtime`, `vertep-dm-cel` and `vertep-rdf`, and a cargo feature per optional provider
- [x] 1.2 Add the `agent` feature to the `vertep` facade that re-exports `vertep-agent`
- [x] 1.3 Write the `vt-agent:` ontology module (Decider, Decision, LlmCall, Tool, ToolCall, Grant, Plan and their properties) in `vertep-rdf`, with PROV-O alignment axioms as plain triples
- [x] 1.4 Write the SHACL shapes for `vt-agent:` and a test that a plain chart's RDF contains no `vt-agent:` terms

## 2. Decide states in the core

- [x] 2.1 Add the decide-state binding (decider, deadline, fallback, candidate descriptors, maxDecisions) to the IR, and validation errors for a missing binding (as `vt:` extension attributes on the state, not a new IR field: [[decisions#Decide bindings ride on vt extension attributes]])
- [x] 2.2 Implement the pure `enabled_events` function over configuration and datamodel, including ancestor transitions, in `vertep-core` (pure, in `vertep-agent::enabled` over the core's `select_transitions`, not in `vertep-core`: same decision)
- [x] 2.3 Emit the `OfferDecision` effect on decide-state entry, with a correlation id derived from the journal position, and `vt.decide.none` for an empty candidate set
- [x] 2.4 Implement decision acceptance: candidate membership, payload schema validation, staleness, and `error.decide.invalid` with reasons
- [x] 2.5 Implement candidate-set fingerprints and the cancel-and-reoffer effect after macrosteps that change them
- [x] 2.6 Cancel outstanding offers when the decide state exits
- [x] 2.7 Parse and serialize the `vt:decide` extension in the SCXML and YAML surfaces, and project it to RDF as `vt:DecideState` (both surfaces keep `vt:` extensions losslessly; `decide::project` adds `vt:DecideState`)

## 3. Decider contract and kinds

- [x] 3.1 Define the `Decider` trait, `DecisionRequest`, `Decision`, `Refusal` and the immutable context view
- [x] 3.2 Implement the decider registry and the instantiation error for unknown bindings
- [x] 3.3 Implement the guard decider (first candidate in document order whose decision guard holds)
- [x] 3.4 Implement the actor decider over `vt:request`
- [x] 3.5 Implement the human decider with an inbox actor and the reply protocol
- [x] 3.6 Implement the LLM decider, with a `oneOf` structured-output schema over the candidates
- [x] 3.7 Implement deadlines as durable timers, fallback consultation, `error.decide.timeout`, and staleness of late answers
- [x] 3.8 Derive candidate JSON Schemas from the type system, with the dynamic value schema for untyped charts
- [x] 3.9 Record decision annotations (rationale, confidence, decider kind and identity, model, prompt hash) as RDF 1.2 triple-term annotations, and reject confidences out of range

## 4. LLM actors

- [x] 4.1 Define the provider contract (prompt, model, parameters, output schema → result with usage, or a classified error)
- [x] 4.2 Implement the `vt:llm` invoke type as an invoked actor, with cancellation on state exit
- [x] 4.3 Implement deterministic template rendering against the context view, with prompt hashing and `template` errors
- [x] 4.4 Implement structured-output validation and `error.llm` with the class `schema`
- [x] 4.5 Journal `llm.result` before delivery, and teach the shell to replay journaled results without starting provider calls
- [x] 4.6 Implement the `vt:retry` extension as a bounded retry substate with durable backoff timers (lowered to journaled retry timers in the agent machine rather than a generated substate: [[decisions#vt:retry lowers to journaled retry timers]])
- [x] 4.7 Implement opt-in streaming with `vt.llm.partial` events, and validation that rejects targeted transitions on partials
- [x] 4.8 Record usage and cost per call as `vt-agent:LlmCall`, and expose cumulative usage as a read-only system variable
- [x] 4.9 Provide one reference provider behind a cargo feature, and a deterministic mock provider for tests (`anthropic` feature: Messages API mapping over an application-supplied `HttpClient`; `ScriptedProvider` is the mock)

## 5. Tools and grants

- [x] 5.1 Implement tool registration (IRI, argument type, result type) and JSON Schema derivation
- [x] 5.2 Validate tool arguments before execution, with `error.tool` and the class `arguments`, and journal tool results in the caller's journal
- [x] 5.3 Implement `vt:grant` declarations on invoking states, and effective grants as the intersection with the parent's grants
- [x] 5.4 Enforce grants in the runtime send path, with `error.communication` and the reason `not-granted`, and journal the denied attempts (in the send path of invoked runtime actors via the `GrantedActor` factory wrapper, and of plan children in the agent machine)
- [x] 5.5 Offer exactly the granted tool schemas to LLM invocations

## 6. Plan charts

- [x] 6.1 Implement plan submission from `vt.plan.proposed`, YAML and RDF parsing, and `error.plan` with the class `parse`
- [x] 6.2 Implement plan validation: IR checks, SHACL, the CEL-or-null datamodel only, no scripts, granted functions only, and CEL type checking
- [x] 6.3 Content-address accepted plans with the chart hashing procedure
- [x] 6.4 Run accepted plans as invoked children with intersected grants, cancelled on state exit, never as virtual actors
- [x] 6.5 Enforce plan bounds (macrosteps, durable duration timer, sends) with `error.plan` and the class `bounds`
- [x] 6.6 Validate typed donedata against the plan's result type, and return `done.invoke` or `error.plan` with the class `result`
- [x] 6.7 Record plan lineage, including rejected plans and their violations, using `vt-agent:Plan` and PROV-O-aligned properties
- [x] 6.8 Route self-modification proposals (a new own version plus a migration) through validation and the `chart-versioning` migration procedure, and journal the acceptance or rejection

## 7. Agent pattern and examples

- [x] 7.1 Implement `vt:maxDecisions` and `vt.agent.budget-exhausted`
- [x] 7.2 Add a ReAct support-agent example chart, with the mock provider and two tools, run end to end
- [x] 7.3 Add an example that forks an agent run from a journal position and injects a different decision

## 8. Formal model (Lean)

- [x] 8.1 Model `enabled_events`, decide-state offers, correlation ids and decision acceptance in `vertep-lean`
- [x] 8.2 Prove decider safety: for every decider output, any transition taken because of a decision was enabled at offer time
- [x] 8.3 Prove grant enforcement: a send from an invoked actor is delivered only if its target is in the intersected grants
- [x] 8.4 Prove that replay with journaled decisions and `llm.result` inputs reaches the same state without consulting deciders
- [x] 8.5 Extend the differential oracle generators with decide states and adversarial decider outputs (unknown events, bad payloads, stale ids), and run them in CI against the Rust core
  - Done (2026-11-11): `crates/vertep-conformance/tests/decide_differential.rs` generates charts with a `vt:decide` state (mostly one the start enters, with two decider transitions that re-enter it) and a `vt:types` block declaring payload types (records, `optional`, lists, unions, a named type, no payload) on some event names, then adversarial decisions (stale, forged and redelivered correlation ids, unknown and unoffered events, payloads of and not of the declared type) for `AgentMachine`. `vertep-decide` replays every decision outcome through the Lean `Agent.decide` and now decides payload conformance itself (`conformsJson` in `DecideCheck.lean`, a port of the agent's payload validator, from the schema and payload each line carries); the agent's core macrosteps, recorded with their answer tape (`AgentMachine::start_with` with `StartOptions { record_core: true, .. }`, `core_tape()`), replay through `vertep-oracle` with equal traces. Smoke sample of 32 cases with the gate, 256 cases under `--ignored differential`; all agree.

## 9. Traceability and docs

- [x] 9.1 Annotate the code implementing each requirement with `@tg: implements:: [[openspec:deciders#<Requirement>]]`, `[[openspec:llm-actors#<Requirement>]]` or `[[openspec:plan-charts#<Requirement>]]` above the function, type or file, with `{confidence: partial}` where it applies
- [x] 9.2 Annotate each test that proves a scenario with `@tg: verifies:: [[openspec:<capability>#<Requirement>#<Scenario>]]` directly above the test call
- [x] 9.3 Record any new design decisions and constraints from implementation in `tg.md/decisions.md` and `tg.md/constraints.md` as typed sections with `motivated_by` and `constrains` edges
- [x] 9.4 DONE AS FRONTMATTER (tg 0.22 does not resolve doc links to openspec requirements; capabilities.md lists deciders, llm-actors and plan-charts in its openspec frontmatter, components#vertep-agent provides the three capabilities, and code carries @tg annotations), was: Update the sections [[capabilities#Deciders]], [[capabilities#LLM actors]] and [[capabilities#Plan charts]] with `specified_by` edges to their requirements, and confirm the capabilities are listed in that file's `openspec:` frontmatter
- [x] 9.5 DONE AS FRONTMATTER (tg 0.22 does not resolve doc links to openspec requirements; capabilities.md lists deciders, llm-actors and plan-charts in its openspec frontmatter, components#vertep-agent provides the three capabilities, and code carries @tg annotations), was: Update [[components#vertep-agent]] with `provides` edges to the implemented requirements
- [x] 9.6 Run `tg check`, `tg ontology check` and `tg trace deciders llm-actors plan-charts --gaps`, and close every gap they report
