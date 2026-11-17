## Context

Before 2be2d10 a node took one `MailboxFactory`, plus one factory per address scheme (`NodeBuilder::mailboxes_for_scheme`). An application that wanted some actors' mailboxes durable, others in memory and a third group held by a relay mediator had to write its own factory. The commit added a routing factory and a JSON form of it. This design records the choices the commit made, so the requirements in this change can be checked against them.

## Goals / Non-Goals

**Goals:**

- One node mixes mailbox backends per address, with no change to charts.
- The same routing can be written as code or as a JSON file, and a file can be checked before deployment.
- Recovery after a restart finds the messages of every durable backend.

**Non-Goals:**

- Loading backend implementations from the file (plugins). A file names a backend; the application registers it.
- Capacity, overflow and durability per route. Those come with `m18-runtime-performance` (D8, D12).
- Moving messages between backends when the routing changes. A message stays in the backend it was enqueued into.

## Decisions

### D1. First matching rule wins

`RoutedMailboxes` tries its rules in the order they were added and uses the first that matches; the default serves every address no rule matches. A rule matches by IRI scheme, by string prefix, or by exact address.

- motivated_by:: [[decisions#Identity, mailbox and transport layers]] {weight: primary}

Rejected: the most specific match (exact over prefix over scheme, longest prefix first). It is harder to read in a file and needs tie rules; with first-match an exact exception is written above the prefix it narrows, as the unit test `routes_pick_backend_per_address_first_match_wins` does.

### D2. A backend is a factory; storage need not be the journal's

`MailboxBackend` is `Memory`, `Store(Arc<dyn Storage>)` or `Custom(Arc<dyn MailboxFactory>)`. `Store` takes any storage: a tiramemsu directory (`MailboxBackend::tiramemsu(dir, shards)`), a memory store, or another implementation. A relay inbox is a factory, so a route to it makes a mediator, including a remote TSP one, hold those actors' messages.

- motivated_by:: [[decisions#Mailbox backend is chosen per address]] {weight: primary}

Rejected: an in-memory tiramemsu backend. `MemoryStore` is already the in-memory choice.

### D3. The file names backends; the application registers them

`MailboxConfig` is `{default, routes: [{match, backend}]}` with backends `memory`, `tiramemsu {dir, shards}` and `named`. A `named` backend is looked up in a map the application passes to `build` (or `NodeBuilder::mailbox_config`). Building fails on a name that is not in the map, on a tiramemsu backend when the `tiramemsu` feature is off, and on a store that cannot be opened. `default` may be omitted and means memory.

Rejected: a registry of backend kinds inside the runtime. It would pull every relay crate into `vertep-runtime`.

### D4. Validation opens nothing

`MailboxConfig::validate` checks shape only: a tiramemsu backend has a directory and at least one shard, a name is non-empty, a match is non-empty. It returns the names the application must register, in order of first use. `vt mailbox check` runs it and prints the result. The CLI never builds a node, so it cannot open stores or resolve names, and says so by listing the names instead.

Rejected: opening the stores in the check. It would create directories and take the single tiramemsu writer of a deployment that might be running.

## Constraints

- [[constraints#Tiramemsu has a single writer per file]] binds D2: a route's tiramemsu directory must not be the journal's directory or another route's, since a second open of the same file fails.

## Risks / Trade-offs

- **A rule order mistake routes durable actors to memory.** → `vt mailbox check` prints the routes in order, so a review sees the order that will apply.
- **Changing the routing strands messages in the old backend.** → Recovery still lists them only if the old backend is still routed; the operator drains before rerouting. Documented, not automated.
- **The two scenarios written into `Pluggable Mailboxes` overlap the new requirements.** → They stay, because tests cite them; the new requirements carry the full rules.

## Migration Plan

None: the code is in the branch, and existing nodes built without a configuration keep their single factory.

## Open Questions

- Should `vt mailbox check` warn when two tiramemsu backends name the same directory? Proposed as a follow-up; it is a validation rule, not a new requirement.
