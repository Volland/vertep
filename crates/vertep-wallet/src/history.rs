//! Exchange histories: the JSON Lines the conformance kit records and the Lean checker
//! `vertep-wallet` replays (`docs/exchange-history.md` is the contract).
//!
//! A history holds vocabulary steps and network events, with claim names, key ids and
//! credential digests; never claim values, private keys or credential bytes.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// A set of claim names.
pub type Claims = BTreeSet<String>;

/// An agent in the setup line: a DID and the key ids it owns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentKeys {
    /// The agent's DID.
    pub id: String,
    /// The key ids it owns.
    pub keys: Vec<String>,
}

/// A verifier trusting an issuer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trust {
    /// The verifier.
    pub verifier: String,
    /// The issuer it trusts.
    pub issuer: String,
}

/// A credential term.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredTerm {
    /// The issuer.
    pub issuer: String,
    /// The issuer's signing key.
    pub ikey: String,
    /// The credential digest.
    pub id: String,
    /// The holder key the credential is bound to.
    pub key: String,
    /// The credential's claim names.
    pub claims: Claims,
}

/// The term of a message on the network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Term {
    /// An offer with the issuer's nonce.
    Offer {
        /// The issuer.
        issuer: String,
        /// The offer nonce.
        nonce: String,
    },
    /// A possession proof, signed by `key`.
    Pop {
        /// The holder key.
        key: String,
        /// The issuer it is for.
        issuer: String,
        /// The issuer's nonce.
        nonce: String,
    },
    /// A credential, signed by `ikey`.
    Cred(CredTerm),
    /// A presentation request.
    Req {
        /// The verifier.
        verifier: String,
        /// The verifier's nonce.
        nonce: String,
        /// Requested claim names.
        claims: Claims,
    },
    /// A presentation, signed by `key`.
    Pres {
        /// The holder key.
        key: String,
        /// The audience.
        verifier: String,
        /// The verifier's nonce.
        nonce: String,
        /// The embedded credential.
        cred: CredTerm,
        /// Disclosed claim names.
        claims: Claims,
    },
    /// Anything else: results, problems, refusals, garbage.
    Other,
}

/// One line of a history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Line {
    /// Line 1.
    Setup {
        /// Honest agents and their keys.
        agents: Vec<AgentKeys>,
        /// Trust relation.
        trust: Vec<Trust>,
        /// Compromised keys.
        compromised: Vec<String>,
    },
    /// An issuer offers.
    Offer {
        /// The issuer.
        issuer: String,
        /// The holder.
        to: String,
        /// The fresh nonce.
        nonce: String,
        /// The message sent.
        msg: String,
    },
    /// A holder proves possession.
    ProvePossession {
        /// The holder.
        holder: String,
        /// Its key.
        key: String,
        /// The issuer.
        issuer: String,
        /// The issuer's nonce.
        nonce: String,
        /// The message sent.
        msg: String,
    },
    /// An issuer issues.
    Issue {
        /// The issuer.
        issuer: String,
        /// Its signing key.
        ikey: String,
        /// The credential digest.
        credential: String,
        /// The holder key.
        key: String,
        /// The offer nonce.
        nonce: String,
        /// Claim names.
        claims: Claims,
        /// The message sent.
        msg: String,
    },
    /// A holder stores a credential.
    Store {
        /// The holder.
        holder: String,
        /// The credential digest.
        credential: String,
    },
    /// A verifier requests a presentation.
    RequestPresentation {
        /// The verifier.
        verifier: String,
        /// The holder.
        to: String,
        /// The fresh nonce.
        nonce: String,
        /// Requested claim names.
        claims: Claims,
        /// The message sent.
        msg: String,
    },
    /// A holder consents.
    Consent {
        /// The holder.
        holder: String,
        /// The verifier.
        verifier: String,
        /// The verifier's nonce.
        nonce: String,
        /// Consented claim names.
        claims: Claims,
    },
    /// A holder presents.
    Present {
        /// The holder.
        holder: String,
        /// The verifier.
        verifier: String,
        /// The verifier's nonce.
        nonce: String,
        /// The credential digest.
        credential: String,
        /// Disclosed claim names.
        claims: Claims,
        /// The message sent.
        msg: String,
    },
    /// A verifier accepts or refuses a delivered message.
    Verify {
        /// The verifier.
        verifier: String,
        /// The message judged.
        msg: String,
        /// `accept` or `refuse`.
        result: String,
        /// Why, for refusals.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// An issuer revokes.
    Revoke {
        /// The issuer.
        issuer: String,
        /// The credential digest.
        credential: String,
    },
    /// Any other message (results, problems, refusals).
    Send {
        /// The sender.
        from: String,
        /// The message.
        msg: String,
    },
    /// The network delivers a message.
    Deliver {
        /// The message.
        msg: String,
        /// The recipient.
        to: String,
    },
    /// The network drops a message.
    Drop {
        /// The message.
        msg: String,
    },
    /// The network duplicates a message.
    Duplicate {
        /// The message.
        msg: String,
    },
    /// The adversary injects a message.
    Inject {
        /// The new message id.
        msg: String,
        /// Its term.
        term: Term,
    },
}

impl Line {
    /// The line's `kind`.
    pub fn kind(&self) -> &'static str {
        match self {
            Line::Setup { .. } => "setup",
            Line::Offer { .. } => "offer",
            Line::ProvePossession { .. } => "prove-possession",
            Line::Issue { .. } => "issue",
            Line::Store { .. } => "store",
            Line::RequestPresentation { .. } => "request-presentation",
            Line::Consent { .. } => "consent",
            Line::Present { .. } => "present",
            Line::Verify { .. } => "verify",
            Line::Revoke { .. } => "revoke",
            Line::Send { .. } => "send",
            Line::Deliver { .. } => "deliver",
            Line::Drop { .. } => "drop",
            Line::Duplicate { .. } => "duplicate",
            Line::Inject { .. } => "inject",
        }
    }
}

/// A shared, append-only history.
#[derive(Clone, Debug, Default)]
pub struct History {
    lines: Arc<Mutex<Vec<Line>>>,
}

impl History {
    /// An empty history.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends `line`.
    pub fn push(&self, line: Line) {
        if let Ok(mut l) = self.lines.lock() {
            l.push(line);
        }
    }

    /// Every line so far.
    pub fn lines(&self) -> Vec<Line> {
        self.lines.lock().map(|l| l.clone()).unwrap_or_default()
    }

    /// The history as JSON Lines.
    pub fn to_jsonl(&self) -> String {
        let mut out = String::new();
        for l in self.lines() {
            if let Ok(s) = serde_json::to_string(&l) {
                out.push_str(&s);
                out.push('\n');
            }
        }
        out
    }
}
