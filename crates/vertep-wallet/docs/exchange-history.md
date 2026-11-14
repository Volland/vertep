# Exchange histories

The contract between the Rust conformance kit (`vertep_wallet::testkit`), which records exchange
histories, and the Lean checker `vertep-wallet` (`lean/vertep-lean/VertepLean/WalletCheck.lean`),
which replays them through the exchange model (`VertepLean/Wallet.lean`) in strict mode.

A history is JSON Lines. Line 1 is the setup; every other line is one action, in the order it
took effect. Every action must be enabled in the model state reached by the lines before it; the
checker reports the first line that is not, with its 1-based line number and the reason.

Claim lists are sets of claim **names** (strings), compared as sets. Histories never carry claim
values, private keys or credential bytes: credential ids are digests (`sha256:<hex>`), keys are
key ids (`did:key:z…` or a key-manager key name), agents are DIDs.

## Setup (line 1)

```json
{"kind":"setup",
 "agents":[{"id":"did:example:issuer","keys":["k-issuer"]},
           {"id":"did:example:holder","keys":["k-holder"]},
           {"id":"did:example:verifier","keys":["k-verifier"]}],
 "trust":[{"verifier":"did:example:verifier","issuer":"did:example:issuer"}],
 "compromised":[]}
```

- `agents`: every honest agent and the keys it owns. A key belongs to at most one agent.
- `trust`: which issuers each verifier trusts.
- `compromised`: keys the adversary can sign with (they may also belong to an agent).

## Message terms

Every message on the network has an id (`"msg":"m17"`, unique in the history) and a term:

| Term | JSON | Signed by |
| --- | --- | --- |
| offer | `{"t":"offer","issuer":I,"nonce":N}` | nobody |
| possession proof | `{"t":"pop","key":K,"issuer":I,"nonce":N}` | `K` |
| credential | `{"t":"cred","issuer":I,"ikey":IK,"id":C,"key":K,"claims":[…]}` | `IK` |
| presentation request | `{"t":"req","verifier":V,"nonce":N,"claims":[…]}` | nobody |
| presentation | `{"t":"pres","key":K,"verifier":V,"nonce":N,"cred":<credential term>,"claims":[…]}` | `K` |
| other | `{"t":"other"}` | nobody (results, problems, refusals) |

A credential term binds credential `C` with claims `claims` to holder key `K`. A presentation
term is signed by `K` over the verifier, the verifier's nonce, the embedded credential term and
the disclosed claim names.

## Honest steps

Honest agents only take these steps; `msg` names the message the step sends, and every step
that sends one is enabled only when `msg` is new (no earlier message has that id).

| `kind` | Fields | Enabled when | Effect |
| --- | --- | --- | --- |
| `offer` | `issuer`, `to`, `nonce`, `msg` | `issuer` is an agent; `nonce` was never used before by anyone | sends `offer(issuer, nonce)`; records the offer |
| `prove-possession` | `holder`, `key`, `issuer`, `nonce`, `msg` | `key` is the holder's; a message with term `offer(issuer, nonce)` was delivered to `holder` | sends `pop(key, issuer, nonce)` |
| `issue` | `issuer`, `ikey`, `credential`, `key`, `nonce`, `claims`, `msg` | `ikey` is the issuer's; the issuer offered `nonce`; nothing was issued on `nonce` yet; a message with term `pop(key, issuer, nonce)` was delivered to the issuer; `credential` was never issued before | sends `cred(issuer, ikey, credential, key, claims)`; records the issuance |
| `store` | `holder`, `credential` | a message whose term is a credential with id `credential` and a key of `holder` was delivered to `holder` | records that `holder` stores that credential term |
| `request-presentation` | `verifier`, `to`, `nonce`, `claims`, `msg` | `nonce` was never used before by anyone | sends `req(verifier, nonce, claims)`; records the request |
| `consent` | `holder`, `verifier`, `nonce`, `claims` | a message `req(verifier, nonce, R)` was delivered to `holder`, and `claims ⊆ R` | records the consent |
| `present` | `holder`, `verifier`, `nonce`, `credential`, `claims`, `msg` | `holder` consented to `(verifier, nonce, C')` with `claims ⊆ C'`; `holder` stores a credential term with id `credential`, bound to its key `K`, whose claims include `claims` | sends `pres(K, verifier, nonce, <that credential term>, claims)`; records the presentation |
| `verify` | `verifier`, `msg`, `result` (`accept` or `refuse`), optional `reason` | `msg` exists and was delivered to `verifier`; for `accept`, see below | for `accept`, records the acceptance |
| `revoke` | `issuer`, `credential` | the issuer issued `credential` | records the revocation |
| `send` | `from`, `msg` | always | sends `other` |

When several credential terms match a `store` (or `present`) line, the step is about the first
one delivered (or stored).

`verify` with `"result":"accept"` is enabled only when the term of `msg` is
`pres(K, V, N, cred(I, IK, C, K', CC), D)` and all of these hold:

- `V` is the verifying agent;
- `V` requested `N` (`req(V, N, R)` recorded) and has not accepted any presentation for `N`;
- `K = K'` (holder binding);
- `V` trusts `I`, and `IK` is a key of `I`;
- `C` is not revoked;
- `D ⊆ R` and `D ⊆ CC`.

`verify` with `"result":"refuse"` is always enabled for a delivered message: refusing is never
unsafe.

## Network events

The adversary controls the network.

| `kind` | Fields | Enabled when | Effect |
| --- | --- | --- | --- |
| `deliver` | `msg`, `to` | `msg` exists | `to` has received `msg` (any number of times, to anyone) |
| `drop` | `msg` | `msg` exists | none |
| `duplicate` | `msg` | `msg` exists | none (a later `deliver` is the duplicate) |
| `inject` | `msg`, `term` | `msg` is new and `term` is derivable | sends `term` as `msg` |

A term is **derivable** from the adversary's knowledge (every term of every existing message,
with the credential terms embedded in presentations) when:

- it is an `offer`, `req` or `other` (unsigned: anyone can build one);
- it is a `pop` or `pres` signed by a compromised key, or exactly equal to a known term;
- it is a `cred` signed by a compromised key, or exactly equal to a known term;
- and, for a `pres`, its embedded credential term is derivable.

## Checker

```
vertep-wallet --history HISTORY.jsonl [HISTORY.jsonl …]
```

Prints `ok: <file>: <n> lines` for each accepted history and exits 0, or prints
`error: <file>:<line>: <kind>: <reason>` for the first failing line and exits 1. `<n>` counts
every non-blank line, the setup included; `<kind>` is the failing line's `kind` (`setup` for
line 1, `parse` for a line that is not JSON). `ok` lines go to standard output, `error` lines to
standard error; every file given is checked. A usage error exits 2.
