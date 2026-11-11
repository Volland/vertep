## 1. Crate and ontology setup

- [ ] 1.1 Create the `vertep-agent` crate in the workspace, with dependencies on `vertep-runtime`, `vertep-dm-cel` and `vertep-rdf`, and a cargo feature per optional provider
- [ ] 1.2 Add the `agent` feature to the `vertep` facade that re-exports `vertep-agent`
- [ ] 1.3 Write the `vt-agent:` ontology module (Decider, Decision, LlmCall, Tool, ToolCall, Grant, Plan and their properties) in `vertep-rdf`, with PROV-O alignment axioms as plain triples
- [ ] 1.4 Write the SHACL shapes for `vt-agent:` and a test that a plain chart's RDF contains no `vt-agent:` terms

## 2. Decide states in the core

- [ ] 2.1 Add the decide-state binding (decider, deadline, fallback, candidate descriptors, maxDecisions) to the IR, and validation errors for a missing binding
- [ ] 2.2 Implement the pure `enabled_events` function over configuration and datamodel, including ancestor transitions, in `vertep-core`
- [ ] 2.3 Emit the `OfferDecision` effect on decide-state entry, with a correlation id derived from the journal position, and `vt.decide.none` for an empty candidate set
- [ ] 2.4 Implement decision acceptance: candidate membership, payload schema validation, staleness, and `error.decide.invalid` with reasons
- [ ] 2.5 Implement candidate-set fingerprints and the cancel-and-reoffer effect after macrosteps that change them
- [ ] 2.6 Cancel outstanding offers when the decide state exits
- [ ] 2.7 Parse and serialize the `vt:decide` extension in the SCXML and YAML surfaces, and project it to RDF as `vt:DecideState`

## 3. Decider contract and kinds

- [ ] 3.1 Define the `Decider` trait, `DecisionRequest`, `Decision`, `Refusal` and the immutable context view
- [ ] 3.2 Implement the decider registry and the instantiation error for unknown bindings
- [ ] 3.3 Implement the guard decider (first candidate in document order whose decision guard holds)
- [ ] 3.4 Implement the actor decider over `vt:request`
- [ ] 3.5 Implement the human decider with an inbox actor and the reply protocol
- [ ] 3.6 Implement the LLM decider, with a `oneOf` structured-output schema over the candidates
- [ ] 3.7 Implement deadlines as durable timers, fallback consultation, `error.decide.timeout`, and staleness of late answers
- [ ] 3.8 Derive candidate JSON Schemas from the type system, with the dynamic value schema for untyped charts
- [ ] 3.9 Record decision annotations (rationale, confidence, decider kind and identity, model, prompt hash) as RDF 1.2 triple-term annotations, and reject confidences out of range

## 4. LLM actors

- [ ] 4.1 Define the provider contract (prompt, model, parameters, output schema → result with usage, or a classified error)
- [ ] 4.2 Implement the `vt:llm` invoke type as an invoked actor, with cancellation on state exit
- [ ] 4.3 Implement deterministic template rendering against the context view, with prompt hashing and `template` errors
- [ ] 4.4 Implement structured-output validation and `error.llm` with the class `schema`
- [ ] 4.5 Journal `llm.result` before delivery, and teach the shell to replay journaled results without starting provider calls
- [ ] 4.6 Implement the `vt:retry` extension as a bounded retry substate with durable backoff timers
- [ ] 4.7 Implement opt-in streaming with `vt.llm.partial` events, and validation that rejects targeted transitions on partials
- [ ] 4.8 Record usage and cost per call as `vt-agent:LlmCall`, and expose cumulative usage as a read-only system variable
- [ ] 4.9 Provide one reference provider behind a cargo feature, and a deterministic mock provider for tests

## 5. Tools and grants

- [ ] 5.1 Implement tool registration (IRI, argument type, result type) and JSON Schema derivation
- [ ] 5.2 Validate tool arguments before execution, with `error.tool` and the class `arguments`, and journal tool results in the caller's journal
- [ ] 5.3 Implement `vt:grant` declarations on invoking states, and effective grants as the intersection with the parent's grants
- [ ] 5.4 Enforce grants in the runtime send path, with `error.communication` and the reason `not-granted`, and journal the denied attempts
- [ ] 5.5 Offer exactly the granted tool schemas to LLM invocations

## 6. Plan charts

- [ ] 6.1 Implement plan submission from `vt.plan.proposed`, YAML and RDF parsing, and `error.plan` with the class `parse`
- [ ] 6.2 Implement plan validation: IR checks, SHACL, the CEL-or-null datamodel only, no scripts, granted functions only, and CEL type checking
- [ ] 6.3 Content-address accepted plans with the chart hashing procedure
- [ ] 6.4 Run accepted plans as invoked children with intersected grants, cancelled on state exit, never as virtual actors
- [ ] 6.5 Enforce plan bounds (macrosteps, durable duration timer, sends) with `error.plan` and the class `bounds`
- [ ] 6.6 Validate typed donedata against the plan's result type, and return `done.invoke` or `error.plan` with the class `result`
- [ ] 6.7 Record plan lineage, including rejected plans and their violations, using `vt-agent:Plan` and PROV-O-aligned properties
- [ ] 6.8 Route self-modification proposals (a new own version plus a migration) through validation and the `chart-versioning` migration procedure, and journal the acceptance or rejection

## 7. Agent pattern and examples

- [ ] 7.1 Implement `vt:maxDecisions` and `vt.agent.budget-exhausted`
- [ ] 7.2 Add a ReAct support-agent example chart, with the mock provider and two tools, run end to end
- [ ] 7.3 Add an example that forks an agent run from a journal position and injects a different decision

## 8. Formal model (Lean)

- [ ] 8.1 Model `enabled_events`, decide-state offers, correlation ids and decision acceptance in `vertep-lean`
- [ ] 8.2 Prove decider safety: for every decider output, any transition taken because of a decision was enabled at offer time
- [ ] 8.3 Prove grant enforcement: a send from an invoked actor is delivered only if its target is in the intersected grants
- [ ] 8.4 Prove that replay with journaled decisions and `llm.result` inputs reaches the same state without consulting deciders
- [ ] 8.5 Extend the differential oracle generators with decide states and adversarial decider outputs (unknown events, bad payloads, stale ids), and run them in CI against the Rust core

## 9. Traceability and docs

- [ ] 9.1 Annotate the code implementing each requirement with `@tg: implements:: [[openspec:deciders#<Requirement>]]`, `[[openspec:llm-actors#<Requirement>]]` or `[[openspec:plan-charts#<Requirement>]]` above the function, type or file, with `{confidence: partial}` where it applies
- [ ] 9.2 Annotate each test that proves a scenario with `@tg: verifies:: [[openspec:<capability>#<Requirement>#<Scenario>]]` directly above the test call
- [ ] 9.3 Record any new design decisions and constraints from implementation in `tg.md/decisions.md` and `tg.md/constraints.md` as typed sections with `motivated_by` and `constrains` edges
- [ ] 9.4 Update the sections [[capabilities#Deciders]], [[capabilities#LLM actors]] and [[capabilities#Plan charts]] with `specified_by` edges to their requirements, and confirm the capabilities are listed in that file's `openspec:` frontmatter
- [ ] 9.5 Update [[components#vertep-agent]] with `provides` edges to the implemented requirements
- [ ] 9.6 Run `tg check`, `tg ontology check` and `tg trace deciders llm-actors plan-charts --gaps`, and close every gap they report
