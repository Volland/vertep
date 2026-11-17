## Why

Commit 2be2d10 ("Mailbox backends chosen per address, from code or a JSON file") added `RoutedMailboxes`, `MailboxConfig`, `NodeBuilder::mailbox_config` and `vt mailbox check`. It was written straight into code, with two scenarios added to `messaging#Pluggable Mailboxes` and no OpenSpec change. The behaviour it fixes is not stated anywhere: which rule wins when several match, what the JSON file may contain, what validation rejects before anything opens, what recovery reports across backends, and what the CLI prints and returns. Code, tests and docs cannot cite requirements that do not exist, and `m18-runtime-performance` extends these routes with `capacity`, `overflow` and `durability`. This change writes the specification for what was built, and lists the tests and links still missing.

## What Changes

- **`messaging`, added requirements:**
  - `Mailbox Routing By Address`: ordered rules by scheme, prefix or exact address; the first match wins; a default serves the rest; backends are memory, any `Storage`, or an application factory; recovery is the union over every backend.
  - `Mailbox Configuration File`: the JSON form of the same routing, its validation without opening anything, the names an application must register, and the errors a build returns.
  - `Offline Mailbox Configuration Check`: `vt mailbox check FILE`, its output and its exit codes.
- No code changes beyond the tests and annotations listed in `tasks.md`. Nothing in the existing requirements changes; the two scenarios already in `Pluggable Mailboxes` stay where they are.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `messaging`: three added requirements.

## Impact

- **Code (already merged in the branch):** `crates/vertep-runtime/src/mailbox.rs` (`RoutedMailboxes`, `AddressMatch`, `MailboxBackend`, `MailboxConfig`, `BackendSpec`, `RouteSpec`, `MatchSpec`), `crates/vertep-runtime/src/node.rs` (`NodeBuilder::mailbox_config`), `crates/vertep-cli/src/commands.rs` (`mailbox_check`), `crates/vertep-cli/src/main.rs`.
- **Tests:** `crates/vertep-runtime/src/mailbox.rs` unit tests, `crates/vertep-runtime/tests/messaging.rs`, `crates/vertep-cli/tests/cli.rs`; two tests are missing (see tasks).
- **API:** none new.
- **lat.md sections touched:**
  - [[decisions#Mailbox backend is chosen per address]] (exists; the decision this change specifies);
  - [[components#vertep-runtime]] and [[components#vertep-cli]] (the CLI paragraph and its `realizes::` edge were filed under the wrong components by 2be2d10);
  - [[views/runtime#Runtime View#Identity, mailbox and transport]] (gains a routing subsection);
  - [[capabilities#Messaging]].
- **Binding decisions and constraints:**
  - [[decisions#Identity, mailbox and transport layers]] motivates the per-address choice;
  - [[constraints#Tiramemsu has a single writer per file]]: a mailbox directory cannot be the journal's directory opened a second time.
- **Requirements added:**
  - `[[openspec:messaging#Mailbox Routing By Address]]`
  - `[[openspec:messaging#Mailbox Configuration File]]`
  - `[[openspec:messaging#Offline Mailbox Configuration Check]]`
- **Follow-up:** `m18-runtime-performance` D8 adds `capacity` and `overflow` to a route, and D12 adds `durability: by_origin`; both extend `Mailbox Configuration File`.
