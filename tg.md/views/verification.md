---
type: View
status: draft
openspec: [formal-model, scxml-conformance, "chart-rdf#SHACL Shapes For Charts", "chart-rdf#Round Trip Identity", "chart-rdf#Canonical Chart Hash", "type-system#JSON Schema Projection", "type-system#SHACL Projection", "event-journal#Deterministic Replay", "event-journal#Message Deduplication", "event-journal#Compare-And-Set Append", "deciders#Decider Safety", "llm-actors#Capability Grants", "shard-placement#Single Activation Guarantee", "wasm-bindings#Conformance Suite In WebAssembly"]
---
# Verification View

How Vertep earns its claims: a Lean 4 model on LeanRDF carries the proofs, its executable `step` is a differential oracle for Rust, the W3C suite runs on both, and certificates land in the graph.

The pattern follows verification-guided development: the Rust code stays untrusted and fast, the Lean model is small and proven, and agreement between them is tested on every conformance chart and on generated charts. [[views/statecharts]] describes what is being verified; [[views/overview]] shows where it sits.

## Trust architecture

Rust finds answers and Lean checks them: the theorems are about the Lean model, and the bridge to Rust is trace equality, not extraction.

- describes:: [[decisions#Lean verification bridge]]
- describes:: [[decisions#Configuration well-formedness is proven]]
- describes:: [[components#vertep-lean]]
- describes:: [[capabilities#Formal model]]
- describes:: [[constraints#Aeneas supports a Rust subset]]

```mermaid
flowchart TB
    subgraph untrusted["Untrusted, fast (Rust)"]
        RS["vertep-core step"]
        RV["Rust validator and type checker"]
        RH["RDFC-1.0 hashing, SHACL in Rust"]
    end
    subgraph trusted["Trusted, small (Lean 4)"]
        LM["IR, WellFormed, step, HasType"]
        TH["Theorems, no sorry,<br/>axioms pinned"]
        LR["LeanRDF: RDF 1.2 model,<br/>proven SHACL Core validator"]
        OR["oracle executable"]
        CE["vt certify checker"]
    end
    subgraph evidence["Evidence"]
        TR["trace equality on every test"]
        CR["conformance report"]
        CT["vt:certifiedBy certificate<br/>(N-Triples on stdout)"]
    end
    RS --> TR
    OR --> TR
    LM --> TH
    LR --> TH
    LM --> OR
    LM --> CE
    RS --> CR
    OR --> CR
    CE --> CT
```

What the trust boundary means in practice:

- A bug in Rust shows up as a trace difference or a failed conformance test, never as a wrong theorem.
- A bug in the Lean model shows up as a failed proof, or as a trace difference against Rust that is then investigated against the W3C text.
- The proofs quantify over every datamodel answer tape, so they hold for every datamodel; they do not prove ECMAScript itself correct.

## The Lean package

`vertep-lean` is a Lean 4 lake package with a pinned toolchain, depending on LeanRDF for the RDF 1.2 model and the proven SHACL Core validator; it has no `sorry` and pins its axioms in tests.

- describes:: [[components#vertep-conformance]]
- describes:: [[decisions#Modular vt ontology on RDF 1.2]]

```mermaid
classDiagram
    class IR {
        Chart, State, Transition
        Executable, Invoke
    }
    class WellFormed {
        Prop
        decide : Decidable proven correct
    }
    class Step {
        step : Session to Input to Tape to Session x Effects x Requests
    }
    class Types {
        Type, HasType
        check proven sound and complete
    }
    class Projection {
        toRDF, fromRDF
        shapes
    }
    class LeanRDF {
        RDF 1.2 graphs
        validate, validate_conforms
        RDFC canonical form
    }
    class Oracle {
        main : N-Quads + inputs + tape to trace JSONL
    }
    class Certify {
        main : N-Quads to certificate RDF
    }
    IR <.. WellFormed
    IR <.. Step
    IR <.. Projection
    Types <.. Step
    LeanRDF <.. Projection
    Step <.. Oracle
    WellFormed <.. Certify
    Projection <.. Certify
```

| Requirement | Content |
|---|---|
| `Lean Package` | `lake build` from a clean checkout, pinned toolchain, LeanRDF at a pinned revision |
| `Axiom Hygiene` | only `propext`, `Quot.sound`, `Classical.choice`; a new axiom fails the build |
| `IR Model In Lean` | the chart IR and a `WellFormed` predicate with a decision procedure proven correct |
| `Executable Step Semantics` | `step` with the same inputs and outputs as Rust, datamodel abstracted as an answer tape |

### The datamodel as an answer tape

Lean cannot run ECMAScript, so the Rust run records every datamodel request and its answer, and the Lean run must issue exactly the same requests in the same order while consuming the tape.

```mermaid
sequenceDiagram
    participant R as Rust step
    participant DM as Rust datamodel (boa, CEL, rust)
    participant T as Answer tape
    participant L as Lean step
    R->>DM: cond(expr) at microstep k
    DM-->>R: true
    R->>T: append (cond, expr, true)
    R->>DM: assign(loc, v)
    DM-->>R: ok
    R->>T: append (assign, loc, ok)
    Note over L,T: later, in the oracle
    L->>T: next request must be (cond, expr)
    T-->>L: true
    L->>T: next request must be (assign, loc)
    T-->>L: ok
    Note over L: a request mismatch is a trace difference
```

For the `null` datamodel Lean evaluates `In()` itself, so null-datamodel charts need no tape.

## Theorem inventory

Each milestone adds proof obligations; M0 proves the semantics, M1 the types, M2 the journal protocol, M3 agent safety, M4 single writers; M5 relies on interop tests and M6 on replaying Lean traces in WebAssembly.

- describes:: [[roadmap#M0 SCXML core]]
- describes:: [[roadmap#M1 Typed authoring]]
- describes:: [[roadmap#M2 Actor runtime]]
- describes:: [[roadmap#M3 Agents]]
- describes:: [[roadmap#M4 Cluster]]

| Milestone | Theorem | Statement | Requirement or task |
|---|---|---|---|
| M0 | Step determinism | equal session, input and tape give equal session, effects and requests | `Step Determinism Theorem` |
| M0 | Configuration well-formedness | from a legal configuration `step` reaches only legal ones: root active, one child per active compound, all children per active parallel, parents active; proven as `configuration_wellformedness` (2026-11-12) | `Configuration Well-Formedness Theorem` |
| M0 | Transition selection | each microstep's set is enabled, conflict-free and optimal per SCXML 3.13 | `Transition Selection Theorem` |
| M0 | Shapes equivalence | `validate shapes (toRDF c) = conforms` iff `WellFormed c` | `Shapes Equivalence Theorem` |
| M0 | RDF round trip | `fromRDF (project c base) = .ok (c, base)` on well-formed charts, any base (`Rdf.fromRDF_project`, proven); equal charts have equal canonical forms | `RDF Round Trip Theorem` |
| M0 | Dynamic type soundness | `step` never requests an undeclared location or data item; named extension point for M1 | `Type Soundness Statement` |
| M1 | Checker correctness | `check T v = true` iff `HasType v T`, on every declaration block (`Types.check_iff`, proven) | backlog 4.1, `Type Checker Correctness` |
| M1 | SHACL projection agreement | `validate (shapes T) (toRDF v) = conforms` iff `HasType v T`, via `validate_conforms` | backlog 4.2, `SHACL Projection` |
| M1 | JSON Schema agreement | a JSON value validates against the projection iff `HasType v T`, for the emitted keyword subset (`JsonSchema.jsonSchema_agrees`, proven) | backlog 4.3, `JSON Schema Projection Agreement` |
| M1 | Typed step soundness | well-typed chart, configuration and inputs give well-typed context and output events | M1 design, decision 9 |
| M2 | Replay determinism | replaying a journal yields the recorded configurations and effects | M2 task 12.2, `Deterministic Replay` |
| M2 | Dedup idempotence | processing a message id twice equals processing it once | M2 task 12.3, `Message Deduplication` |
| M2 | Outbox | every recorded send is dispatched at least once; no unrecorded send is dispatched | M2 task 12.4 |
| M2 | Single activation by CAS | of two appends at the same expected position at most one succeeds | M2 task 12.5, `Compare-And-Set Append` |
| M3 | Decider safety | any transition taken because of a decision was enabled at offer time | M3 task 8.2, `Decider Safety` |
| M3 | Grant enforcement | a send from an invoked actor is delivered only if its target is in the intersected grants | M3 task 8.3, `Capability Grants` |
| M3 | Replay without deciders | journaled decisions and `llm.result` inputs reach the same state without consulting deciders | M3 task 8.4 |
| M4 | One writer per epoch | per shard and epoch at most one writer's appends are accepted; accepted versions are totally ordered across epochs | M4 task 6.4, `Single Activation Guarantee` |

```mermaid
flowchart LR
    M0["M0<br/>semantics, shapes, round trip"] --> M1["M1<br/>types and projections"]
    M1 --> M2["M2<br/>journal protocol"]
    M2 --> M3["M3<br/>decider safety, grants"]
    M2 --> M4["M4<br/>fenced single writer"]
    M2 --> M6["M6<br/>Lean traces in WASM"]
    M4 --> M5["M5<br/>interop tests, no new theorems"]
```

## Protocol models

The journal protocol, decide states with grants, and shard leases are modelled as nondeterministic systems over any interleaving; their theorems are invariants of every reachable state, and each model has an oracle executable.

- describes:: [[capabilities#Event journal]]
- describes:: [[capabilities#Deciders]]
- describes:: [[capabilities#LLM actors]]
- describes:: [[capabilities#Shard placement]]
- describes:: [[components#vertep-runtime]] {note: "journal model and property tests"}
- describes:: [[components#vertep-agent]] {note: "decider model and decide-state differential tests"}
- describes:: [[components#vertep-cluster]] {note: "lease model and history replay"}
- describes:: [[decisions#Fencing epochs on journal appends]]

| Model (`VertepLean/`) | Proven, in every reachable state | Oracle and Rust test |
|---|---|---|
| `Journal`, `JournalTheorems` | `replay_determinism`, `replay_no_divergence`, `replay_equals_live`; `turn_idempotent`, `accepted_once`; `outbox_safe`, `outbox_complete`, `no_message_lost`; `cas_single`, `turn_cas_exclusive` | `vertep-journal` judges runs of `tests/journal_protocol.rs`: two nodes, crashes before and after append, redeliveries, concurrent activations |
| `Agent`, `AgentTheorems` | `decider_safety`, `stale_not_taken`, `unknown_not_taken`, `decision_dedup`, `decision_once`, `replay_live`; `chainGrant_mem`, `grants_do_not_escalate`, `grant_enforced`, `ungranted_not_delivered` | `vertep-decide` replays decision outcomes of `tests/decide_differential.rs`: generated charts with a decide state, adversarial decisions |
| `Lease`, `LeaseTheorems` | `grant_unique`, `one_writer_per_epoch`, `journal_epochs_ordered`, `shard_epochs_ordered`, `message_once`, `stale_append_fenced`, `frames_ordered`, `stale_frame_dropped` | `vertep-lease` replays `History::to_jsonl` of the cluster fault-injection test |

```mermaid
flowchart LR
    subgraph models["Lean models (abstract machine, any interleaving)"]
        J["Journal: CAS append, dedup, outbox, replay"]
        A["Agent: offers, decide, grants"]
        L["Lease: authorities, fenced store, receivers"]
    end
    subgraph rust["Rust under test"]
        RT["vertep-runtime Node"]
        AG["vertep-agent AgentMachine"]
        CL["vertep-cluster fault injection"]
    end
    RT -- "observation JSON" --> OJ["vertep-journal"]
    AG -- "decision outcomes JSONL" --> OD["vertep-decide"]
    CL -- "history JSONL" --> OL["vertep-lease"]
    J --> OJ
    A --> OD
    L --> OL
```

What the models abstract, and what the oracles found:

- The agent model takes the SCXML core as parameters (candidate names, offer-time and decision-time selection, schema check, macrostep), so its theorems hold for the Lean `step` and for any other core; the decision oracle takes the two decision-time answers from the Rust run, like the datamodel answer tape.
- The journal model advances the outbox mark only over delivered sends, as the Transactional Outbox requires. Rust used to advance it after handing sends to the transport and keep a refused send only as an in-memory retry, which a crash would lose; fixed 2026-11-11, the Rust mark now passes only delivered sends (`refused_send_survives_a_crash`).
- The journal property tests found that two live nodes whose durable mailboxes share one store lose messages: `StoreMailbox` numbered row IRIs by its own sequence, so they collided and acknowledging one message stripped the other's type and recipient. Fixed 2026-11-11 with row IRIs that carry the envelope's digest; `journal_protocol_shared_durable_mailboxes_lose_no_message` runs.
- The lease oracle needs every grant in the history. The fault-injection harness now records the grants `Membership::tick` makes as well as its own, and its histories replay in strict mode (11,241 decisions agree on five seeds) from the fault-injection test itself.
- The configuration proof found three Rust runtime bugs in the W3C Appendix D entry algorithm itself, each giving an illegal configuration on a valid chart; all fixed 2026-11-11/12 in both cores with Rust and Lean regressions (see [[decisions#Appendix D readings in the Rust core]]): the domain recomputed on entry from the history just recorded (`deep_history_of_single_region_parallel_reenters_the_parallel`), and entry climbing above a domain that lies below a history's parent (`history_entry_stops_at_the_transition_domain`, `history_default_through_a_history_stops_at_the_domain`). `reachable_configurations_are_legal` in `vertep-conformance` checks legality on generated charts; its generator never builds these shapes, which is why the proof, not the property test, found them.
- The protocol-model review also found that the Rust type checker did not terminate on `type A = optional<A>` against a non-null value; see [[decisions#Type checking stops on non-contractive recursion]], which the Lean type language mirrors (see the type language section below).

## Type language

`VertepLean/Types/` mirrors the type language of `vertep-core`: `HasType` is the least relation closed under the typing rules, `check` is the executable checker, and `check_iff` proves them equal on every declaration block.

- describes:: [[decisions#Type checking stops on non-contractive recursion]]
- describes:: [[decisions#One IR type language]]
- describes:: [[components#vertep-core]] {note: "types module, mirrored by VertepLean/Types"}
- describes:: [[components#vertep-conformance]] {note: "types_differential against vertep-types"}

Because `HasType` is a least fixed point, the guarded but non-contractive `type A = optional<A>` has only `null` (and `undefined`) as values. `check` carries the names under expansion at the current value position, cleared on descent into a list item, record field or union payload, and rejects a name that repeats there, as Rust does; it terminates on every block, validated or not, by the measure (value size, declarations not yet expanded here, type size). Completeness cuts loops: the name expansions and present optionals at one position form a chain, and a chain that repeats a name has a shorter one that does not (`Chain.dedup`).

| File | Content | Proven |
|---|---|---|
| `Types/Basic.lean` | `Ty`, `Decls`, lexical rules of `int`, `decimal`, `datetime`, `iri`, name resolution | `unexpanded_lt` (termination) |
| `Types/Check.lean` | `HasType`, `check` | `check_sound`, `Chain.dedup`, `check_iff` |
| `Types/JsonSchema.lean` | keyword-tree `Schema`, `validate` for the subset, `proj` mirroring `json_schema.rs` | `validate_proj` (`validate` of the projection equals `check` on JSON values), `jsonSchema_agrees` |
| `Types/Oracle.lean` | `vertep-types --cases CASES.jsonl` | differential: `tests/types_differential.rs`, 4,000 generated blocks, types and values, same verdict |

```mermaid
flowchart LR
    HT["HasType<br/>least fixed point"] <-->|check_iff| CK["check<br/>names under expansion"]
    CK <-->|"validate_proj: JSON values, distinct tags"| JS["validate (proj T)<br/>JSON Schema subset"]
    CK -->|vertep-types| RS["Rust Declarations::check"]
```

What the model leaves out, and what remains:

- `pattern` is read as the lexical rule its regular expression encodes; the regular expressions themselves are not proven equal to those rules. `int` is decided on the exact decimal, Rust decides it on the `f64`, so the differential cases stay away from the edges of the signed 64-bit range.
- The SHACL projection (backlog 4.2) and typed step soundness (4.4: the Lean `step` has no expression typer, so the values it assigns come from the untyped answer tape) are open.
- Typed charts certify (4.5, 2026-11-12): `Types/Syntax.lean` reads a chart's `vt:types` block as Rust does and `Rdf/Types.lean` derives its type triples (type IRIs, definitions, shapes), which the chart version covers, see [[decisions#The Lean checker recomputes a chart's type triples]].

## Differential oracle loop

The Lean `step` runs as an executable on the same charts and inputs as Rust; any difference in configurations, effects or datamodel requests fails CI and is shrunk into a regression test.

- describes:: [[capabilities#SCXML conformance]]
- describes:: [[components#vertep-rdf]] {note: "charts cross to Lean as N-Quads of their projection"}

```mermaid
flowchart LR
    G["generator: well-formed charts + input sequences"] --> RS["Rust core run"]
    IRP["IRP tests"] --> RS
    RS --> TAPE["answer tape"]
    RS --> T1["Rust trace JSONL"]
    G --> NQ["chart as N-Quads (vertep-rdf)"]
    IRP --> NQ
    NQ --> LO["Lean oracle: decode, WellFormed, step"]
    TAPE --> LO
    LO --> T2["Lean trace JSONL"]
    T1 --> CMP{"identical?"}
    T2 --> CMP
    CMP -- "yes" --> OK["pass"]
    CMP -- "no" --> SH["shrink to minimal chart and inputs"]
    SH --> REG["store as regression test"]
```

Design choices behind the loop:

- **One exchange format.** Charts go to Lean as N-Quads of their RDF projection, decoded by the Lean `fromRDF` that the round-trip theorem covers, so every differential test also exercises the projection (`Differential Oracle`).
- **Trace granularity.** One JSON Lines record per macrostep: configuration, effects, datamodel requests.
- **Budget.** A bounded sample runs on every pull request and the full generated corpus nightly; the oracle is compiled natively.
- **Shrinking.** `Randomized Differential Testing` requires counterexamples to be shrunk and stored, so a found bug stays found.
- **Later milestones reuse it.** M1 compares checker verdicts, M2 replays crash and redelivery histories through the Lean journal model, M3 feeds adversarial decider outputs, M4 replays fault-injection histories through the lease model, M6 runs the same traces in WebAssembly.

## Shapes and round trip

The SHACL shapes for charts are not a second, hand-kept definition of validity: a theorem ties them to `WellFormed`, and the RDF projection is proven reversible.

- describes:: [[capabilities#Chart RDF]]
- describes:: [[constraints#RDF is open-world]]
- describes:: [[decisions#IR is the hub]]

```mermaid
flowchart LR
    C["chart c (IR)"] -- "toRDF" --> G["RDF 1.2 graph"]
    G -- "validate shapes (LeanRDF)" --> V{"conforms?"}
    C -- "WellFormed c (decided)" --> W{"well-formed?"}
    V <-. "Shapes Equivalence Theorem" .-> W
    G -- "fromRDF" --> C2["chart c'"]
    C2 -. "RDF Round Trip Theorem: c' = c" .- C
    G -- "RDFC-1.0" --> H["canonical form, sha256"]
    H --> IRI["urn:vertep:chart:sha256:hex"]
```

Rules SHACL Core cannot state directly, such as a history default transition targeting descendants of the history's parent, are expressed over containment triples the projection materializes (`vt:ancestor`), so no SHACL-SPARQL is needed. If a rule still cannot be expressed, the equivalence proof fails, which keeps the gap visible. The projection uses no blank nodes, so canonicalization stays linear in practice.

The round trip is proven by capture: every node's emission is a sublist of the graph that holds every triple the decoder may read about fragments at or below the node. Fragments below a node start with its fragment and a colon, so siblings never overlap; a state's subtree is the region of its descendants' `s.<id>` fragments, kept apart by unique identifiers; only the chart node is typed `vt:Chart`; and the derived `vt:historyDefault` and `vt:choice` triples are never read (`VertepLean/Rdf/RoundTrip/`, `Rdf.fromRDF_project`).

## Conformance suite

The W3C SCXML IRP suite runs on the Rust core and on the Lean oracle for the `null` and `ecmascript` datamodels; every expected deviation is allow-listed with a reason, and an unexpected pass fails.

- describes:: [[constraints#SCXML interpretation algorithm]]
- describes:: [[decisions#Full SCXML first]]

```mermaid
flowchart TD
    W["IRP .txml templates, pinned revision"] --> X["pinned XSLT: ecmascript and null variants"]
    X --> H["committed files + content hash, checked in CI"]
    H --> HOST["in-memory test host<br/>sessions, SCXML I/O processor,<br/>invoked sessions, virtual clock"]
    HOST --> R["Rust verdict per test"]
    HOST --> TP["answer tapes"]
    TP --> L["Lean verdict per test"]
    R --> AL{"allow-list"}
    L --> AL
    AL --> REP["conformance report JSON + summary"]
    AL -- "mandatory fail not allow-listed" --> FAIL["build fails"]
    AL -- "allow-listed test passes" --> FAIL
    AL -- "entry names missing test" --> FAIL
```

| Requirement | Rule |
|---|---|
| `IRP Suite Import` | pinned revision, per-datamodel variants, IRP classification mandatory, optional or manual |
| `Pass Fail Protocol` | `pass` final state passes; `fail`, budget overrun or interpreter error fails with a reason |
| `Test Host` | in-memory, multi-session, virtual time, no real clock or network |
| `Rust Core Conformance` | every mandatory test gates CI |
| `Lean Model Conformance` | Lean verdict and trace must equal Rust's |
| `Expected Deviation Allow List` | test, datamodel, reason; unexpected pass and dangling entries fail |
| `Optional Test Reporting` | optional tests reported separately, outside the gate |
| `Conformance Report` | per test: datamodel, class, Rust verdict, Lean verdict, reason; totals |
| `Round Trip Corpus Checks` | every corpus chart survives SCXML and RDF round trips and conforms to the shapes |

The CEL datamodel runs the same suite through `confCel.xsl` variants of every datamodel-neutral template (`suite/cel/`). Tests without a variant are listed in `suite/cel/EXCLUDED.tsv` with a reason: not a template, ECMAScript regular expressions over `_event.raw`, or `<script>`, which the expression-only CEL datamodel lacks. Every mandatory CEL variant gates CI like the other datamodels.

Known risk: boa's ECMAScript gaps can fail tests unrelated to SCXML; those are allow-listed with the engine issue as reason, and the trait keeps another engine possible. M6 runs the same suite in WebAssembly (`Conformance Suite In WebAssembly`).

## Certificates in the graph

`vt certify` runs a Lean-compiled checker: given a chart as RDF it decides `WellFormed` and, when it holds, prints a certificate as N-Triples attached to the chart version IRI with `vt:certifiedBy`.

- describes:: [[decisions#Content-addressed chart versions]]
- describes:: [[components#vertep-cli]]

```mermaid
sequenceDiagram
    participant U as User or CI
    participant CLI as vt (vertep-cli)
    participant RDF as vertep-rdf
    participant CK as vt certify (Lean binary)
    U->>CLI: vt certify order.scxml
    CLI->>RDF: parse, project to N-Quads, hash
    RDF-->>CLI: N-Quads + chart version IRI
    CLI->>CK: vt-certify --chart file.nq
    CK->>CK: decode, derive type triples, recompute version, decide WellFormed
    alt well-formed
        CK-->>CLI: certificate N-Triples, exit 0
        CLI-->>U: print certificate on stdout
    else not well-formed
        CK-->>CLI: rejection with the failing rule, exit 1 (2 if not a chart projection)
    end
```

The checker (`VT_CERTIFY`, or `vt-certify` on `PATH`, built by `lake build` in `lean/vertep-lean`) is found and run by `certify` in `crates/vertep-cli/src/commands.rs`; the certificate is built by `certificate` in `lean/vertep-lean/VertepLean/Certify.lean`. Its theorem list is fixed there:

```turtle
<urn:vertep:chart:sha256:4be1...> vt:certifiedBy <urn:vertep:cert:sha256:91c0...> .
<urn:vertep:cert:sha256:91c0...> a vt:Certificate ;
    vt:theorem "Vertep.checkWF_iff", "Vertep.violations_eq_nil_iff", "Vertep.Rdf.fromRDF_sound",
               "Vertep.step_deterministic", "Vertep.transition_selection",
               "Vertep.step_requests_declared" ;
    vt:leanToolchain "leanprover/lean4:v4.35.0" ;
    vt:leanRdfRevision "b407b97424150fec25a1ade514c71ea7abc61b42" ;
    vt:checkerHash "sha256:..." .
```

The certificate IRI hashes the version and the certificate body. `vt-certify` adds `prov:generatedAtTime` only when given `--time`, which `vt certify` never passes, so the certificate for a chart and checker is the same on every run. `vt certify` only prints it; putting it into a graph store is up to the caller.

Because the chart IRI is a content hash, a certificate can never apply to a different chart; a new version needs a new certificate. Once loaded into a store, certificates are ordinary graph facts, so "which running actors use uncertified charts" is a SPARQL query over the runtime graph.

## Later - Aeneas for the kernel

Proving the Rust code itself is deferred: the pure `step` kernel is written in a style Aeneas can translate, so the oracle bridge can later be replaced by a proof for that kernel only.

- describes:: [[decisions#No-std core and tokio shell]]

```mermaid
flowchart LR
    NOW["now: Lean model + differential oracle"] --> KEEP["kernel kept in Aeneas-friendly Rust:<br/>no async, no trait objects inside the kernel,<br/>no interior mutability, simple loops"]
    KEEP --> LATER["later: Aeneas translates step kernel to Lean"]
    LATER --> REF["prove translated kernel refines the Lean step"]
    REF --> GAP["oracle remains for shell, datamodels and stores"]
```

The datamodel trait object at the kernel boundary is the main obstacle; when Aeneas work starts, the kernel will be parameterized by a request and answer interface identical to the answer tape, which is already the Lean abstraction.
