## ADDED Requirements

### Requirement: Mailbox Routing By Address
The runtime SHALL provide a mailbox factory that chooses a backend for each address from an ordered list of rules, each matching by address scheme, by address prefix or by exact address, using the first rule that matches and a default backend for every other address. A backend SHALL be an in-memory mailbox, a mailbox stored in any storage implementation (not necessarily the journal's), or a factory the application supplies. Recovery SHALL report every address that any backend holds messages for, each once.

#### Scenario: First matching rule wins
- **WHEN** an exact rule for `urn:t:special` routes to memory and a later prefix rule for `urn:t:` routes to a store
- **THEN** a message for `urn:t:special` is held in memory and a message for `urn:t:a` is held in the store

#### Scenario: Unmatched address uses the default
- **WHEN** no rule matches an address
- **THEN** its mailbox is created by the default backend

#### Scenario: Recovery spans every backend
- **WHEN** two routes lead to two different stores and each holds a message for a different address, and the same address is reported by both
- **THEN** recovery lists each of those addresses exactly once, in order

#### Scenario: Application factory behind a route
- **WHEN** a route names a factory the application supplies, such as a relay inbox
- **THEN** mailboxes for the matching addresses are created by that factory and receive its messages

### Requirement: Mailbox Configuration File
The runtime SHALL accept the routing as a JSON document with an optional `default` backend, which means memory when omitted, and an ordered list of `routes`, each a `match` (`scheme`, `prefix` or `exact`) and a `backend` (`memory`, `tiramemsu` with `dir` and `shards`, or `named`). Validation SHALL open nothing, SHALL reject a tiramemsu backend without a directory or with zero shards, an empty name and an empty match, and SHALL return the names the application must register in order of first use. Building SHALL reject a name the application did not register, a tiramemsu backend in a build without tiramemsu support, and a store that cannot be opened. A node builder SHALL accept the document with the registered factories.

#### Scenario: Omitted default is memory
- **WHEN** a document has routes and no `default`
- **THEN** the default backend is memory

#### Scenario: Validation lists the names to register
- **WHEN** two routes both name the backend `tsp`
- **THEN** validation succeeds and returns `tsp` once

#### Scenario: Invalid backend rejected before anything opens
- **WHEN** a document has a tiramemsu backend with zero shards, or with an empty directory, or a route with an empty match or an empty name
- **THEN** validation fails naming the default or the route number, and building fails without creating any directory

#### Scenario: Malformed document rejected
- **WHEN** the text is not JSON of this shape, such as a number as the default
- **THEN** parsing fails with a mailbox configuration error

#### Scenario: Tiramemsu route without tiramemsu support
- **WHEN** a build without the `tiramemsu` feature builds a document with a tiramemsu backend
- **THEN** building fails, saying the backend needs the feature

### Requirement: Offline Mailbox Configuration Check
The `vt mailbox check FILE` command SHALL parse and validate a mailbox configuration without opening any store, and SHALL print the default backend, each route in order with its match and backend, and the names the application must register. It SHALL exit with 0 for a valid file, 1 for an invalid one, and 2 when the file cannot be read.

#### Scenario: Valid file is listed
- **WHEN** a file routes scheme `did` to the named backend `tsp` and prefix `vt://orders/` to a tiramemsu directory `d` with 4 shards
- **THEN** the command prints `default -> memory`, `scheme did -> named tsp`, `prefix vt://orders/ -> tiramemsu d (4 shards)` and `register before build: tsp`, and exits 0

#### Scenario: Invalid file exits 1
- **WHEN** the file has a tiramemsu backend with zero shards
- **THEN** the command prints the problem and exits 1

#### Scenario: Unreadable file exits 2
- **WHEN** the file does not exist
- **THEN** the command exits 2
