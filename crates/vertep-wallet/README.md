# vertep-wallet

Agent wallets for [Vertep](https://volland.github.io/vertep/): a wallet per identity that
receives, keeps, presents, issues and verifies verifiable credentials.

- **Protocol-based.** Every exchange is a journaled run of a role chart (issuer, holder,
  verifier); consent is a `vt:decide` state whose candidates the consent policy computes, so no
  decider, not even an LLM, can disclose more than the policy allows.
- **Pluggable.** Exchange protocols, credential formats, credential stores, consent policies and
  status resolvers are traits in a `WalletRegistry`, registered by IRI. Built in: the Vertep
  Credential Exchange over TSP, OpenID4VCI (pre-authorised code) and OpenID4VP (DCQL,
  `direct_post`); VCDM 2.0 with Data Integrity `eddsa-jcs-2022` and `eddsa-rdfc-2022`, and
  SD-JWT VC; a sealed store; rule policies; W3C Bitstring Status List.
- **Verified.** Every protocol maps its messages onto one exchange vocabulary. The Lean model in
  `lean/vertep-lean/VertepLean/Wallet.lean` proves, over any interleaving and a network
  adversary, possession before issuance, one issuance per offer, consent before presentation,
  disclosure within consent, sound acceptance and one acceptance per nonce. The conformance kit
  (`testkit`) records histories under adversarial schedules and the Lean checker
  `vertep-wallet` replays them: a plugin is conformant when every history is accepted.

Credentials are encrypted at rest under a key only the key manager unwraps; the graph and the
journal hold metadata and digests, never claim values or private keys.

```rust
use std::sync::Arc;
use vertep_federation::{InMemoryKeyManager, KeyManager, KeyType};
use vertep_wallet::credential::{DidKeyOnly, IssueRequest, SigningKey};
use vertep_wallet::format::sdjwt::SdJwtVc;
use vertep_wallet::plugin::CredentialFormat;

let km = Arc::new(InMemoryKeyManager::new());
let issuer = SigningKey::did_key(km.as_ref(), &km.generate(KeyType::Ed25519)?)?;
let holder = SigningKey::did_key(km.as_ref(), &km.generate(KeyType::Ed25519)?)?;

// issue an SD-JWT VC bound to the holder's key
let claims = serde_json::json!({"name": "Olena", "nationality": "UA"});
let credential = SdJwtVc.issue(km.as_ref(), &issuer, &IssueRequest {
    types: vec!["https://example.org/IdentityCredential".into()],
    claims: claims.as_object().cloned().unwrap(),
    holder_key: holder.public,
    valid_from: 1_794_355_200_000,
    valid_until: None,
    status: None,
})?;

// present only `nationality` to a verifier, for its nonce
let disclose = ["nationality".to_owned()].into();
let p = SdJwtVc.present(km.as_ref(), &credential, &holder, "did:example:verifier", "n-1", &disclose)?;
let v = SdJwtVc.verify_presentation(&p, &DidKeyOnly)?;
assert_eq!(v.disclosed_names(), disclose);
assert_eq!(v.signer, holder.did_key_id());
# Ok::<(), Box<dyn std::error::Error>>(())
```

The engine (`exchange::Exchanges`), the plugin interfaces (`plugin`), the history contract
(`docs/exchange-history.md`) and `vt wallet` are described on the
[wallet page](https://volland.github.io/vertep/docs/wallet.html).

Licensed MIT or Apache-2.0.
