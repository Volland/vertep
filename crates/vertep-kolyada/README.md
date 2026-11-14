# vertep-kolyada

Kolyada: verifiable role-based agent protocols for [Vertep](https://github.com/Volland/vertep).

A protocol is one JSON-LD document that names roles, message types and a choreography. Its
identity is the hash of its graph, every role runs a projection of it, and messages are signed
before any transport seals them, so a peer that trusts nobody can still check them.

```rust
use vertep_kolyada::Protocol;

let doc = r#"{
  "@context": "https://w3id.org/vertep/kolyada/1.0",
  "type": "Protocol",
  "version": "1.0",
  "roles": ["asker", "answerer"],
  "messages": [{ "id": "ask" }, { "id": "answer" }],
  "initial": "Start",
  "final": ["End"],
  "transitions": [
    { "from": "Start", "sender": "asker", "receivers": ["answerer"], "message": "ask", "to": "Wait" },
    { "from": "Wait", "sender": "answerer", "receivers": ["asker"], "message": "answer", "to": "End" }
  ]
}"#;

let protocol = Protocol::parse(doc.as_bytes()).unwrap();
assert!(protocol.id().as_str().starts_with("urn:vertep:protocol:sha256:"));

// What the answerer accepts, state by state.
let interface = protocol.interface("answerer").unwrap();
assert_eq!(interface.accepts["Start"], [("ask".to_owned(), "asker".to_owned())]);
```

The crate has no keys, transport or runtime. Signing and verification enter through the `Signer`
and `Verifier` traits, and `vertep-federation` implements them over the key manager and the DID
resolver. Test vectors for other implementations are in `vectors/`.
