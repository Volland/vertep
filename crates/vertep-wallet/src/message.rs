//! Exchange messages, independent of any wire protocol: what a protocol codec decodes wire
//! messages into and encodes them from. Each body maps onto one vocabulary step, or onto no
//! step for results and problems.

use serde::{Deserialize, Serialize};

use crate::history::Claims;
use crate::vocab::Step;

/// A message between two parties of one exchange.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeMessage {
    /// The exchange id, chosen by the party that starts the exchange.
    pub exchange: String,
    /// The sender's DID.
    pub from: String,
    /// The recipient's DID.
    pub to: String,
    /// The content.
    pub body: Body,
}

/// What a message carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Body {
    /// An issuer offers credentials of `types` in `format`, with its nonce.
    Offer {
        /// The issuer.
        issuer: String,
        /// The issuer's nonce.
        nonce: String,
        /// The credential types offered.
        types: Vec<String>,
        /// The format IRI.
        format: String,
    },
    /// A holder asks for the credential, proving possession of its key over the nonce: a
    /// compact JWS (`typ` `openid4vci-proof+jwt`) with `aud` the issuer and `nonce`.
    CredentialRequest {
        /// The possession proof.
        proof: String,
    },
    /// The issued credential.
    Credential {
        /// The format IRI.
        format: String,
        /// The credential's serialisation.
        credential: String,
    },
    /// A verifier asks for a presentation.
    PresentationRequest {
        /// The verifier (the audience).
        verifier: String,
        /// The verifier's nonce.
        nonce: String,
        /// Requested claim names.
        claims: Claims,
        /// Acceptable credential types (empty: any).
        #[serde(default)]
        types: Vec<String>,
        /// Why the verifier asks.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        purpose: Option<String>,
    },
    /// A presentation.
    Presentation {
        /// The format IRI.
        format: String,
        /// The presentation's serialisation.
        presentation: String,
    },
    /// The outcome of a request or presentation.
    Result {
        /// Whether it was accepted.
        accepted: bool,
        /// Why not.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// A refusal or error.
    Problem {
        /// Why.
        reason: String,
    },
}

impl Body {
    /// The vocabulary step the message's sending is, if any (`Result` and `Problem` map onto
    /// no step: they are `other` messages in histories).
    pub fn step(&self) -> Option<Step> {
        match self {
            Body::Offer { .. } => Some(Step::Offer),
            Body::CredentialRequest { .. } => Some(Step::ProvePossession),
            Body::Credential { .. } => Some(Step::Issue),
            Body::PresentationRequest { .. } => Some(Step::RequestPresentation),
            Body::Presentation { .. } => Some(Step::Present),
            Body::Result { .. } | Body::Problem { .. } => None,
        }
    }
}
