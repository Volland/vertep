//! The Vertep Credential Exchange 1.0 (design D10): exchange messages as JSON typed under
//! `https://w3id.org/vertep/exchange/1.0#`, carried by any [`crate::exchange::Wire`]: TSP
//! relationships (direct, routed or through a mediator; feature `tsp`) or the conformance kit's
//! network. Nonces and audiences travel inside the signed proofs and presentations, never only
//! in this unsigned envelope.

use serde_json::{Value, json};

use crate::error::WalletError;
use crate::message::{Body, ExchangeMessage};
use crate::plugin::ExchangeProtocol;
use crate::vocab::{EXCHANGE_NS, PROTOCOL_VCE};

/// The Vertep Credential Exchange 1.0.
// @tg: implements:: [[openspec:credential-exchange#Native Exchange Over TSP]] {confidence: partial, note: "the codec; the TSP wire is tsp::TspWire"}
#[derive(Clone, Copy, Debug, Default)]
pub struct VertepExchange;

const TYPES: [&str; 7] = [
    "Offer",
    "CredentialRequest",
    "Credential",
    "PresentationRequest",
    "Presentation",
    "Result",
    "Problem",
];

fn type_name(b: &Body) -> &'static str {
    match b {
        Body::Offer { .. } => "Offer",
        Body::CredentialRequest { .. } => "CredentialRequest",
        Body::Credential { .. } => "Credential",
        Body::PresentationRequest { .. } => "PresentationRequest",
        Body::Presentation { .. } => "Presentation",
        Body::Result { .. } => "Result",
        Body::Problem { .. } => "Problem",
    }
}

fn kebab(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

impl ExchangeProtocol for VertepExchange {
    fn iri(&self) -> &str {
        PROTOCOL_VCE
    }

    fn encode(&self, m: &ExchangeMessage) -> Result<Vec<u8>, WalletError> {
        let mut body =
            serde_json::to_value(&m.body).map_err(|e| WalletError::Protocol(e.to_string()))?;
        if let Some(o) = body.as_object_mut() {
            o.remove("type");
        }
        serde_json::to_vec(&json!({
            "type": format!("{EXCHANGE_NS}{}", type_name(&m.body)),
            "exchange": m.exchange,
            "from": m.from,
            "to": m.to,
            "body": body,
        }))
        .map_err(|e| WalletError::Protocol(e.to_string()))
    }

    fn decode(&self, bytes: &[u8]) -> Result<ExchangeMessage, WalletError> {
        let v: Value = serde_json::from_slice(bytes)
            .map_err(|e| WalletError::Malformed(format!("exchange message: {e}")))?;
        let ty = v
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| WalletError::Unmapped("message without type".into()))?;
        let name = ty
            .strip_prefix(EXCHANGE_NS)
            .filter(|n| TYPES.contains(n))
            .ok_or_else(|| WalletError::Unmapped(ty.to_owned()))?;
        let mut body = v.get("body").cloned().unwrap_or_else(|| json!({}));
        if let Some(o) = body.as_object_mut() {
            o.insert("type".into(), Value::from(kebab(name)));
        }
        let body: Body = serde_json::from_value(body)
            .map_err(|e| WalletError::Malformed(format!("{name}: {e}")))?;
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| WalletError::Malformed(format!("message without {k}")))
        };
        Ok(ExchangeMessage {
            exchange: s("exchange")?,
            from: s("from")?,
            to: s("to")?,
            body,
        })
    }
}
