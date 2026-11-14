# agent-wallet Specification

## Purpose
Gives every Vertep identity a wallet: a journaled actor that receives, keeps and presents verifiable credentials, and issues and verifies them for other agents. Its protocols, formats, stores, consent policies and status checks are plugins, and it never discloses a claim without consent.

## Requirements

### Requirement: Wallet Actor
A wallet SHALL be an actor addressed under the identity it holds credentials for, with one wallet per identity. Its credentials, exchanges and consent decisions SHALL be recorded in its journal, so a wallet restarted on any node returns to the same state.

#### Scenario: Wallet survives a restart
- **WHEN** a wallet accepts a credential and the node stops and starts again
- **THEN** the restarted wallet lists the same credential, with the same metadata and status

#### Scenario: One wallet per identity
- **WHEN** a second wallet is created for an identity that already has one
- **THEN** creation fails with an error naming the existing wallet

### Requirement: Sealed Credential Store
A wallet SHALL store each credential encrypted at rest, under a key that only the key manager holds. The graph and the journal's RDF SHALL hold only the credential's metadata: types, issuer, subject identity, validity period, status reference and a content digest. A stored record whose content no longer matches its digest SHALL be refused, never presented.

#### Scenario: Stored credential is encrypted
- **WHEN** a credential with a claim `birthDate` is accepted and the store's bytes are read directly
- **THEN** the claim's value appears nowhere in them

#### Scenario: Graph holds metadata only
- **WHEN** the wallet's graph is queried after a credential is accepted
- **THEN** the credential's types, issuer, validity and digest are found, and none of its claim values are

#### Scenario: Tampered record refused
- **WHEN** a stored credential record is altered and then selected for a presentation
- **THEN** the wallet refuses it with an integrity error and presents nothing

### Requirement: Holder Binding Keys
A wallet SHALL bind every credential it requests to a key held by the key manager, and SHALL sign possession proofs and presentations through the key manager, so no private key reaches a credential, a message or the caller.

#### Scenario: Binding key from the key manager
- **WHEN** a wallet requests a credential
- **THEN** the credential is bound to a public key the key manager holds, and its possession proof was signed by that key

#### Scenario: Presentation signed by the bound key
- **WHEN** a wallet presents a credential
- **THEN** the presentation is signed by the key the credential is bound to, and verification fails if any other key signs it

### Requirement: Pluggable Wallet Interfaces
The wallet SHALL take five kinds of plugins, each registered under an IRI:
- exchange protocols;
- credential formats;
- credential stores;
- consent policies;
- status resolvers.

An application SHALL be able to add plugins of each kind and replace built-in ones. Using an IRI that no plugin is registered under SHALL fail with an error naming it.

#### Scenario: Application registers a credential format
- **WHEN** an application registers a format under its own IRI and a wallet is offered a credential in that format
- **THEN** the wallet verifies and stores it through the registered format

#### Scenario: Built-in plugin replaced
- **WHEN** an application registers its own credential store under the built-in store's IRI
- **THEN** the wallet stores credentials through the application's store

#### Scenario: Unknown protocol refused
- **WHEN** an exchange names a protocol IRI that no plugin is registered under
- **THEN** the exchange fails at once with an error naming the IRI, and nothing is journaled as accepted

### Requirement: Credential Formats
The wallet SHALL issue, verify and present credentials in these formats:
- W3C VCDM 2.0 with Data Integrity proofs `eddsa-jcs-2022` and `eddsa-rdfc-2022`;
- SD-JWT VC signed with EdDSA.

With SD-JWT VC, a presentation SHALL disclose only the selected claims, together with a key-binding proof over the verifier's nonce and audience.

#### Scenario: Data Integrity round trip
- **WHEN** a credential is issued with `eddsa-jcs-2022` and with `eddsa-rdfc-2022` and each is verified
- **THEN** both verify, and changing any claim makes verification fail

#### Scenario: RDF canonical proof survives reserialisation
- **WHEN** an `eddsa-rdfc-2022` credential is reserialised with its properties in another order
- **THEN** it still verifies

#### Scenario: Selective disclosure
- **WHEN** an SD-JWT VC with claims `name`, `birthDate` and `nationality` is presented with only `nationality` selected
- **THEN** the verifier sees `nationality`, and cannot recover `name` or `birthDate` from the presentation

#### Scenario: Interoperable examples
- **WHEN** the published examples of the Data Integrity EdDSA and SD-JWT VC specifications are verified
- **THEN** the valid examples verify and the invalid ones are rejected

### Requirement: Consent Before Disclosure
Before any presentation, the holder's chart SHALL decide the disclosure in a decide state. The decision SHALL be bounded by the request and the wallet's consent policy:
- it may disclose only claims that are both requested and allowed by the policy;
- it may disclose them only to that verifier and for that request's nonce.

The decider SHALL be pluggable: a rule policy, a human prompt or an LLM. A decision outside its bounds SHALL NOT be taken, and every consent decision SHALL be journaled.

#### Scenario: Policy narrows the disclosure
- **WHEN** a verifier requests `name` and `birthDate` and the policy allows only `birthDate` for that verifier
- **THEN** the presentation discloses `birthDate` alone

#### Scenario: Policy refuses
- **WHEN** a verifier the policy does not allow requests any claim
- **THEN** no presentation is sent, and the exchange ends with a refusal the verifier receives

#### Scenario: LLM decider cannot exceed the policy
- **WHEN** an LLM decider answers with a disclosure that includes a claim the policy does not allow
- **THEN** that decision is not taken, the outcome is journaled as out of bounds, and nothing is presented

#### Scenario: Consent is journaled
- **WHEN** a presentation is sent and the wallet's journal is replayed
- **THEN** the replay shows the consent decision, its verifier, nonce and claims, before the presentation

### Requirement: Credential Status
Issuers SHALL publish credential status with the W3C Bitstring Status List, and verifiers SHALL check it before accepting a credential. A status list that cannot be fetched or verified SHALL make verification fail unless the verifier's policy explicitly accepts unknown status.

#### Scenario: Revoked credential refused
- **WHEN** an issuer revokes a credential and a verifier then checks a presentation of it
- **THEN** verification fails with a revocation error

#### Scenario: Unreachable status fails closed
- **WHEN** a credential's status list cannot be fetched and the policy does not accept unknown status
- **THEN** verification fails, naming the status list

### Requirement: Credential Lifecycle
A wallet SHALL list, show and delete its credentials, and SHALL never select an expired, revoked or deleted credential for a presentation.

#### Scenario: Expired credential not presented
- **WHEN** a request matches only a credential whose `validUntil` has passed
- **THEN** the wallet reports that no credential satisfies the request, and presents nothing

#### Scenario: Deleted credential gone
- **WHEN** a credential is deleted
- **THEN** it is no longer listed, its sealed record is erased, and the journal records the deletion without its content

### Requirement: Wallet Command
`vt wallet` SHALL provide `create`, `list`, `show`, `issue`, `offer`, `accept`, `request`, `present`, `verify`, `revoke` and `conformance`. Its keys SHALL stay in the configured key store, and it SHALL report errors with the failing check named.

#### Scenario: Issue and present between two wallets
- **WHEN** `vt wallet issue` offers a credential to a second wallet, it is accepted, and the second wallet presents it to a verifier with `vt wallet present`
- **THEN** `vt wallet verify` on the verifier side reports the presentation valid, with its issuer and disclosed claims

#### Scenario: Failing check named
- **WHEN** `vt wallet verify` is given a presentation whose signature does not match
- **THEN** it exits with a failure status and an error naming the signature check
