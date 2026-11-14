# oid4vc Specification

## Purpose
Connects Vertep wallets to the OpenID for Verifiable Credentials ecosystem: OpenID4VCI issuance with the pre-authorised code flow and OpenID4VP presentation with DCQL, as exchange-protocol plugins that satisfy the same exchange model as every other protocol.

## Requirements

### Requirement: OpenID4VCI Pre-Authorised Issuance
As an issuer, a Vertep agent SHALL serve an OpenID4VCI 1.0 credential issuer with the pre-authorised code flow:
- the credential offer, optionally with a transaction code;
- the token endpoint;
- the nonce endpoint;
- the credential endpoint, which takes a JWT possession proof.

As a holder, a wallet SHALL accept such an offer and obtain the credential.

#### Scenario: Holder obtains a credential from an offer
- **WHEN** a wallet is given a credential offer URI from a Vertep issuer
- **THEN** it redeems the pre-authorised code, proves possession over the issuer's nonce, and stores the issued credential

#### Scenario: Wrong transaction code refused
- **WHEN** a holder redeems a pre-authorised code with the wrong transaction code
- **THEN** the token endpoint refuses it, and no credential is issued

#### Scenario: Proof without a fresh nonce refused
- **WHEN** a credential request carries a possession proof over a nonce the issuer did not issue, or one already used
- **THEN** the credential endpoint answers with an `invalid_nonce` error and issues nothing

### Requirement: Credential Issuer Metadata
An OpenID4VCI issuer SHALL publish its metadata at `/.well-known/openid-credential-issuer`. The metadata SHALL list every credential configuration it issues, with format, proof types and signing algorithms, and its endpoints.

#### Scenario: Metadata lists configurations
- **WHEN** the metadata of an issuer configured for one SD-JWT VC and one Data Integrity credential is fetched
- **THEN** it lists both configurations with their formats and the endpoints the holder needs

### Requirement: OpenID4VP Verifier And Holder
As a verifier, a Vertep agent SHALL create OpenID4VP 1.0 authorisation requests that carry:
- a DCQL query;
- a nonce;
- its client identifier;
- the `direct_post` response mode.

It SHALL verify the response it receives. As a holder, a wallet SHALL answer such a request through its consent step with a presentation that satisfies the query.

#### Scenario: Holder answers a DCQL query
- **WHEN** a verifier requests, through DCQL, an SD-JWT VC of a given type with the claim `nationality`, and the holder's policy allows it
- **THEN** the holder posts a presentation disclosing `nationality` only, and the verifier accepts it

#### Scenario: Unsatisfiable query reported
- **WHEN** a request's DCQL query matches no credential in the wallet
- **THEN** the holder answers with an `access_denied` error, and discloses nothing

#### Scenario: Response for another client refused
- **WHEN** a verifier receives a presentation whose key-binding audience is a different client identifier
- **THEN** it refuses the presentation

### Requirement: OpenID4VC Interoperability
The OpenID4VC plugins SHALL accept the credential offers, metadata, authorisation requests and DCQL queries given as examples in the OpenID4VCI 1.0 and OpenID4VP 1.0 specifications. They SHALL emit messages that those specifications' JSON structures accept.

#### Scenario: Specification examples parse
- **WHEN** the specifications' example offers, metadata documents, requests and DCQL queries are parsed
- **THEN** every example is accepted, and each re-serialises with the same members

### Requirement: OpenID4VC Conformance
The OpenID4VCI and OpenID4VP plugins SHALL map their messages onto the exchange vocabulary, and SHALL pass the conformance kit, so the exchange-model theorems apply to them.

#### Scenario: OpenID4VC histories accepted
- **WHEN** the conformance kit runs both plugins under seeded adversarial schedules
- **THEN** `vertep-wallet` accepts every recorded history
