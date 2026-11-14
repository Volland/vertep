## 1. Graph first

- [x] 1.1 Add milestone `M10 Agent protocols` to `tg.md/roadmap.md`, with `precedes::` from M5, and add it to the roadmap diagram
- [x] 1.2 Add a Component section `vertep-kolyada` to `tg.md/components.md` before any code lands, with `provides::`, `uses::`, `realizes::` and `scheduled_in::` edges, and add it to the component diagram
- [x] 1.3 Add Capability sections `Agent protocols`, `Protocol conformance` and `Protocol spaces` to `tg.md/capabilities.md` and list them in its `openspec:` frontmatter
- [x] 1.4 Record D1-D12 as `Decision` sections in `tg.md/decisions.md` with the `motivated_by::` edges design.md names
- [x] 1.5 Record the constraints in `tg.md/constraints.md` with `source:` and `constrains::` edges
- [x] 1.6 Write the View `tg.md/views/kolyada.md` with `describes::` edges and Mermaid diagrams (no wiki links inside them)

## 2. Protocol document

- [x] 2.1 Create `crates/vertep-kolyada` with `//! @tg: implements::` for D1-D12 and the workspace lints
- [x] 2.2 Embed the frozen `vk:` context and the parser for the profile; reject foreign contexts and unknown terms (`Protocol Document`)
- [x] 2.3 Implement identity over `vertep-rdf` canonicalisation (`Protocol Identity`)
- [x] 2.4 Implement the SHACL subset interpreter (`Payload Shapes`)
- [x] 2.5 Implement well-formedness checks with named reasons (`Well-Formed Choreography`)
- [x] 2.6 Implement projection and the interface view (`Role Projection`)
- [x] 2.7 Tests for every scenario of the five requirements, each with `@tg: verifies::`
- [ ] 2.8 Write the agreement test between the subset interpreter and full SHACL validation in `vertep-rdf` (`Payload Shapes`)

## 3. Envelope and conformance

- [x] 3.1 Implement the signed envelope with a `Signer` and a `Verifier` trait, so the key manager and the resolver plug in (`Signed Message Envelope`)
- [x] 3.2 Implement `accept` with the fixed refusal reasons (`Conformance Check`)
- [x] 3.3 Implement causal delivery with the declared buffer and timeout (`Causal Delivery`)
- [x] 3.4 Implement violation records and equivocation proofs (`Violation Records`)
- [x] 3.5 Generate and publish test vectors (feature `vectors`) and a language-neutral check, `vectors/check.mjs`, that runs with Node and no Rust (`Test Vectors`); a TypeScript check of identities and verdicts waits for an RDFC-1.0 canonicalizer in JavaScript
- [x] 3.6 Tests for every scenario of these requirements, each with `@tg: verifies::`

## 4. Offers, discovery and the RDF bridge

- [x] 4.1 Implement signed offers and the `VertepProtocol` service entry (`Protocol Offers`, `Protocol Discovery`)
- [x] 4.2 Add the one-way bridge to `vertep-kolyada::bridge`; `vertep-rdf` cannot depend on the crate (`RDF Bridge`)
- [x] 4.3 Tests for every scenario of the three requirements, each with `@tg: verifies::`

## 5. Spaces in federation

- [x] 5.1 Implement genesis, joins and instance state in `vertep-federation::protocol` (`Protocol Instance`, `Joining`)
- [ ] 5.2 Implement the join policy, refusing by default (`Join Policy`)
- [x] 5.3 Implement role-derived authority (`Role-Derived Authority`); the crate has no grants to consult, so "no grant widens a role" holds by construction and is tested at crate level only
- [ ] 5.4 Implement pairwise sealed fan-out over the existing mailboxes (`Pairwise Sealed Fan-Out`)
- [x] 5.5 Implement hash-linked history, head comparison, late join and hash-only entries (`Hash-Linked History`)
- [x] 5.6 Implement sub-protocol invocation: the child genesis, its bindings drawn from the parent, and the completion message; the parent chart's waiting state is a chart concern (`Sub-Protocol Invocation`)
- [ ] 5.7 Tests for every scenario of these requirements, including TSP, DIDComm and local transports, each with `@tg: verifies::`

## 6. Runtime binding

- [ ] 6.1 Add `vt:plays` to the chart ontology and the IR metadata, and generate the DID service entry from it
- [ ] 6.2 Run the inbound check at the grants slot, journal the verdict with the input, derive checker state on replay
- [ ] 6.3 Run the outbound check in the send path
- [ ] 6.4 Tests for every scenario of `Chart Role Binding`, each with `@tg: verifies::`

## 7. CLI

- [ ] 7.1 Implement `vt kolyada`: `validate`, `hash`, `project`, `offer`, `check`, `vectors`
- [ ] 7.2 Tests for the commands, each with `@tg: verifies::`

## 8. Lean model

- [ ] 8.1 Write `Protocol.lean`: roles, instances, projection, `accept`, causal delivery, `Reachable`
- [ ] 8.2 Write `ProtocolTheorems.lean`: accepted messages are enabled by the projection, unbound roles are never accepted, causally consistent orders converge; add axiom checks to `VertepTests`
- [ ] 8.3 Write `ProtocolUnsafe.lean`: a variant without the role check and its counterexample
- [ ] 8.4 Write `ProtocolCheck.lean` and `VertepProtocolMain.lean`, with `lean_exe «vertep-kolyada»`, for strict JSONL replay
- [ ] 8.5 Record histories from the Rust tests and replay them in the Lean checker; add the model to the Verification View
- [ ] 8.6 Tests for `Theorems proven`, `Unsafe variant rejected` and `Rust history replays`, each with `@tg: verifies::`

## 9. Docs and annotations

- [x] 9.1 Annotate the entry points of the requirements the crate implements with `@tg: implements::`, using `{confidence: partial}` where a requirement is partly met; the rest are annotated as their tasks land
- [x] 9.2 Write the crate README with a doctest and a site page `site/docs/protocols.html`
- [ ] 9.3 Run `cargo fmt`, `cargo clippy -D warnings`, the workspace tests and the Lean build
- [ ] 9.4 `tg check`, `tg ontology check` and `tg trace agent-protocols protocol-conformance protocol-spaces --gaps` show no new gaps
