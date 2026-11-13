## Why

Vertep actors have DIDs, keys behind a `KeyManager`, TSP relationships and one kind of credential: the grant credential that `vertep-federation` issues and checks. They have no wallet. An agent cannot:

- receive a credential from an issuer;
- keep it safely;
- prove what it holds to a verifier;
- refuse to disclose more than it agreed to.

Without these it cannot act for a person or an organisation outside its own cluster.

Wallet protocols are where agents fail dangerously:

- an LLM-driven agent over-disclosing;
- a presentation replayed to a second verifier;
- a credential issued to a key nobody proved they hold.

Vertep already handles protocols like these:

- it models them as statecharts and journals every step;
- it proves its distributed protocols in Lean, with checkers that judge real Rust runs (journal, decide, leases, custody).

The wallet should be built the same way:

- **Protocol-based:** every exchange is a chart.
- **Pluggable:** protocols, formats, stores, consent and status are plugins.
- **Lean-verified:** one exchange model with theorems, and a checker that every plugin's runs must pass.

## What Changes

- **New crate `vertep-wallet`:** the wallet as a Vertep actor, one per holder identity.
  - A **sealed credential store**: credentials are encrypted at rest under a key-manager key, and the graph sees only metadata (type, issuer, validity, status, a digest).
  - **Holder-binding keys** from the `KeyManager`. No private key leaves it.
  - Each **role**, whether holder, issuer or verifier, runs as a Vertep chart.
- **Five plugin interfaces**, registered by IRI:
  - `ExchangeProtocol`: role charts, message codec and transport binding;
  - `CredentialFormat`: issue, verify, derive a presentation, holder binding;
  - `CredentialStore`;
  - `ConsentPolicy`;
  - `StatusResolver`.
  - The wallet never hard-codes a protocol or a format.
- **Consent is a `vt:decide` state.** Before any presentation, the holder chart decides what to disclose.
  - The decision goes through the `Decider` interface: a rule policy, a human, or an LLM.
  - It is bounded by the request and the policy, so even an LLM decider cannot disclose more than the policy allows. This reuses the decider-safety theorems.
- **An abstract exchange vocabulary** that every protocol maps its messages onto: offer, possession proof, issue, store, request, consent, present, verify, revoke.
- **A Lean model of credential exchange** over a hostile network:
  - the network can drop, duplicate, reorder, replay and inject messages;
  - signatures are unforgeable.
  - **Theorems:**
    - issuance only to a key that proved possession of a fresh offer;
    - at most one issuance per offer;
    - no presentation without consent;
    - disclosure within what was requested and consented;
    - a verifier accepts only genuine, unrevoked, holder-bound presentations made for it and its nonce;
    - no presentation accepted twice.
- **A Lean checker `vertep-wallet`** replays exchange histories in strict mode, and a **conformance kit** runs any plugin under adversarial network schedules and records the histories.
  - **A plugin is conformant when the checker accepts every history the kit records.** The same check applies to third-party plugins, with no Lean work on their side.
- **Plugins in this milestone:**
  - **Formats:**
    - VCDM 2.0 with Data Integrity `eddsa-jcs-2022` (generalised from the grant credential) and `eddsa-rdfc-2022` (on `vertep-rdf` canonicalisation);
    - SD-JWT VC with EdDSA, for selective disclosure.
  - **Protocols:**
    - the native *Vertep Credential Exchange* over TSP, for every role;
    - OpenID4VCI with the pre-authorised code flow, for issuer and holder;
    - OpenID4VP with DCQL and `direct_post`, for holder and verifier.
  - **Status:** Bitstring Status List, published by issuers and checked by verifiers.
- **Grant credentials become wallet credentials.** A peer can present its grant through any exchange protocol. Verification still yields a `Grant` before any journal append.
- **`vt wallet`:** `create`, `list`, `show`, `issue`, `offer`, `accept`, `request`, `present`, `verify`, `revoke` and `conformance`.

**Assumptions** (adjust before applying if they are wrong):

- All three roles are in scope, because agents issue to and verify each other.
- DIDComm Issue Credential and Present Proof, mdoc/ISO 18013-5, BBS+ and P-256 keys are out of scope; the plugin interfaces must admit them later.
- OpenID4VC covers the pre-authorised code flow only, with no authorisation-code flow and no wallet attestation.

## Capabilities

### New Capabilities

- `agent-wallet`:
  - the wallet actor and its sealed store;
  - holder-binding keys;
  - the plugin registry and the five interfaces;
  - consent as a decide state;
  - the credential formats and the status-list plugin;
  - `vt wallet`.
- `credential-exchange`:
  - the abstract exchange vocabulary and role charts;
  - the native TSP protocol;
  - the Lean exchange model, its theorems and the `vertep-wallet` checker;
  - the conformance kit and the plugin conformance rule.
- `oid4vc`: OpenID4VCI with the pre-authorised code flow (issuer and holder), and OpenID4VP with DCQL and `direct_post` (holder and verifier), as exchange-protocol plugins over HTTPS.

### Modified Capabilities

- `did-identity`: grant credentials can be held in a wallet and presented through any exchange protocol, and still yield a `Grant` before the journal. One ADDED requirement; the existing requirements stay as they are.

## Impact

- **Code:**
  - New crate `vertep-wallet`, with features `tsp`, `oid4vc` and `sd-jwt`, plus `testkit` for conformance.
  - `vertep-federation`: the Data Integrity code in `credential.rs` moves behind the `CredentialFormat` interface; `verify_grant_credential` keeps its API.
  - `vertep-agent`: a consent `Decider` and the consent decide state.
  - `vertep-cli`: `vt wallet`.
  - `lean/vertep-lean`:
    - modules `Wallet`, `WalletTheorems` and `WalletCheck`;
    - executable `vertep-wallet`.
  - Facade `vertep`: feature `wallet`.
- **Dependencies:**
  - existing: `ed25519-dalek`, `sha2`, `base64` and `bs58`; `ureq` (HTTP client) and `axum` (HTTP endpoints, feature `oid4vc`).
  - new: none required. SD-JWT and the JWS compact form are small enough to implement over the `KeyManager`, as the Data Integrity code already is.
- **The graph (`tg.md`):**
  - **New:**
    - a `vertep-wallet` component scheduled in a new milestone `M9 Agent wallet`;
    - a view `tg.md/views/wallet.md`;
    - capability sections for the three new capabilities.
  - **Changed:** the Verification View's Protocol models table gains the wallet model.
- **Decisions this change builds on:**
  - [[decisions#Keys stay behind a key manager]]: holder-binding and issuer keys;
  - [[decisions#Grants enforced before the journal]]: grant credentials presented through the wallet;
  - [[decisions#Lean verification bridge]]: the model and its checker;
  - [[decisions#TSP direct when reachable, routed otherwise]]: the native protocol;
  - the decider-safety results in the Agent model: consent.
- **Requirements added:**
  - [[openspec:agent-wallet#Wallet Actor]], [[openspec:agent-wallet#Pluggable Wallet Interfaces]], [[openspec:agent-wallet#Consent Before Disclosure]];
  - [[openspec:credential-exchange#Exchange Model In Lean]], [[openspec:credential-exchange#Plugin Conformance]];
  - [[openspec:oid4vc#OpenID4VP Verifier And Holder]];
  - [[openspec:did-identity#Grant Credentials From A Wallet]];
  - and the rest listed in the delta specs.
