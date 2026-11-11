//! Actor addresses and the stable IRIs of runtime records.
//!
//! Every actor has exactly one address, an absolute IRI, which is also the RDF subject of its
//! state and the name of its current-state graph. Journal entries, snapshots, timers and dead
//! letters get IRIs derived from the address and a journal position or send id, so that
//! replaying or re-projecting a journal yields the same IRIs.

use core::fmt;

use oxrdf::NamedNode;
use sha2::{Digest, Sha256};

/// Why a string is not an actor address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddressError {
    /// The rejected text.
    pub text: String,
    /// The reason.
    pub reason: String,
}

impl fmt::Display for AddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not an actor address: {:?} ({})", self.text, self.reason)
    }
}

impl std::error::Error for AddressError {}

/// An actor address: an absolute IRI.
// @tg: implements:: [[openspec:actor-model#IRI Actor Addresses]]
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Address(NamedNode);

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}>", self.0.as_str())
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl Address {
    /// Parses an address. Relative references and strings that are not IRIs are rejected.
    ///
    /// # Errors
    ///
    /// [`AddressError`] for anything but an absolute IRI.
    pub fn parse(text: &str) -> Result<Self, AddressError> {
        NamedNode::new(text).map(Address).map_err(|e| AddressError {
            text: text.to_owned(),
            reason: e.to_string(),
        })
    }

    /// The address of the virtual actor of chart type `chart_type` with key `key`:
    /// `<chart_type>/<percent-encoded key>`. The same pair always gives the same address.
    ///
    /// # Errors
    ///
    /// [`AddressError`] when `chart_type` is not an absolute IRI.
    // @tg: implements:: [[openspec:actor-model#Virtual Actor Identity]]
    pub fn virtual_actor(chart_type: &str, key: &str) -> Result<Self, AddressError> {
        Address::parse(chart_type)?;
        let mut s = String::with_capacity(chart_type.len() + key.len() + 1);
        s.push_str(chart_type);
        s.push('/');
        percent_encode_into(key, &mut s);
        Address::parse(&s)
    }

    /// The address of the child invoked by `parent` with invoke id `invokeid`:
    /// `<parent>/invoke/<percent-encoded id>`.
    pub fn invoked(parent: &Address, invokeid: &str) -> Address {
        Address(parent.derived("invoke", invokeid))
    }

    /// The IRI text.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// The IRI as an RDF named node.
    pub fn node(&self) -> &NamedNode {
        &self.0
    }

    /// The IRI scheme, such as `urn` or `https`.
    pub fn scheme(&self) -> &str {
        self.as_str().split(':').next().unwrap_or_default()
    }

    /// `<self>/<kind>/<percent-encoded part>`.
    fn derived(&self, kind: &str, part: &str) -> NamedNode {
        let mut s = String::with_capacity(self.as_str().len() + kind.len() + part.len() + 2);
        s.push_str(self.as_str());
        s.push('/');
        s.push_str(kind);
        s.push('/');
        percent_encode_into(part, &mut s);
        NamedNode::new_unchecked(s)
    }

    /// The IRI of the journal entry at `position`, also the name of its graph.
    // @tg: implements:: [[openspec:runtime-ontology#Stable Runtime IRIs]]
    pub fn entry_iri(&self, position: u64) -> NamedNode {
        self.derived("entry", &position.to_string())
    }

    /// The IRI of the snapshot at `position`, also the name of its graph.
    pub fn snapshot_iri(&self, position: u64) -> NamedNode {
        self.derived("snapshot", &position.to_string())
    }

    /// The IRI of the dead letter for message `message_id`.
    pub fn dead_letter_iri(&self, message_id: &str) -> NamedNode {
        self.derived("dead-letter", &short_digest(message_id))
    }

    /// The graph holding this actor's dead letters.
    pub fn dead_letter_graph(&self) -> NamedNode {
        NamedNode::new_unchecked(format!("{}/dead-letters", self.as_str()))
    }

    /// The graph holding this actor's store-backed mailbox.
    pub fn mailbox_graph(&self) -> NamedNode {
        NamedNode::new_unchecked(format!("{}/mailbox", self.as_str()))
    }

    /// The IRI of the mailbox row with sequence number `seq`.
    pub fn mailbox_row_iri(&self, seq: u64) -> NamedNode {
        self.derived("mailbox", &seq.to_string())
    }

    /// The graph holding this actor's outbox marks.
    pub fn outbox_graph(&self) -> NamedNode {
        NamedNode::new_unchecked(format!("{}/outbox", self.as_str()))
    }

    /// The correlation id of the `k`-th request issued by the macrostep journaled at
    /// `position`: `<self>/request/<position>.<k>`.
    // @tg: implements:: [[openspec:messaging#Deterministic Correlation Ids]]
    pub fn correlation_id(&self, position: u64, k: usize) -> String {
        format!("{}/request/{position}.{k}", self.as_str())
    }
}

/// The IRI of the input event of entry `entry`.
pub fn input_iri(entry: &NamedNode) -> NamedNode {
    NamedNode::new_unchecked(format!("{}/input", entry.as_str()))
}

/// The IRI of the `k`-th send of entry `entry`; also the message id the send is delivered with.
pub fn send_iri(entry: &NamedNode, k: usize) -> NamedNode {
    NamedNode::new_unchecked(format!("{}/send/{k}", entry.as_str()))
}

/// The IRI of the timer with send id `send_id` scheduled by entry `entry`; also the message id
/// its event is delivered with.
pub fn timer_iri(entry: &NamedNode, send_id: &str) -> NamedNode {
    let mut s = format!("{}/timer/", entry.as_str());
    percent_encode_into(send_id, &mut s);
    NamedNode::new_unchecked(s)
}

/// Percent-encodes everything but RFC 3986 unreserved characters.
pub fn percent_encode_into(text: &str, out: &mut String) {
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(b >> 4)]));
            out.push(char::from(HEX[usize::from(b & 0xF)]));
        }
    }
}

/// Decodes `%XX` escapes; `None` when the result is not UTF-8 or an escape is malformed.
pub fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// The first 16 bytes of the SHA-256 of `text`, as lowercase hex.
pub fn short_digest(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes())[..16])
}

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    const L: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from(L[usize::from(b >> 4)]));
        s.push(char::from(L[usize::from(b & 0xF)]));
    }
    s
}

/// The random seed of the macrostep journaled as `entry`: the first 8 bytes of the SHA-256 of
/// its IRI, so replay reproduces every random value without storing it.
pub fn seed_of(entry: &NamedNode) -> u64 {
    let d = Sha256::digest(entry.as_str().as_bytes());
    let mut b = [0u8; 8];
    b.copy_from_slice(&d[..8]);
    u64::from_be_bytes(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_round_trip() {
        let mut s = String::new();
        percent_encode_into("a b/ü#", &mut s);
        assert_eq!(s, "a%20b%2F%C3%BC%23");
        assert_eq!(percent_decode(&s).unwrap(), "a b/ü#");
    }

    #[test]
    fn derived_iris_are_stable_and_distinct() {
        let a = Address::parse("urn:vertep:test:a").unwrap();
        assert_eq!(a.entry_iri(3).as_str(), "urn:vertep:test:a/entry/3");
        assert_ne!(a.entry_iri(3), a.snapshot_iri(3));
        let e = a.entry_iri(3);
        assert_eq!(
            timer_iri(&e, "r 1").as_str(),
            "urn:vertep:test:a/entry/3/timer/r%201"
        );
        assert_eq!(seed_of(&e), seed_of(&a.entry_iri(3)));
        assert_ne!(seed_of(&e), seed_of(&a.entry_iri(4)));
        assert_eq!(a.scheme(), "urn");
    }
}
