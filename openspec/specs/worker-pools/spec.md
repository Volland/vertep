# worker-pools Specification

## Purpose
Runs stateless workers across a cluster: many activations behind one address, balanced across nodes, with no journal of their own, for LLM calls, tools and heavy computation.

## Requirements

### Requirement: Stateless Worker Activations
A stateless worker address SHALL be served by zero or more interchangeable activations, created on demand, and any activation MUST be able to handle any message sent to the address.

#### Scenario: Activations created on demand
- **WHEN** messages arrive for a worker address that has no activation
- **THEN** an activation is created and handles them

#### Scenario: Any activation can serve a request
- **WHEN** two requests to the same worker address are handled by different activations
- **THEN** both receive the same kind of reply as if a single activation had handled them

### Requirement: Pool Placement Across Nodes
Worker activations SHALL be placeable on any active cluster member that declares it can run that worker type, independently of shard ownership.

#### Scenario: Workers spread across nodes
- **WHEN** a worker type is declared on three nodes and load grows
- **THEN** activations of that worker are created on more than one of the three nodes

#### Scenario: Node without the worker type is not used
- **WHEN** a node does not declare a worker type
- **THEN** no activation of that worker type is placed on it

### Requirement: Load Balancing Policies
The system SHALL balance messages for a worker address across its activations by a policy configured per worker type, providing at least round robin, least outstanding requests and prefer local node.

#### Scenario: Least outstanding requests
- **WHEN** one activation has five requests in progress and another has one, under the least outstanding policy
- **THEN** the next request goes to the activation with one request in progress

#### Scenario: Prefer local node
- **WHEN** the sender's node runs an activation with free capacity, under the prefer local policy
- **THEN** the request is handled on the sender's node without a network hop

### Requirement: Worker Concurrency Limits
Each worker type SHALL have a configured maximum number of activations per node and a maximum number of requests in progress per activation, and requests beyond those limits MUST wait in the pool queue up to its bound.

#### Scenario: Limit reached
- **WHEN** every activation of a worker type is at its request limit and its per-node activation limit is reached
- **THEN** further requests wait in the pool queue until capacity is free

#### Scenario: Queue bound reached
- **WHEN** the pool queue is full
- **THEN** a new request is refused at once and the caller receives an error event

### Requirement: No Journal For Workers
Stateless workers SHALL keep no journal and no durable state, and their activations MUST be discardable at any time without loss of committed actor state.

#### Scenario: Activation discarded under memory pressure
- **WHEN** an idle worker activation is discarded
- **THEN** no journal entry or state is lost, and the next request creates or uses another activation

#### Scenario: Worker writes nothing to the journal store
- **WHEN** a worker handles a request
- **THEN** no entry is appended to any journal on behalf of the worker

### Requirement: Results Journaled By The Caller
The result of a worker request made by a virtual actor SHALL reach the caller as an input event that is recorded in the caller's journal, so that replay of the caller never repeats the worker call.

#### Scenario: Replay does not call the worker
- **WHEN** a virtual actor that received a worker result is deactivated and replayed
- **THEN** the replayed state uses the recorded result and the worker is not called again

#### Scenario: Duplicate result discarded
- **WHEN** a worker result is delivered twice because of a retry
- **THEN** the caller's journal records it once, matched by correlation identifier

### Requirement: Worker Failure Reporting
A worker request that cannot complete because the worker fails, its node fails or its deadline passes SHALL be reported to the caller as an error event or a request timeout, and the system MUST NOT silently retry a request on another activation unless the worker type is declared idempotent.

#### Scenario: Worker node crashes
- **WHEN** the node running the activation that handles a request crashes
- **THEN** the caller receives an error event or its request times out, according to its request deadline

#### Scenario: Idempotent worker is retried
- **WHEN** a request to a worker type declared idempotent fails because its activation crashed
- **THEN** the request is retried once on another activation before an error is reported
