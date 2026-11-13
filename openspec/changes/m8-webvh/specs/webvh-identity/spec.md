## Purpose

Lets Vertep create and maintain `did:webvh` DIDs with its own keys, documents and custodians: a hash-chained, signed DID log whose entries are signed through the key manager, witnessed by a durable object's custodians, and verified before they are published.

## ADDED Requirements

### Requirement: Create With Vertep Keys
The webvh layer SHALL create a `did:webvh` DID, consisting of a genesis log entry with its SCID and DID document, for a domain and an optional path. Its update keys SHALL be held by a key manager, and every signature SHALL be made through the key manager, so no private key material reaches the log, the files or the caller.

#### Scenario: Created DID resolves
- **WHEN** a DID is created for `example.org` and path `objects:ticket-8812` and its log is given to the `did:webvh` resolver
- **THEN** the resolver verifies the log and returns the created DID document

#### Scenario: Keys stay in the key manager
- **WHEN** a DID is created and its files are written
- **THEN** no file and no returned value contains private key material, and the update key in the log is the key manager's public key

### Requirement: Document From An Identity
The webvh layer SHALL build the DID document of a new DID from a Vertep identity: its authentication, assertion and key-agreement methods, and its service entries (`TSPTransport`, `DIDCommMessaging`, `VertepCustody`).

#### Scenario: Services carried into the document
- **WHEN** a DID is created from an identity with a `TSPTransport` endpoint and a `VertepCustody` declaration
- **THEN** the resolved document has both services, and `CustodyConfig::from_document` reads the declared custodians and quorum

### Requirement: Update And Rotate
The webvh layer SHALL append log entries that replace the DID document or rotate the update keys, signed by a current update key. With pre-rotation enabled, a rotation SHALL be accepted only for keys whose hashes the previous entry announced.

#### Scenario: Update keeps the DID
- **WHEN** a DID's document is updated with a new service endpoint
- **THEN** the log has one more entry, the DID is unchanged, and resolution returns the new document

#### Scenario: Rotation with pre-rotation
- **WHEN** a DID created with pre-rotation rotates to the key whose hash it announced
- **THEN** the new entry verifies, and the old update key can no longer sign entries

#### Scenario: Unannounced key refused
- **WHEN** a DID with pre-rotation tries to rotate to a key whose hash was not announced
- **THEN** the rotation fails, and the log is unchanged

### Requirement: Deactivate
The webvh layer SHALL deactivate a DID with a final signed log entry, after which no further entry is accepted.

#### Scenario: Deactivated DID
- **WHEN** a DID is deactivated and resolved
- **THEN** resolution reports it deactivated, and a later update fails

### Requirement: Custodians Witness Object Logs
The DID log of a durable object SHALL name the object's custodians as `did:webvh` witnesses, each by the `did:key` of its Ed25519 signing key, with the custody quorum as the threshold. An entry SHALL be published only with witness proofs from at least the threshold.

#### Scenario: Witnessed by a quorum
- **WHEN** an object with five custodians and quorum three updates its DID document and three custodians witness the entry
- **THEN** the entry is published with three proofs, and the resolver accepts it

#### Scenario: Too few witnesses blocks publishing
- **WHEN** only two of five custodians witness an entry
- **THEN** the entry is not published, the published log is unchanged, and the caller is told how many proofs are missing

### Requirement: Custodian Witnessing Rules
A custodian SHALL witness a log entry of its object only when all of the following hold:

- it is a valid next entry of the object's log, signed by a current update key;
- the custodian set and quorum the entry's `VertepCustody` service declares equal the effective or pending configuration of the custody record the custodian has accepted.

#### Scenario: Valid entry witnessed
- **WHEN** the writer asks a custodian to witness an entry that declares the configuration the custodian accepted
- **THEN** the custodian returns a witness proof for the entry's version id

#### Scenario: Unaccepted custodian set refused
- **WHEN** an entry declares a custodian set that differs from every configuration the custodian has accepted
- **THEN** the custodian refuses to witness it, with the reason

#### Scenario: Forged entry refused
- **WHEN** an entry is not signed by a current update key of the object's DID
- **THEN** the custodian refuses to witness it

### Requirement: Custody Changes Publish As Log Entries
A controller-signed change of an object's custodian set SHALL carry the `did:webvh` log entry that declares the new set. After the custody switch, the writer SHALL collect witness proofs and publish that entry through the object's log publisher. Resolvers SHALL then see the new set.

#### Scenario: Replace a keeper end to end
- **WHEN** the controller replaces a keeper with the webvh tooling and the writer completes the change
- **THEN** the published log's last entry declares the new set with witness proofs from a quorum, and resolving the object's DID yields the new custodians

### Requirement: Verify Before Publish
Every publisher SHALL verify the complete log with its witness proofs through the `did:webvh` resolver before writing it, and SHALL refuse a log that does not verify.

#### Scenario: Broken log refused
- **WHEN** a log whose last entry has a bad signature is handed to a publisher
- **THEN** nothing is written, and the error names the failing entry
