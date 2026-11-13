# did:webvh 1.0 interop vectors

Copied unchanged from the `tests/test_vectors/test_suite/` directory of the DIF
[`didwebvh-rs`](https://github.com/decentralized-identity/didwebvh-rs) 0.8.2 crate, which imported
them from the DIF [didwebvh-test-suite](https://github.com/decentralized-identity/didwebvh-test-suite)
pull request 4 (upstream commit `7be2491f04102e322197774504ff986c18b1f63e`, path
`vectors/<scenario>/`). Apache License 2.0.

Each directory is one scenario: `did.jsonl`, the expected `resolutionResult.json` for the latest
version, and `did-witness.json` where the scenario has witnesses. `tests/interop.rs` publishes
each log through Vertep and resolves it with Vertep's `did:webvh` resolver.
