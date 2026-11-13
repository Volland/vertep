## 1. Graph first

- [x] 1.1 Add milestone `M9 Agent wallet` to `tg.md/roadmap.md`, with `precedes::` from M5 and M8, and add it to the roadmap diagram
- [x] 1.2 Add a Component section `vertep-wallet` to `tg.md/components.md`, before any code lands:
  - edges `provides::` (agent-wallet, credential-exchange, oid4vc), `uses::` (vertep-federation, vertep-agent, vertep-runtime, vertep-rdf), `realizes::` and `scheduled_in::` M9;
  - add the crate to the component diagram;
  - update the `vertep-lean` section with the wallet model
- [x] 1.3 Add Capability sections `Agent wallet`, `Credential exchange` and `OID4VC` to `tg.md/capabilities.md`, and list `agent-wallet`, `credential-exchange` and `oid4vc` in its `openspec:` frontmatter
- [x] 1.4 Record design decisions D1–D13 as `Decision` sections in `tg.md/decisions.md`, with the `motivated_by::` edges that design.md names
- [x] 1.5 Record the six constraints in `tg.md/constraints.md`, each with `source:` and `constrains::` edges to the decisions it binds:
  - OpenID4VC 1.0;
  - Data Integrity EdDSA;
  - SD-JWT and SD-JWT VC;
  - Bitstring Status List;
  - EdDSA-only key manager;
  - no remote JSON-LD contexts
- [x] 1.6 Write the View `tg.md/views/wallet.md`, with `describes::` edges:
  - wallet actor and role charts;
  - plugin interfaces;
  - exchange vocabulary;
  - Lean model and conformance pipeline;
  - Mermaid diagrams with no wiki links inside them

## 2. Lean exchange model

- [ ] 2.1 Write `VertepLean/Wallet.lean` (D5):
  - agents, keys and compromised keys;
  - symbolic messages and signatures;
  - fresh nonces;
  - adversary knowledge and network actions (send, deliver, drop, duplicate, inject);
  - issuer, holder and verifier steps;
  - ghost state;
  - `Reachable`
- [ ] 2.2 Write `VertepLean/WalletTheorems.lean`, proving the six theorems as invariants of `Reachable`: `issue_needs_possession`, `issue_once_per_offer`, `present_needs_consent`, `disclosure_within_consent`, `accept_sound` and `accept_once_per_nonce`. Add axiom checks to `VertepTests`
- [ ] 2.3 Write `VertepLean/WalletUnsafe.lean`: a verifier without the audience check, and a reachable state where it accepts a presentation made for another verifier
- [ ] 2.4 Write `VertepLean/WalletCheck.lean` and `VertepWalletMain.lean`, and add `lean_exe «vertep-wallet»`:
  - strict replay of a JSONL exchange history;
  - the first failing step reported with its position and reason
- [ ] 2.5 Add the wallet model to the Verification View's Protocol models table and diagram
- [ ] 2.6 Tests for `Theorems proven` and `Unsafe variant rejected`, each with `@tg: verifies::` in a Rust test that runs the Lean build, as the other models do

## 3. Crate, interfaces and vocabulary

- [ ] 3.1 Create the crate `vertep-wallet` (workspace member and workspace dependency) with features `tsp`, `sd-jwt`, `oid4vc` and `testkit`. Add facade feature `wallet` to `vertep`
- [ ] 3.2 Define the traits `ExchangeProtocol`, `CredentialFormat`, `CredentialStore`, `ConsentPolicy` and `StatusResolver`, and `WalletRegistry` with built-in IRIs under `https://w3id.org/vertep/wallet#` (D3, `Pluggable Wallet Interfaces`)
- [ ] 3.3 Define the exchange vocabulary and history JSONL (D4, `Exchange Vocabulary`), with no claim values and no keys in histories
- [ ] 3.4 Write the wallet ontology terms in `ontology/wallet.ttl`: credential metadata, exchange and consent records
- [ ] 3.5 Tests: `Application registers a credential format`, `Built-in plugin replaced`, `Unknown protocol refused`, `History in the vocabulary` and `Unmapped message refused`, each with `@tg: verifies::`

## 4. Wallet actor and sealed store

- [ ] 4.1 Write the wallet chart and the virtual actor `wallet/<DID>` (D2, `Wallet Actor`), one per identity
- [ ] 4.2 Implement the sealed store (D8, `Sealed Credential Store`):
  - an XChaCha20-Poly1305 content key, wrapped by HPKE to the key manager's X25519 key;
  - the digest as associated data;
  - metadata-only RDF;
  - erasure on delete
- [ ] 4.3 Implement holder-binding keys through `KeyManager` (`Holder Binding Keys`)
- [ ] 4.4 Implement lifecycle: list, show, delete, and selection that excludes expired, revoked and deleted credentials (`Credential Lifecycle`)
- [ ] 4.5 Tests for every scenario of `Wallet Actor`, `Sealed Credential Store`, `Holder Binding Keys` and `Credential Lifecycle`, each with `@tg: verifies::`

## 5. Credential formats

- [ ] 5.1 Move the Data Integrity code from `vertep-federation::credential` into a `DataIntegrity` format with `eddsa-jcs-2022`. `verify_grant_credential` keeps its API, and the M5 grant tests still pass
- [ ] 5.2 Add `eddsa-rdfc-2022`:
  - `oxjsonld` with bundled contexts only (VCDM 2.0, Data Integrity, status list, Vertep);
  - canonicalisation by `vertep-rdf::canon`;
  - a document with a remote context is refused
- [ ] 5.3 Implement SD-JWT VC behind `sd-jwt`:
  - JWS EdDSA through the key manager;
  - salted disclosures;
  - `_sd` arrays;
  - a key-binding JWT with `sd_hash`, nonce and audience
- [ ] 5.4 Add the test vectors of the Data Integrity EdDSA and SD-JWT VC specifications, valid and invalid
- [ ] 5.5 Tests for `Data Integrity round trip`, `RDF canonical proof survives reserialisation`, `Selective disclosure` and `Interoperable examples`, each with `@tg: verifies::`

## 6. Consent

- [ ] 6.1 Implement `ConsentPolicy` (D7), which computes candidates: request ∩ policy for that verifier ∩ live credentials, plus `refuse`. The default policy refuses everything
- [ ] 6.2 Write the holder role chart's consent `vt:decide` state, with rule, human and LLM deciders through `Decider`. The LLM sees claim names, verifier and purpose only
- [ ] 6.3 Tests for every scenario of `Consent Before Disclosure`, including the out-of-bounds LLM answer, each with `@tg: verifies::`

## 7. Status lists

- [ ] 7.1 Implement Bitstring Status List issuing and revocation, published through the HTTP endpoint or a `vertep-webvh` publisher (D12)
- [ ] 7.2 Implement `StatusResolver` over `ureq`, with verification, a cache bounded by `ttl` and policy, and fail-closed on unknown status
- [ ] 7.3 Tests for `Revoked credential refused` and `Unreachable status fails closed`, each with `@tg: verifies::`

## 8. Native exchange over TSP

- [ ] 8.1 Write the issuer, holder and verifier role charts of the Vertep Credential Exchange 1.0. `vt validate` and `vt certify` must pass on each (`Role Charts`)
- [ ] 8.2 Implement the codec for messages under `https://w3id.org/vertep/exchange/1.0#` and the TSP binding: direct, routed and through a mediator (D10, `Native Exchange Over TSP`)
- [ ] 8.3 Enforce, in the role charts and formats:
  - possession over the issuer's own fresh nonce, one issuance per offer;
  - audience, nonce freshness and single use;
  - holder binding, trusted issuer, status and requested claims only

  These are `Possession Before Issuance`, `Issuance At Most Once Per Offer` and `Bound And Fresh Presentations`
- [ ] 8.4 Tests for every scenario of `Role Charts`, `Possession Before Issuance`, `Issuance At Most Once Per Offer`, `Bound And Fresh Presentations` and `Native Exchange Over TSP`, each with `@tg: verifies::`

## 9. Conformance kit

- [ ] 9.1 Implement `testkit`, the network between role charts (D6):
  - an in-memory transport for TSP and a loopback HTTP proxy for OpenID4VC;
  - a seeded adversarial scheduler: drop, duplicate, reorder, replay to the same and another verifier, inject from knowledge;
  - complete history recording
- [ ] 9.2 Run `vertep-wallet` over recorded histories from the Rust tests, as the custody and lease oracles run; the test fails if the Lean build is missing and feature `differential` is on
- [ ] 9.3 Write a deliberately faulty test plugin whose verifier skips the nonce check
- [ ] 9.4 Tests for `Conformant history accepted`, `Disabled step reported`, `Built-in plugins conformant` and `Faulty plugin caught`, each with `@tg: verifies::`

## 10. OpenID4VC

- [ ] 10.1 Implement OpenID4VCI issuer endpoints with axum (D11):
  - issuer metadata;
  - the pre-authorised code token endpoint with transaction codes;
  - the nonce endpoint;
  - the credential endpoint with `openid4vci-proof+jwt`

  This is `OpenID4VCI Pre-Authorised Issuance` and `Credential Issuer Metadata`
- [ ] 10.2 Implement the OpenID4VCI holder over `ureq`: offer URI, token, nonce, proof and credential
- [ ] 10.3 Implement the OpenID4VP verifier: DCQL query, nonce, `redirect_uri` and `decentralized_identifier` client identifiers with EdDSA request objects, and `direct_post` handling
- [ ] 10.4 Implement the OpenID4VP holder: request parsing, DCQL matching, consent, the presentation through the format, and `access_denied`
- [ ] 10.5 Write the role charts and codecs that map onto the exchange vocabulary, and run the conformance kit (`OpenID4VC Conformance`)
- [ ] 10.6 Add the specification example fixtures for offers, metadata, requests and DCQL (`OpenID4VC Interoperability`)
- [ ] 10.7 Tests for every scenario of the five `oid4vc` requirements, each with `@tg: verifies::`

## 11. Grant credentials from a wallet

- [ ] 11.1 Make grant credentials a wallet credential type. A presented grant is verified (format, then status) and installed as a `Grant` with `issuer` and `id`. Revocation removes it at the next status check (D13)
- [ ] 11.2 Tests for `Grant presented over an exchange`, `Presented grant enforced before the journal` and `Revoked grant credential refused`, each with `@tg: verifies::`

## 12. CLI, docs and annotations

- [ ] 12.1 Implement `vt wallet`: `create`, `list`, `show`, `issue`, `offer`, `accept`, `request`, `present`, `verify`, `revoke` and `conformance`, with keys in the configured key store (`Wallet Command`)
- [ ] 12.2 Tests for `Issue and present between two wallets` and `Failing check named`, each with `@tg: verifies::`
- [ ] 12.3 Annotate the entry points of every requirement with `@tg: implements::`, using `{confidence: partial}` where a requirement is partly met. Annotate the crate's `lib.rs` with `//! @tg: implements::` for D1–D13
- [ ] 12.4 Write the crate README with a doctest. Add a site page `site/docs/wallet.html` covering the wallet, plugins, consent, the exchange model and theorems, and the conformance kit, and link it from the docs index
- [ ] 12.5 Run `cargo fmt`, `cargo clippy -D warnings` and the workspace tests. The Lean build passes with `differential`
- [ ] 12.6 `tg check`, `tg ontology check` and `tg trace agent-wallet credential-exchange oid4vc did-identity --gaps` show no new gaps
