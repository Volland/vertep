## Context

See proposal.md, Why. What exists today:

- **Keys.** [[decisions#Keys stay behind a key manager]]: `KeyManager` signs with Ed25519, does X25519 key agreement, and exposes only public keys and key names.
- **Grant credentials.** `vertep-federation::credential` issues and verifies grant credentials:
  - VCDM 2.0 with an `eddsa-jcs-2022` Data Integrity proof;
  - the issuer must be trusted and the key must be in its `assertionMethod`;
  - the validity period is checked;
  - the result is a `Grant`, enforced before the journal ([[decisions#Grants enforced before the journal]]).
- **Deciders.** `vt:decide` states and the `Decider` trait choose among enabled candidates. The Lean `Agent` model proves `decider_safety`, `stale_not_taken` and `unknown_not_taken`: a decision outside the offered candidates is never taken.
- **TSP.** TSP relationships, direct or routed ([[decisions#TSP direct when reachable, routed otherwise]]), and a versioned application protocol inside TSP ([[decisions#TSP pickup is a versioned protocol inside TSP]]).
- **The Lean bridge** ([[decisions#Lean verification bridge]]). Each protocol is modelled as a nondeterministic system over any interleaving. Its theorems are invariants of every reachable state. A Lean checker (`vertep-journal`, `vertep-decide`, `vertep-lease`, `vertep-custody`) replays JSONL histories from Rust tests in strict mode. `CustodyReconfigUnsafe` shows the pattern for an unsafe variant with a counterexample.
- **Libraries already in the workspace:**
  - `chacha20poly1305` and `hpke` (keeper replicas);
  - `oxjsonld` (JSON-LD to RDF);
  - RDF canonicalisation in `vertep-rdf::canon`;
  - `ureq` (HTTP client) and `axum` (HTTP server).

## Goals / Non-Goals

**Goals:**

- **One exchange model** that every protocol plugin is checked against, with no per-plugin Lean work.
- **Every exchange is a journaled chart run**, so audit, replay and crash recovery come from the runtime rather than from wallet code.
- **No private key and no claim value** reaches the graph, the journal's RDF, a history file or a log line.
- **Interoperability with OpenID4VC 1.0** wallets and verifiers that accept EdDSA.

**Non-Goals:**

- **Out of scope for now; the plugin interfaces must admit them later:**
  - DIDComm Issue Credential and Present Proof;
  - ISO mdoc;
  - BBS+ and `ecdsa-sd-2023`;
  - P-256 keys, and therefore ES256.
- **Not attempted:** OpenID4VCI's authorisation-code flow, wallet attestations, and HAIP profile compliance.
- **No browser wallet in this milestone.** The crate keeps `vertep-wasm` builds possible, but no WebAssembly API is added.
- **No proof that the cryptography is sound.** The model treats signatures, hashes and encryption as perfect, and the specs' test vectors cover the implementation.

## Decisions

### D1. A new crate `vertep-wallet`, with formats and protocols as feature-gated modules

The crate depends on `vertep-federation` (keys, DIDs, resolver, TSP), `vertep-agent` (deciders) and `vertep-runtime` (actors, journals). Its features:

- `tsp`: the native protocol;
- `sd-jwt`;
- `oid4vc`, which needs `axum` and `ureq`;
- `testkit`: the conformance kit.

**Alternatives rejected:**

- **Growing `vertep-federation`.** Federation is mandatory for every DID actor, and most deployments need no wallet.
- **An external SSI stack** such as `ssi`/DIDKit. It has its own key handling, which conflicts with [[decisions#Keys stay behind a key manager]]. Its dependency tree and WebAssembly size are large. Its protocol state would also live outside the journal.

- motivated_by: [[decisions#Keys stay behind a key manager]]
- motivated_by: [[decisions#Lean verification bridge]]

### D2. The wallet is a virtual actor; each exchange is an invoked role chart

- **The wallet.** A wallet is the virtual actor `wallet/<percent-encoded DID>` running the wallet chart, which handles listing, deletion, policy changes and exchange dispatch.
- **Exchanges.** Each exchange is an agent actor of the wallet's host running its protocol's role chart: issuer, holder or verifier. (Planned as an invoked child of the wallet actor; built as a sibling actor keyed by the wallet and the exchange id, which keeps each exchange's journal separate and replayable on its own.)
  - Wire messages become chart events, and chart sends become wire messages, through the protocol's codec.
  - The holder chart's disclosure step is a `vt:decide` state (D7).
- **What this gives:**
  - every exchange is a journaled sequence of macrosteps, so restart and replay need no wallet-specific code;
  - `vt certify` covers the role charts;
  - the existing journal and decider theorems apply unchanged.

**Alternative rejected:** a hand-written Rust state machine per protocol. It would duplicate the runtime's persistence and could not be certified or replayed.

- motivated_by: [[decisions#Virtual actor addresses append the key to the type]]
- motivated_by: [[decisions#Full SCXML first]]

### D3. Five plugin interfaces, registered by IRI

The interfaces are:

- `ExchangeProtocol`: its IRI and version, a role chart per role, and a codec that maps wire messages to and from chart events and exchange-vocabulary steps;
- `CredentialFormat`: issue, verify, derive a presentation, and the holder-binding check;
- `CredentialStore`: put, get, list and erase sealed records;
- `ConsentPolicy`: given a request and the wallet's credentials, the allowed disclosure candidates;
- `StatusResolver`: fetch and verify a status list entry.

A `WalletRegistry` maps IRIs to implementations, as the DID resolver registry does for methods. Built-ins register under `https://w3id.org/vertep/wallet#…` IRIs. An application can add plugins and replace built-ins.

**Alternative rejected:** one big `Wallet` trait. Formats and protocols vary independently (OpenID4VP carries SD-JWT VC or Data Integrity), so they must be separate.

- motivated_by: [[decisions#Membership behind one trait]] {note: "the same one-interface, many-implementations pattern with a shared test kit"}

### D4. One exchange vocabulary; histories carry names and digests, never claim values

Every codec labels each message with a vocabulary step:

| Step | Data |
| --- | --- |
| `offer` | issuer, nonce |
| `prove-possession` | holder key, nonce, issuer |
| `issue` | credential digest, bound key, offer nonce |
| `store` | digest |
| `request-presentation` | verifier, nonce, audience, requested claim names |
| `consent` | verifier, nonce, claim names |
| `present` | credential digest, signer key, nonce, audience, disclosed claim names |
| `verify` | accept or refuse, with reason |
| `refuse` | none |
| `revoke` | credential digest |

An exchange history is JSONL with one step per line, plus the network events: send, deliver, drop, duplicate, inject. Claim values and private keys are never written; claim names and SHA-256 digests stand in for them.

**Alternative rejected:** checking each protocol's own wire messages in Lean. That needs a Lean model per protocol and per version, which a third-party plugin author would have to write.

- motivated_by: [[decisions#Lean verification bridge]]

### D5. The Lean exchange model: symbolic messages, a network adversary, perfect cryptography

`VertepLean/Wallet.lean` models the system as follows:

- **Agents** are issuers, holders and verifiers, each with keys. A model parameter names the compromised keys.
- **Messages** are symbolic terms. The adversary knows every message ever sent, and can deliver any known message to anyone, any number of times. It can drop and reorder messages, and build new messages from known terms.
- **Signatures.** `Sig k m` can be built only by an agent holding `k`, or by the adversary when `k` is compromised.
- **Nonces** are fresh by construction: each one is a counter and its creator.
- **Ghost state** records every offer, possession proof, issuance, consent, presentation, acceptance and revocation.
- **Status** is the issuer's revoked set. A verifier reads it at acceptance; cached status is the risk under Risks.

`WalletTheorems.lean` proves the six theorems of [[openspec:credential-exchange#Exchange Model In Lean]] as invariants of `Reachable`. Each assumes the relevant issuer's and holder's keys are not compromised.

`WalletUnsafe.lean` weakens the verifier to skip the audience check and exhibits a reachable state where it accepts a presentation that was made for another verifier. This shows the audience check is necessary. The same pattern is used in `CustodyReconfigUnsafe`.

**Alternative rejected:** model checking with an external tool such as ProVerif or Tamarin. It is a second toolchain, its results stay outside the graph's Lean certificates, and it has no oracle connection to the Rust runs.

- motivated_by: [[decisions#Lean verification bridge]]

### D6. Plugin conformance is trace refinement, judged by the Lean checker

- **The checker.** `vertep-wallet` (`VertepWalletMain.lean`, `WalletCheck.lean`) replays a history as model actions. Each line must be an enabled action in the current model state, or the run fails at that line with a reason. This works as `vertep-lease` and `vertep-custody` do.
- **The kit as the network.** `vertep_wallet::testkit` owns the network between the role charts: an in-memory transport for TSP, and a loopback HTTP proxy for OpenID4VC. A seeded scheduler drops, duplicates, reorders and replays messages, including to a second verifier, and injects messages from the adversary's knowledge.
- **What follows.** The kit records every network event, so the history is complete. Any plugin whose histories the checker accepts inherits the theorems for the runs tested.

**Alternative rejected:** requiring each plugin to prove in Lean that its chart refines the model. That is sound, but it excludes most plugin authors. Trace checking gives everyone the same bar, and the built-in plugins also get chart certificates.

- motivated_by: [[decisions#Lean verification bridge]]

### D7. Consent is a decide state whose candidates are computed by the policy

The holder chart's consent state is a `vt:decide` state. Its candidates are computed when the decision is offered:

1. The `ConsentPolicy` takes the request's claims, intersects them with what the policy allows for that verifier, and keeps only claims a live credential has.
2. It returns one candidate per allowed disclosure, plus `refuse`.

The decider, whether a rule, a human prompt or an LLM, picks one candidate. Because of `decider_safety` and `unknown_not_taken`, a decision outside the offered set is never taken, so an LLM cannot widen a disclosure. The consent step in the history carries the chosen candidate.

**Alternative rejected:** a consent callback in Rust. It is not journaled as a decision, and the Agent-model theorems would not cover it.

- motivated_by: [[decisions#Offer-time decider safety]]
- motivated_by: [[decisions#Deciders at choice points]]

### D8. Sealed store: records encrypted under a wallet content key wrapped by the key manager

- **Encryption.** Each record is encrypted with XChaCha20-Poly1305 under the wallet's content key, using a fresh nonce and the credential digest as associated data.
- **Key wrapping.** The content key is wrapped with HPKE to the wallet's X25519 key-agreement key, which the key manager holds.
- **What the RDF sees.** The journal's RDF gets only the metadata triples and the digest. The sealed record lives in the store under the digest.
- **Deletion** erases the record, and the journal keeps a deletion entry with the digest only.

**Alternative rejected:** clear credentials in the journal, protected by storage encryption. Journals are replicated and exported for audit, and an exported journal would leak PII.

- motivated_by: [[decisions#Keepers store ciphertext under a per-object content key]]
- motivated_by: [[decisions#Keys stay behind a key manager]]

### D9. Formats: Data Integrity generalised from grants, `eddsa-rdfc-2022` over bundled contexts, SD-JWT VC in-house

- **`eddsa-jcs-2022`.** The Data Integrity code moves from `vertep-federation::credential` into a `DataIntegrity` format. `verify_grant_credential` keeps its signature and calls the format.
- **`eddsa-rdfc-2022`:**
  - `oxjsonld` turns the document into RDF, using only bundled contexts (VCDM 2.0, Data Integrity, status list and Vertep's own);
  - `vertep-rdf::canon` canonicalises the result;
  - a document that needs a remote context is refused.
- **SD-JWT VC** is implemented in the crate:
  - JWS compact with EdDSA through the key manager;
  - salted SHA-256 disclosures;
  - a key-binding JWT over the nonce, audience and `sd_hash`.

**Alternative rejected:** the `sd-jwt` crates. Their signers would need adapting to the key manager anyway, and the format is small enough to implement and test against the specification's vectors.

- motivated_by: [[decisions#Keys stay behind a key manager]]

### D10. The native protocol is a versioned application protocol inside TSP

The Vertep Credential Exchange 1.0 has these message types, each under `https://w3id.org/vertep/exchange/1.0#`:

- `Offer`, `CredentialRequest`, `Credential`;
- `PresentationRequest`, `Presentation`, `Result`;
- `Problem`.

They travel over the TSP relationship between the two DIDs, direct or routed, and through a mediator when the peer is offline. Each message carries its exchange id; nonces and audience are in the signed payload.

- motivated_by: [[decisions#TSP pickup is a versioned protocol inside TSP]]
- motivated_by: [[decisions#TSP direct when reachable, routed otherwise]]

### D11. OpenID4VC: axum endpoints for issuers and verifiers, ureq for holders

- **Issuers** serve the following, behind feature `oid4vc`:
  - `/.well-known/openid-credential-issuer`;
  - the token endpoint for the pre-authorised code;
  - the nonce endpoint;
  - the credential endpoint, which takes a `openid4vci-proof+jwt` proof.
- **Verifiers** create authorisation requests with a DCQL query and `direct_post`. They use one of two client identifier prefixes:
  - `redirect_uri`, for unsigned requests;
  - `decentralized_identifier`, for request objects signed with EdDSA through the key manager.
- **Each endpoint** hands its message to the role chart, so HTTP is only a binding.

**Alternative rejected:** implementing only the holder side. Agents then could not issue to or verify ecosystem wallets, and the conformance kit would need an external issuer.

### D12. Status lists are credentials the issuer publishes; verifiers cache them with a bound

- **Issuers** keep a Bitstring Status List per purpose (revocation), as a credential in a format the issuer supports. They publish it through a publisher: the HTTP endpoint, or the directory, git and command publishers of [[components#vertep-webvh]].
- **Verifiers** fetch the list with `ureq`, verify it, and cache it for at most the list's `ttl`, capped by the verifier's policy.
- **Unknown status fails closed** unless the policy says otherwise.

### D13. Grant credentials are wallet credentials

- A grant credential is a credential type the wallet holds like any other.
- The target's verifier role chart verifies it with the `DataIntegrity` format, then checks status, then calls the existing grant path. The resulting `Grant` is installed with `issuer` and `id` set from the credential.
- Revocation removes the installed grant at the next status check, and no later than the cache bound.
- Enforcement stays where it is, before the journal.

- motivated_by: [[decisions#Grants enforced before the journal]]

## Constraints

Each of these external rules binds the decisions named after it:

- **OpenID4VCI 1.0 and OpenID4VP 1.0** (OpenID Foundation) bind D11 and the `oid4vc` spec:
  - the nonce endpoint and `invalid_nonce`;
  - the `openid4vci-proof+jwt` proof type;
  - DCQL;
  - `direct_post`;
  - client identifier prefixes.
- **W3C VCDM 2.0, Data Integrity and its EdDSA cryptosuites (v1.0)** bind D9: proof configuration hashing, and `eddsa-jcs-2022` and `eddsa-rdfc-2022` transformation.
- **IETF SD-JWT and SD-JWT VC** bind D9:
  - disclosure hashing;
  - the `_sd` arrays;
  - the key-binding JWT with `sd_hash`;
  - `typ` values.
- **W3C Bitstring Status List v1.0** binds D12: the GZIP-compressed, base64url-encoded bitstring, its minimum size, and `statusPurpose`.
- **The key manager signs only Ed25519** ([[decisions#Keys stay behind a key manager]]). This binds D9 and D11 to EdDSA, and limits interoperability with ES256-only wallets.
- **JSON-LD remote context loading is a security risk.** This binds D9: bundled contexts only.

## Risks / Trade-offs

- **EdDSA only, so some OpenID4VC wallets and verifiers cannot interoperate** (many require ES256). → Mitigation: the `CredentialFormat` and `KeyManager` interfaces admit P-256 later, and a follow-up adds a P-256 key type.
- **Cached status: a revoked credential can be accepted until the cache expires.** → Mitigation: the cache is bounded by the list's `ttl` and the policy. The model's verifier reads status fresh, so the theorems cover a zero-length cache, and the design states the window.
- **Conformance proves only the runs tested.** Trace checking is not a proof for all executions of a plugin. → Mitigation: many seeded adversarial schedules, the faulty-plugin scenario shows the kit catches a missing check, and the built-in role charts are also certified.
- **Symbolic cryptography: an implementation bug in a signature or hash is outside the model.** → Mitigation: specification test vectors for every format, and negative vectors for tampering.
- **Consent fatigue or a misconfigured policy.** The model guarantees disclosure stays within the policy, not that the policy is wise. → Mitigation: the default policy refuses everything, and every consent decision is journaled and queryable.
- **LLM deciders could leak claim values through their own prompts.** The consent request gives the decider claim names, verifier and purpose only, never values; the spec's LLM scenario covers this.

## Migration Plan

- **Grant credentials keep working unchanged.** Existing grant credentials verify through the same `verify_grant_credential` API, now backed by the `DataIntegrity` format, and the test vectors of M5 must still pass.
- **No storage migration.** Wallets are new actors, and nothing existing is stored differently.
- **Rollback:** the facade feature `wallet` is off by default, and removing it removes the crate from builds.

## Open Questions

- **The registered media type for the native protocol's messages** is undecided: the TSP message type IRI may be enough. Any choice keeps D10.
- **Whether `vt wallet` gets an interactive consent prompt in the terminal**, or only policy files, in this milestone. It does not change the decide-state design.
