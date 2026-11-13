## ADDED Requirements

### Requirement: Grant Credentials From A Wallet
A grant credential SHALL be holdable in a wallet like any other credential. Its grantee SHALL be able to present it to the target's verifier over any conformant exchange protocol. A presented grant credential SHALL yield its grant only after it is verified, including its status, and the grant SHALL be enforced before any journal append, exactly like a configured grant.

#### Scenario: Grant presented over an exchange
- **WHEN** an issuer the target trusts issues a grant credential to an agent's wallet, and the agent presents it to the target over the native TSP exchange
- **THEN** the target holds the grant, and the agent's granted events are delivered

#### Scenario: Presented grant enforced before the journal
- **WHEN** the same agent then sends an event outside the presented grant
- **THEN** the event is refused at the mailbox, and nothing is appended to the target's journal

#### Scenario: Revoked grant credential refused
- **WHEN** the issuer revokes the grant credential, and the agent presents it again
- **THEN** verification fails, the target holds no grant from it, and the agent's events are refused as ungranted
