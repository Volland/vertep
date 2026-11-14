//! Built-in credential formats (`agent-wallet` requirement `Credential Formats`, design D9):
//! VCDM 2.0 with Data Integrity `eddsa-jcs-2022` and `eddsa-rdfc-2022`, and SD-JWT VC with
//! EdDSA. Every signature is made through the key manager.

pub mod di;
#[cfg(feature = "sd-jwt")]
pub mod sdjwt;

use serde_json::Value;
use vertep_federation::Timestamp;

use crate::credential::StatusRef;
use crate::error::WalletError;

/// The VCDM 2.0 context.
pub const VC_CONTEXT: &str = "https://www.w3.org/ns/credentials/v2";
/// The VCDM 2.0 examples context (bundled for the specification's test vectors).
pub const VC_EXAMPLES_CONTEXT: &str = "https://www.w3.org/ns/credentials/examples/v2";

/// Bundled JSON-LD contexts: the only ones `eddsa-rdfc-2022` loads (design constraint "JSON-LD
/// remote contexts are not loaded").
pub fn bundled_context(url: &str) -> Option<&'static str> {
    match url {
        VC_CONTEXT => Some(include_str!("../../contexts/credentials-v2.jsonld")),
        VC_EXAMPLES_CONTEXT => Some(include_str!(
            "../../contexts/credentials-examples-v2.jsonld"
        )),
        _ => None,
    }
}

pub(crate) fn rfc3339(ms: i64) -> String {
    Timestamp(ms).to_rfc3339()
}

pub(crate) fn parse_time(v: Option<&Value>) -> Result<Option<i64>, WalletError> {
    match v.and_then(Value::as_str) {
        None => Ok(None),
        Some(s) => Timestamp::parse_rfc3339(s)
            .map(|t| Some(t.0))
            .ok_or_else(|| WalletError::Malformed(format!("bad time {s}"))),
    }
}

pub(crate) fn status_json(s: &StatusRef) -> Value {
    serde_json::json!({
        "type": "BitstringStatusListEntry",
        "statusPurpose": s.purpose,
        "statusListIndex": s.index.to_string(),
        "statusListCredential": s.list,
    })
}

pub(crate) fn status_from(v: Option<&Value>) -> Option<StatusRef> {
    let v = v?;
    let v = if let Some(a) = v.as_array() {
        a.first()?
    } else {
        v
    };
    Some(StatusRef {
        list: v.get("statusListCredential")?.as_str()?.to_owned(),
        index: v.get("statusListIndex").and_then(|i| {
            i.as_str()
                .and_then(|s| s.parse().ok())
                .or_else(|| i.as_u64())
        })?,
        purpose: v
            .get("statusPurpose")
            .and_then(Value::as_str)
            .unwrap_or("revocation")
            .to_owned(),
    })
}
