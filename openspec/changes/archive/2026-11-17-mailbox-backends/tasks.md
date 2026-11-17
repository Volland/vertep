The code landed in commit 2be2d10 before this specification. Section 1 records what was built; the open tasks close the gaps between it and the requirements.

## 1. Built in 2be2d10

- [x] 1.1 `RoutedMailboxes`, `AddressMatch` (scheme, prefix, exact) and `MailboxBackend` (`Memory`, `Store`, `Custom`, `tiramemsu(dir, shards)`) in `vertep-runtime/src/mailbox.rs`
- [x] 1.2 `MailboxConfig`, `BackendSpec`, `RouteSpec`, `MatchSpec` with `from_json`, `validate` and `build`
- [x] 1.3 `NodeBuilder::mailbox_config`
- [x] 1.4 `vt mailbox check FILE` in `vertep-cli`
- [x] 1.5 Tests: `config_builds_routes_and_rejects_unknown_names`, `validate_names_what_to_register_and_rejects_bad_specs`, `routes_pick_backend_per_address_first_match_wins` (unit); `routed_tiramemsu_mailbox_survives_a_restart_and_memory_does_not`, `node_builder_takes_mailboxes_from_a_config` (integration); `mailbox_check_lists_routes_and_names_and_rejects_bad_files` (e2e)
- [x] 1.6 Decision [[decisions#Mailbox backend is chosen per address]], CHANGELOG entry, site pages (runtime, distribution, CLI)

## 2. Missing tests

- [x] 2.1 Unit test: two store routes holding messages for overlapping addresses; `recover` lists each address once, sorted (`Mailbox Routing By Address#Recovery spans every backend`)
- [x] 2.2 Unit test: an address no rule matches is created by the default backend (`#Unmatched address uses the default`)
- [x] 2.3 Unit test, compiled only without the `tiramemsu` feature: building a tiramemsu backend fails naming the feature (`Mailbox Configuration File#Tiramemsu route without tiramemsu support`)
- [x] 2.4 Extend `validate_names_what_to_register_and_rejects_bad_specs` to assert that a failed build creates no directory (`#Invalid backend rejected before anything opens`)

## 3. Annotations

- [x] 3.1 `// @tg: implements:: [[openspec:messaging#Mailbox Routing By Address]]` above `RoutedMailboxes`; `// @tg: implements:: [[openspec:messaging#Mailbox Configuration File]]` above `MailboxConfig` and `NodeBuilder::mailbox_config`; `// @tg: implements:: [[openspec:messaging#Offline Mailbox Configuration Check]]` above `mailbox_check`
- [x] 3.2 `// @tg: verifies::` above each existing test for its scenario: `routes_pick_backend_per_address_first_match_wins` → `#First matching rule wins` {level: unit}; `config_builds_routes_and_rejects_unknown_names` → `#Application factory behind a route`, `#Omitted default is memory` and `#Malformed document rejected` {level: unit}; `validate_names_what_to_register_and_rejects_bad_specs` → `#Validation lists the names to register` and `#Invalid backend rejected before anything opens` {level: unit}; `mailbox_check_lists_routes_and_names_and_rejects_bad_files` → the three `Offline Mailbox Configuration Check` scenarios {level: e2e}
- [x] 3.3 `// @tg: verifies::` above the tests from section 2 for their scenarios

## 4. Docs

- [x] 4.1 `tg.md/components.md`: move the `vt mailbox check` paragraph and its `realizes::` edge into the `vertep-cli` section (2be2d10 filed them under the facade and the examples)
- [x] 4.2 `tg.md/views/runtime.md`: a subsection on mailbox routing with a diagram and `describes::` edges to the decision and the capability; add `messaging` to the view's `openspec:` frontmatter if missing
- [x] 4.3 `tg.md/capabilities.md` `Messaging`: name the routing factory and the configuration file
- [x] 4.4 After archive, check that `openspec/specs/messaging/spec.md` holds the three requirements and that the annotations from section 3 resolve

## 5. Check

- [x] 5.1 Run `tg check`, `tg ontology check` and `tg trace messaging --gaps`, and close every gap this change reports
