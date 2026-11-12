## Purpose

LLM actors bring language models and tools into a chart as ordinary invoked actors, with typed prompts and structured results. Every result is journaled so runs replay without calling a model, tool use is bounded by capability grants, and calls, costs and lineage are recorded in the `vt-agent:` ontology.

## ADDED Requirements

### Requirement: LLM Invocation
The system SHALL support `<invoke type="vt:llm">`, which starts an LLM call as an invoked actor of the invoking state. The invocation names a provider binding, a model identifier, a prompt template, an optional output type and optional generation parameters. The call SHALL be cancelled when the invoking state exits, following SCXML invoke semantics.

#### Scenario: Successful LLM call
- **WHEN** a state with `<invoke type="vt:llm" id="summarise">` is entered and the provider returns a completion
- **THEN** the actor receives `done.invoke.summarise`, carrying the result as event data, and the transitions on it are taken by ordinary SCXML selection

#### Scenario: Call cancelled on state exit
- **WHEN** the invoking state is exited while the LLM call is in flight
- **THEN** the call is cancelled, no `done.invoke` event for it is processed, and any result that arrives later is dropped as belonging to a cancelled invocation

#### Scenario: Unknown provider binding
- **WHEN** an LLM invocation names a provider binding that is not registered
- **THEN** the event `error.execution` is raised in the invoking state with the reason `unknown-provider`, and no call is made

### Requirement: Prompt Templates
The system SHALL build the prompt by rendering the declared template against a read-only view of the context and the triggering event. The rendered prompt SHALL be recorded by content hash in the journal entry of the call. Rendering SHALL be deterministic: the same template, context and event SHALL produce the same prompt bytes.

#### Scenario: Template rendered from context
- **WHEN** a template references `context.customer.name` and the context holds the name "Ada"
- **THEN** the rendered prompt contains "Ada", and the journal entry records the SHA-256 hash of the rendered prompt

#### Scenario: Missing template variable
- **WHEN** a template references a context path that does not exist
- **THEN** rendering fails, `error.execution` is raised with the reason `template` and the missing path, and no model call is made

#### Scenario: Deterministic rendering
- **WHEN** the same template is rendered twice with equal context and event
- **THEN** both renderings have the same content hash

### Requirement: Provider Contract
The system SHALL define a provider contract that accepts a rendered prompt, a model identifier, generation parameters and an optional output schema, and returns a result or an error. A result contains the content, usage counts and the provider's model identifier. Providers SHALL be pluggable, and no specific vendor SHALL be required to build or run Vertep.

#### Scenario: Two providers interchangeable
- **WHEN** the same chart is run once with provider binding `a` and once with provider binding `b`, both returning valid structured output
- **THEN** the chart behaves the same given the same results, and only the journaled results and accounting differ

#### Scenario: Provider error surfaces as event
- **WHEN** a provider returns a rate-limit error
- **THEN** the event `error.llm` is raised to the invoking state, with the error class `rate-limited` and a retry-after hint when the provider gave one

### Requirement: Structured Output
When an LLM invocation declares an output type, the system SHALL request structured output conforming to that type's JSON Schema, validate the returned content against it, and deliver the parsed value as the event payload. A result that fails validation SHALL raise `error.llm` with the class `schema` and SHALL NOT be delivered as `done.invoke`.

#### Scenario: Valid structured output
- **WHEN** the output type is the record `{ category: enum(billing, tech, other), urgency: int }` and the model returns `{"category":"tech","urgency":2}`
- **THEN** `done.invoke` carries the typed value, and guards can read `category` and `urgency` from it

#### Scenario: Invalid structured output
- **WHEN** the model returns `{"category":"sales"}`
- **THEN** `error.llm` is raised with the class `schema` and the violated paths, and the invalid content is journaled for audit

### Requirement: Journaled Model Results
Every model result, structured or not, SHALL be appended to the invoking actor's journal as an `llm.result` input before it is processed. The input carries the invoke id, the prompt hash, the content, usage and the model identifier. Replay SHALL use journaled results and SHALL NOT call any provider.

#### Scenario: Result journaled before delivery
- **WHEN** a provider returns a result for invoke id `summarise`
- **THEN** the journal contains an `llm.result` entry for `summarise` before the `done.invoke.summarise` transition is executed

#### Scenario: Replay without provider
- **WHEN** an agent run is replayed from its journal in an environment with no provider bindings registered
- **THEN** replay reaches the same configuration and context as the original run

#### Scenario: Forked run reuses history
- **WHEN** a run is forked from journal position N and a different decision is injected at N+1
- **THEN** all model results up to N come from the journal, and only calls after the fork reach a provider

### Requirement: Retries Expressed in the Chart
The system SHALL NOT retry a failed or invalid model call implicitly. Retries, backoff and alternative models SHALL be expressed as chart transitions on `error.llm`, optionally with the `vt:retry` invoke extension. That extension expands to a bounded retry substate with a durable backoff timer.

#### Scenario: Retry with backoff
- **WHEN** an invocation declares `vt:retry` with at most 3 attempts and exponential backoff, and the first two calls fail with `rate-limited`
- **THEN** the third call is made after the backoff timers fire, and each attempt is a separate journaled call

#### Scenario: Retries exhausted
- **WHEN** all declared attempts fail
- **THEN** `error.llm` is raised to the invoking state with the class of the last failure, and the attempt count is recorded

### Requirement: Streaming Partials
The system SHALL support opt-in streaming for LLM invocations. Each partial result is delivered as a `vt.llm.partial` event, and only the final, complete result is journaled as `llm.result`. Partial events SHALL NOT be required for replay, and transitions on them SHALL be restricted to targetless transitions.

#### Scenario: Partials update the context
- **WHEN** streaming is enabled and the model streams three chunks
- **THEN** three `vt.llm.partial` events are delivered, a targetless transition may append them to a display buffer, and one `llm.result` entry with the full content is journaled

#### Scenario: Partial transition with a target rejected
- **WHEN** a chart declares a transition with a target on `vt.llm.partial`
- **THEN** chart validation fails, because partials are not journaled and cannot drive replayable state changes

### Requirement: Tool Actors
The system SHALL let a tool be registered as an actor kind, with an IRI, an argument type and a result type in the IR type language. Charts invoke tools with `<invoke>` or `vt:request`. The tool's JSON Schemas SHALL be derived from its types and offered to LLM actors that are granted the tool. Tool results SHALL be journaled in the caller's journal.

#### Scenario: Tool call from an LLM decision
- **WHEN** an LLM decider chooses the candidate `call_tool` with the arguments for tool `search` that conform to its argument schema
- **THEN** the chart sends a `vt:request` to the `search` tool actor, and the tool's reply is journaled in the agent's journal and delivered as an event

#### Scenario: Invalid tool arguments
- **WHEN** a tool request carries arguments that do not conform to the tool's argument type
- **THEN** the tool is not executed, and the caller receives `error.tool` with the class `arguments` and the violated paths

#### Scenario: Tool schema offered to the model
- **WHEN** an LLM invocation is granted the tools `search` and `fetch`
- **THEN** the provider request includes exactly those two tool schemas, derived from their declared types

### Requirement: Capability Grants
An invoking state SHALL declare the send targets, tools and decider bindings that an invoked LLM, tool or plan actor may use. That actor SHALL be able to send only to granted targets. A send to a target that wasn't granted SHALL fail with `error.communication` raised in the sending actor, and the message SHALL NOT be delivered.

#### Scenario: Granted send succeeds
- **WHEN** an invoked LLM actor is granted the target `#_parent` and the tool `search`, and it sends to `search`
- **THEN** the message is delivered

#### Scenario: Ungranted send fails
- **WHEN** the same actor attempts to send to the virtual actor `vt:actor/Payments/1`, which was not granted
- **THEN** `error.communication` is raised in the sending actor with the reason `not-granted`, nothing reaches `vt:actor/Payments/1`, and the attempt is journaled

#### Scenario: Grants do not escalate
- **WHEN** an invoked actor with grants G invokes its own child and declares grants H for it
- **THEN** the child's effective grants are the intersection of G and H

### Requirement: LLM as Main Driver
The system SHALL support an agent pattern in which an LLM drives the whole agent. This pattern SHALL be an ordinary chart: a `vt:decide` loop state whose candidates are the agent's actions, tool calls and a finish event, with a bounded number of iterations declared on the chart. No runtime feature SHALL be specific to this pattern beyond deciders, grants and LLM invocations.

#### Scenario: ReAct loop runs to completion
- **WHEN** an agent chart in the ReAct pattern receives a task, the LLM decider chooses `call_tool` twice and then `finish`
- **THEN** the chart executes two tool requests, records three journaled decisions and reaches its final state with the answer in `done.state` data

#### Scenario: Iteration bound enforced
- **WHEN** the declared iteration bound is 10 and the decider has not chosen `finish` after 10 decisions
- **THEN** the loop raises `vt.agent.budget-exhausted`, and the chart's declared transition on it is taken

### Requirement: Usage and Cost Accounting
The system SHALL record, for every model call, the prompt tokens, completion tokens, model identifier, provider binding, latency and an optional cost in a declared currency. These SHALL be recorded as `vt-agent:` statements linked to the call. The system SHALL expose the cumulative usage of an actor and of its invoked descendants to the chart as a read-only system variable.

#### Scenario: Usage recorded per call
- **WHEN** a call uses 1200 prompt tokens and 300 completion tokens
- **THEN** the graph contains a `vt-agent:LlmCall` for it with those counts, the model identifier and the latency

#### Scenario: Budget guard on cumulative usage
- **WHEN** a guard reads the cumulative token usage of the agent, and that usage exceeds the agent's declared budget
- **THEN** the guard can route the chart to a budget-exhausted state before the next call is made

### Requirement: Agent Ontology Module
The system SHALL publish the `vt-agent:` ontology module with SHACL shapes. It defines at least `Decider`, `Decision`, `LlmCall`, `Tool`, `ToolCall`, `Grant`, `Plan` and their properties. It SHALL be aligned with PROV-O by subclassing, so that deciders are `prov:Agent`s and calls and decisions are `prov:Activity`s, and it SHALL work without a PROV-O reasoner.

#### Scenario: Shapes validate agent records
- **WHEN** the journal graph of an agent run is validated against the `vt-agent:` SHACL shapes
- **THEN** it conforms, and a record of an LLM call that is missing its model identifier is reported as a violation

#### Scenario: PROV query without reasoning
- **WHEN** a SPARQL query asks for every `prov:Activity` associated with a given `prov:Agent`, using the shipped alignment axioms materialized as ordinary triples
- **THEN** the query returns the agent's LLM calls and decisions without a PROV-O reasoner

#### Scenario: Plain charts need no agent module
- **WHEN** a chart that uses no decide states, LLM invocations or tools is projected to RDF
- **THEN** the projection contains no `vt-agent:` terms
