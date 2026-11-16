//! `vt:plays`: the agent protocols a chart plays, and in which role.
//!
//! A chart declares each protocol role it plays as one token of the `vt:plays` extension
//! attribute on `<scxml>`: the protocol identity, `#`, and the role, with tokens separated by
//! whitespace:
//!
//! ```xml
//! <scxml xmlns:vt="https://w3id.org/vertep/vt#"
//!        vt:plays="urn:vertep:protocol:sha256:4579…7bc6#seller">
//! ```
//!
//! The declaration rides on an ordinary [`Extension`] node, so every surface (SCXML, YAML, RDF)
//! keeps it losslessly and the step kernel never sees it. The agent's `VertepProtocol` DID
//! service entry is generated from it, and the mailbox runs the protocol check for the roles it
//! names (`vertep-federation`, module `role_binding`).

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use crate::ir::{Chart, ExtensionValue, VT_NS};

/// Local name of the `vt:plays` attribute on `<scxml>`.
pub const PLAYS_ATTR: &str = "plays";

/// The prefix of a protocol identity.
pub const PROTOCOL_PREFIX: &str = "urn:vertep:protocol:sha256:";

/// One role of one protocol a chart plays.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RolePlay {
    /// The protocol identity, `urn:vertep:protocol:sha256:` and 64 lowercase hex digits.
    pub protocol: String,
    /// The role.
    pub role: String,
}

impl RolePlay {
    /// Reads one `<protocol>#<role>` token.
    ///
    /// # Errors
    ///
    /// [`PlaysError`] naming the token and what is wrong with it.
    pub fn parse(token: &str) -> Result<Self, PlaysError> {
        let bad = |reason: &'static str| PlaysError {
            token: token.to_string(),
            reason,
        };
        let (protocol, role) = token.split_once('#').ok_or(bad("no #role"))?;
        let hex = protocol
            .strip_prefix(PROTOCOL_PREFIX)
            .ok_or(bad("not a protocol identity"))?;
        if hex.len() != 64 || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(bad("the hash is not 64 lowercase hex digits"));
        }
        if role.is_empty() || role.contains('#') {
            return Err(bad("the role is empty or contains #"));
        }
        Ok(RolePlay {
            protocol: protocol.to_string(),
            role: role.to_string(),
        })
    }
}

impl fmt::Display for RolePlay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.protocol, self.role)
    }
}

/// A `vt:plays` token that is not `<protocol identity>#<role>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaysError {
    /// The token.
    pub token: String,
    /// What is wrong with it.
    pub reason: &'static str,
}

impl fmt::Display for PlaysError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "vt:plays token {:?}: {}", self.token, self.reason)
    }
}

/// The text of the chart's `vt:plays` attribute, if it has one.
#[must_use]
pub fn plays_attribute(chart: &Chart) -> Option<&str> {
    chart.extensions.iter().find_map(|e| match &e.value {
        ExtensionValue::Attribute(v) if e.ns == VT_NS && e.local == PLAYS_ATTR => Some(v.as_str()),
        _ => None,
    })
}

/// The protocol roles the chart plays, sorted and without duplicates; empty when it declares
/// none.
///
/// # Errors
///
/// [`PlaysError`] for the first malformed token.
// @tg: implements:: [[openspec:protocol-spaces#Chart Role Binding]] {confidence: partial, note: "the vt:plays declaration"}
pub fn plays(chart: &Chart) -> Result<Vec<RolePlay>, PlaysError> {
    let mut out = plays_attribute(chart)
        .unwrap_or_default()
        .split_whitespace()
        .map(RolePlay::parse)
        .collect::<Result<Vec<_>, _>>()?;
    out.sort();
    out.dedup();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Extension, Span};

    const ID: &str = "urn:vertep:protocol:sha256:45790502561699ae0a09e8e6c58096fcf6de7eca7bb242f3c0ca7b329e287bc6";

    fn chart(value: &str) -> Chart {
        Chart {
            extensions: alloc::vec![Extension {
                ns: VT_NS.into(),
                local: PLAYS_ATTR.into(),
                value: ExtensionValue::Attribute(value.into()),
                position: 0,
                span: Span::NONE,
            }],
            ..Chart::default()
        }
    }

    #[test]
    fn reads_sorted_roles() {
        let c = chart(&alloc::format!("{ID}#seller\n  {ID}#auditor {ID}#seller"));
        let p = plays(&c).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].role, "auditor");
        assert_eq!(p[1].to_string(), alloc::format!("{ID}#seller"));
        assert!(plays(&Chart::default()).unwrap().is_empty());
    }

    #[test]
    fn rejects_malformed_tokens() {
        for bad in [
            "seller",
            "urn:other:x#seller",
            "urn:vertep:protocol:sha256:abc#seller",
        ] {
            assert!(plays(&chart(bad)).is_err(), "{bad}");
        }
        assert!(plays(&chart(&alloc::format!("{ID}#"))).is_err());
    }
}
