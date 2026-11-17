## Purpose

Defines what an agent protocol is: a role-based choreography written as a restricted JSON-LD document, identified by the hash of its graph, projected onto each role, offered by signed statements and discovered through the DID document.

## ADDED Requirements

### Requirement: Protocol Document
A protocol SHALL be one JSON-LD document in the `vk:` profile: a frozen, versioned context embedded in the crate and never fetched. The document SHALL declare roles, message types with payload shapes, states, and transitions, where each transition names the sending role, the message type and the target state. A document that carries or references another context, or uses a term outside the profile, SHALL be rejected with an error naming the term.

#### Scenario: Valid document accepted
- **WHEN** a document in the `vk:` profile declares two roles, two message types and a three-state choreography
- **THEN** it parses into a protocol model with those roles, messages and transitions

#### Scenario: Foreign context refused
- **WHEN** a document lists a remote context URL in `@context`
- **THEN** it is rejected, no network request is made, and the error names the context

#### Scenario: Unknown term refused
- **WHEN** a document uses a term that the `vk:` context does not define
- **THEN** it is rejected with an error naming the term

### Requirement: Protocol Identity
The identity of a protocol SHALL be `urn:vertep:protocol:sha256:` followed by the hex SHA-256 of the RDFC-1.0 canonical N-Quads of its expanded graph, which includes the version of the `vk:` context. Two documents with the same graph SHALL have the same identity, whatever their JSON-LD spelling.

#### Scenario: Spellings share an identity
- **WHEN** the same protocol is written with its members and the elements of its sets in a different order
- **THEN** both have the same identity

#### Scenario: Any change changes the identity
- **WHEN** one transition's target state is changed
- **THEN** the identity differs

### Requirement: Payload Shapes
A message type's payload SHALL be described by a SHACL node shape in a subset of SHACL: `sh:datatype`, `sh:minCount`, `sh:maxCount`, `sh:in`, and nested node shapes. A shape outside the subset SHALL be rejected when the document is parsed. The checker's own interpretation of the subset SHALL give the same verdict as full SHACL validation in `vertep-rdf` on every conformance vector.

#### Scenario: Payload validated
- **WHEN** a payload has a required field of the declared datatype
- **THEN** the shape accepts it

#### Scenario: Missing required field
- **WHEN** a payload lacks a field with `sh:minCount 1`
- **THEN** the shape rejects it, naming the field

#### Scenario: Shape outside the subset
- **WHEN** a document uses `sh:sparql` in a payload shape
- **THEN** the document is rejected

### Requirement: Well-Formed Choreography
A protocol SHALL be accepted only when it is well formed:
- every non-final state's outgoing transitions have one single sending role, so one role decides at each state;
- a transition's sender is never also its receiver;
- every state can reach a final state;
- the only constructs are sequence, choice by one role, loops, and timeout or cancel expressed as ordinary messages.

Parallel regions that span roles, and choices decided by different roles, SHALL be rejected with the state and the reason.

#### Scenario: Two deciders refused
- **WHEN** one state has outgoing transitions sent by two different roles
- **THEN** the protocol is rejected, naming the state and both roles

#### Scenario: Unreachable end refused
- **WHEN** a state has no path to a final state
- **THEN** the protocol is rejected, naming the state

### Requirement: Role Projection
For each role, the crate SHALL compute a projection deterministically from the document: an automaton whose steps are the role's sends and receives, with every transition the role does not take part in treated as silent. The projection SHALL also give an interface view: for each state, the message types the role accepts and the roles that may send them.

#### Scenario: Projection of a request and reply
- **WHEN** a protocol has `buyer` send `quote.request` and `seller` reply with `quote`
- **THEN** the seller's projection accepts `quote.request` from `buyer` first and then sends `quote`, and the buyer's interface view lists `quote` from `seller` after the request

#### Scenario: Projection is deterministic
- **WHEN** the same document is projected twice, in any order of roles
- **THEN** each role's projection is identical

### Requirement: Protocol Offers
A statement that a DID offers a protocol in a role SHALL be a separate document signed by that DID: the protocol identity, the role, an optional expiry and an optional list of accepted initiators. The statement SHALL not be part of the protocol document and SHALL not change its identity.

#### Scenario: Offer verified
- **WHEN** an offer signed by the DID's key is checked against the DID document
- **THEN** it is valid for that DID, protocol and role

#### Scenario: Expired offer refused
- **WHEN** an offer past its expiry is checked
- **THEN** it is refused

### Requirement: Protocol Discovery
A DID document SHALL be able to list offered protocols in a service entry of type `VertepProtocol` giving each protocol identity and the roles played. A peer SHALL fetch the protocol document by its identity from any source, verify the hash locally, and trust nothing else about the source. An agent MAY also answer a query message with the protocols it accepts.

#### Scenario: Discovery by service entry
- **WHEN** a resolver returns a DID document with a `VertepProtocol` service entry
- **THEN** the peer learns the protocol identities and roles without contacting the agent

#### Scenario: Wrong document refused
- **WHEN** a source returns a document whose hash is not the listed identity
- **THEN** the peer refuses it

### Requirement: RDF Bridge
The crate SHALL import a verified protocol document into a graph, with the offers that name it, so that questions such as which DIDs offer a protocol in a role are answered with SPARQL. The bridge lives in `vertep-kolyada`, because `vertep-rdf` cannot depend on it. The bridge SHALL be one-way: the identity SHALL never be computed from anything but the document.

#### Scenario: Who offers a role
- **WHEN** three agents' offers for one protocol are imported
- **THEN** a SPARQL query for the role returns the three DIDs
