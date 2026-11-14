## Purpose

Defines credential exchange once, independently of any wire protocol: an abstract vocabulary every exchange protocol maps onto, role charts that run it, a native protocol over TSP, and a Lean model whose theorems and checker every protocol plugin's runs must satisfy.

## ADDED Requirements

### Requirement: Exchange Vocabulary
Every exchange protocol SHALL map each message it sends or receives onto one of these abstract steps:
- `offer` (with the issuer's nonce);
- `prove-possession`;
- `issue`;
- `store`;
- `request-presentation` (with the verifier's nonce, audience and query);
- `consent`;
- `present`;
- `verify`;
- `refuse`;
- `revoke`.

Each exchange SHALL record its steps as an exchange history in this vocabulary.

#### Scenario: History in the vocabulary
- **WHEN** an issuance and a presentation run over any built-in protocol
- **THEN** each exchange has a history whose steps are all vocabulary steps, in the order they took effect

#### Scenario: Unmapped message refused
- **WHEN** a protocol plugin delivers a message it does not map onto a vocabulary step
- **THEN** the exchange fails with an error naming the message, and nothing is issued, stored or accepted

### Requirement: Role Charts
Each exchange protocol SHALL run each of its roles (issuer, holder, verifier) as a Vertep chart. Every role chart SHALL validate and SHALL be certifiable by `vt certify`, and every exchange SHALL be journaled, so replay reproduces it without contacting any peer.

#### Scenario: Role charts validate and certify
- **WHEN** `vt validate` and `vt certify` run on every role chart of the built-in protocols
- **THEN** each chart is valid and receives a certificate

#### Scenario: Exchange replays from the journal
- **WHEN** a completed presentation exchange is replayed from the holder's journal
- **THEN** the replay reaches the same final state without sending or receiving any message

### Requirement: Possession Before Issuance
An issuer SHALL issue a credential only to a key that signed a possession proof over a nonce that this issuer created, has not yet used, and has not let expire. A proof over another issuer's nonce, a used nonce or an expired nonce SHALL be refused.

#### Scenario: Valid possession proof
- **WHEN** a holder signs a possession proof over a fresh offer nonce and requests the credential
- **THEN** the issuer issues it, bound to the key that signed the proof

#### Scenario: Another issuer's nonce refused
- **WHEN** a holder sends a possession proof over a nonce created by a different issuer
- **THEN** the issuer refuses, and issues nothing

#### Scenario: Reused nonce refused
- **WHEN** a second request arrives with a possession proof over a nonce that already led to an issuance
- **THEN** the issuer refuses the second request

### Requirement: Issuance At Most Once Per Offer
An issuer SHALL issue at most one credential per offer, however often a request is delivered, duplicated or retried.

#### Scenario: Redelivered request issues once
- **WHEN** the same credential request is delivered three times
- **THEN** exactly one credential is issued, and the later deliveries receive that same credential or a refusal

### Requirement: Bound And Fresh Presentations
A verifier SHALL accept a presentation only when all of these hold:
- it answers a request this verifier made, with this verifier's nonce, which has not been used and has not expired;
- it names this verifier as its audience;
- it is signed by the key its credential is bound to;
- its credential was issued by an issuer the verifier trusts;
- its credential is not revoked;
- it discloses only claims the request asked for.

A verifier SHALL accept at most one presentation per nonce.

#### Scenario: Replay to another verifier refused
- **WHEN** a presentation made for verifier A is sent to verifier B
- **THEN** B refuses it for the wrong audience and nonce

#### Scenario: Replayed presentation refused
- **WHEN** verifier A receives, for a second time, a presentation it already accepted
- **THEN** A refuses it as a reused nonce

#### Scenario: Wrong holder key refused
- **WHEN** a presentation is signed by a key other than the one its credential is bound to
- **THEN** the verifier refuses it for failed holder binding

#### Scenario: Untrusted issuer refused
- **WHEN** a presentation carries a credential from an issuer the verifier does not trust
- **THEN** the verifier refuses it, naming the issuer

#### Scenario: Unrequested claim refused
- **WHEN** a presentation discloses a claim that the request did not ask for
- **THEN** the verifier refuses it

### Requirement: Native Exchange Over TSP
The Vertep Credential Exchange SHALL carry issuance and presentation between DIDs as TSP messages typed under a versioned Vertep IRI. Messages SHALL be sent direct when the peer is reachable and routed through intermediaries otherwise, and SHALL support all three roles.

#### Scenario: Issuance over TSP
- **WHEN** an issuer agent and a holder agent with TSP relationships run an issuance
- **THEN** the holder's wallet stores the credential, and both journals record the exchange

#### Scenario: Presentation through a mediator
- **WHEN** a verifier requests a presentation from a holder that is reachable only through a TSP mediator
- **THEN** the presentation is delivered through the mediator and verified, and the mediator stores only sealed messages

### Requirement: Exchange Model In Lean
The Lean package SHALL model credential exchange among issuers, holders and verifiers over a network that may drop, duplicate, reorder, replay and inject messages, with signatures that only a key's holder can make. It SHALL prove the following for every reachable state, with no `sorry` and only the standard axioms:
- `issue_needs_possession`;
- `issue_once_per_offer`;
- `present_needs_consent`;
- `disclosure_within_consent`;
- `accept_sound`, meaning a verifier accepts only genuine, unrevoked presentations bound to the holder's key and made for that verifier and nonce;
- `accept_once_per_nonce`.

#### Scenario: Theorems proven
- **WHEN** the Lean package builds
- **THEN** the six theorems are proven, and the axiom check of each lists only the standard axioms

#### Scenario: Unsafe variant rejected
- **WHEN** the model's verifier is weakened to skip the audience check
- **THEN** a counterexample, which is a reachable state where a presentation made for one verifier is accepted by another, is exhibited in Lean

### Requirement: Exchange History Checker
The Lean executable `vertep-wallet` SHALL replay an exchange history as model steps in strict mode. It SHALL accept the history only if every step is enabled in the model when it occurs, and SHALL otherwise report the first failing step with its position and the reason.

#### Scenario: Conformant history accepted
- **WHEN** the history of a correct issuance and presentation is given to `vertep-wallet`
- **THEN** it accepts the history and reports the number of steps checked

#### Scenario: Disabled step reported
- **WHEN** a history contains an `issue` with no prior possession proof over a fresh nonce
- **THEN** `vertep-wallet` rejects it, naming that step's position and the missing possession proof

### Requirement: Plugin Conformance
A conformance kit SHALL run any exchange-protocol plugin through issuances and presentations under adversarial network schedules: drops, duplicates, reordering, replays to the same and other verifiers, and injected messages. It SHALL record every history and have `vertep-wallet` judge it. A plugin SHALL count as conformant only when every recorded history is accepted.

#### Scenario: Built-in plugins conformant
- **WHEN** the kit runs the native TSP protocol and the OpenID4VC plugins over many seeded schedules
- **THEN** every recorded history is accepted

#### Scenario: Faulty plugin caught
- **WHEN** the kit runs a test plugin whose verifier does not check the nonce
- **THEN** at least one history is rejected, and the kit reports the plugin as not conformant with the failing step
