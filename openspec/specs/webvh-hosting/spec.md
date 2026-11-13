# webvh-hosting Specification

## Purpose
Lets operators publish and serve `did:webvh` logs from any web host (a directory behind a web server, a git repository behind GitHub Pages, or a bucket through a command), drive DIDs with the `vt webvh` commands, and run `vt-mediator` under a `did:webvh` of its own.

## Requirements

### Requirement: Published File Layout
A publisher SHALL write a DID's files at the paths the `did:webvh` 1.0 specification derives from the DID:

- `/.well-known/did.jsonl` for a DID without a path, or `/<path>/did.jsonl` for one with a path;
- `did-witness.json` beside the log when the DID has witnesses;
- a `did.json` `did:web` document beside the log.

#### Scenario: Root and path DIDs
- **WHEN** a DID for `example.org` and one for `example.org:objects:ticket-8812` are published to a directory
- **THEN** their logs are at `example.org/.well-known/did.jsonl` and `example.org/objects/ticket-8812/did.jsonl`, each with its `did.json` beside it

### Requirement: Log Publishers
The webvh layer SHALL provide these publishers behind one interface:

- a directory publisher;
- a git publisher, which writes into a working copy, commits and pushes;
- a command publisher, which writes into a directory and then runs a configured command.

A failed publish SHALL be reported with the underlying error, and SHALL NOT be reported as success.

#### Scenario: Git publish commits
- **WHEN** a log is published through the git publisher into a repository with a reachable remote
- **THEN** one commit with the changed files is pushed

#### Scenario: Command failure reported
- **WHEN** the command publisher's command exits non-zero
- **THEN** the publish fails with the command's exit status and error output

### Requirement: Serving Logs
`vt webvh serve` SHALL serve a directory of published DIDs over HTTP. Logs SHALL be served as `text/jsonl`, witness files and `did.json` as `application/json`, with permissive CORS and a cache lifetime no longer than the DID's `ttl`. It SHALL answer HEAD and GET, return 404 for missing files, and never serve files outside the directory.

#### Scenario: Resolver resolves from the server
- **WHEN** a published DID's domain is routed to `vt webvh serve` and the DID is resolved
- **THEN** the resolver fetches `did.jsonl` and `did-witness.json` from the server and verifies them

#### Scenario: Path traversal refused
- **WHEN** a request asks for `/../secret`
- **THEN** the server answers 404 and reads nothing outside the directory

### Requirement: vt webvh Commands
The `vt webvh` command SHALL create, update, rotate, deactivate, verify, witness, publish and serve `did:webvh` DIDs. It SHALL keep keys in a file key manager directory and print the resulting DID and log.

#### Scenario: Create then verify
- **WHEN** `vt webvh create --domain example.org --keys KEYS --out SITE` is followed by `vt webvh verify SITE/example.org/.well-known/did.jsonl`
- **THEN** create prints the new DID, and verify reports the log valid with its version count

#### Scenario: Object with custodians
- **WHEN** `vt webvh object create` is given a domain, a path, custodians with roles and a quorum
- **THEN** the created document declares the `VertepCustody` service, and the log names the custodians as witnesses with the quorum as the threshold

### Requirement: Mediator Under did:webvh
`vt-mediator` SHALL accept `did = "webvh:<host>[:<path>]"`. On first start it SHALL create the DID with a `TSPTransport` endpoint at its public URL, keep the log in its data directory, and serve it at the path the DID determines. On later starts it SHALL keep the same DID.

#### Scenario: Mediator serves its own log
- **WHEN** `vt-mediator` starts with `did = "webvh:mediator.example.org"`
- **THEN** `GET /.well-known/did.jsonl` returns a log that resolves to a document whose `TSPTransport` endpoint is the configured public URL

#### Scenario: Restart keeps the DID
- **WHEN** the mediator restarts with the same data directory
- **THEN** it serves the same DID and log

### Requirement: Third-Party Logs Interoperate
Logs that Vertep writes SHALL verify with the DIF `didwebvh-rs` resolver used on its own, and Vertep SHALL resolve and witness logs other conforming implementations wrote.

#### Scenario: Reference test vector resolves
- **WHEN** a `did:webvh` 1.0 log from the `didwebvh-rs` test vectors is published and resolved through Vertep
- **THEN** it verifies and yields the document the vectors expect
