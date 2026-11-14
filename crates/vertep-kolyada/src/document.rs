//! The protocol document: a JSON-LD document in the `vk:` profile, its identity and its
//! well-formedness rules.
//!
//! The profile is fixed. `@context` must be [`KOLYADA_CONTEXT`] (served from the copy bundled in
//! this crate, never fetched), the only members are the ones the context defines, and nested
//! nodes carry no `type`. The identity is the SHA-256 of the RDFC-1.0 canonical N-Quads of the
//! document's expanded graph, so two JSON-LD spellings of one protocol have one identity.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use oxrdf::Graph;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::shape::{Shape, ShapeDecl, ShapeError};

/// The context IRI of the profile, version 1.0.
pub const KOLYADA_CONTEXT: &str = "https://w3id.org/vertep/kolyada/1.0";

/// The bundled copy of the context.
pub const KOLYADA_CONTEXT_JSON: &str = include_str!("../ontology/kolyada-v1.jsonld");

/// The profile version a document must declare.
pub const PROFILE_VERSION: &str = "1.0";

/// The prefix of protocol identities.
pub const ID_PREFIX: &str = "urn:vertep:protocol:sha256:";

/// The prefix that message ids must not use: it belongs to the control messages.
pub const RESERVED_PREFIX: &str = "kolyada:";

/// The default number of messages a receiver holds while waiting for dependencies.
pub const DEFAULT_BUFFER_SIZE: u32 = 64;

/// The default time a held message waits for its dependencies, in milliseconds.
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// A protocol identity: `urn:vertep:protocol:sha256:` and the hex hash of the document's graph.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolId(String);

impl ProtocolId {
    /// The identity of a digest.
    #[must_use]
    pub fn from_digest(digest: &[u8; 32]) -> Self {
        let mut s = String::from(ID_PREFIX);
        for b in digest {
            s.push_str(&format!("{b:02x}"));
        }
        Self(s)
    }

    /// Parses an identity; `None` unless it is the prefix and 64 lowercase hex digits.
    #[must_use]
    pub fn parse(iri: &str) -> Option<Self> {
        let hex = iri.strip_prefix(ID_PREFIX)?;
        (hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
            .then(|| Self(iri.to_owned()))
    }

    /// The identity as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProtocolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The document as written. Unknown members are an error.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    /// The context: [`KOLYADA_CONTEXT`] or an array holding only it.
    #[serde(rename = "@context")]
    pub context: Value,
    /// `Protocol`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The profile version, `1.0`.
    pub version: String,
    /// The role names.
    pub roles: Vec<String>,
    /// The message types.
    pub messages: Vec<MessageDecl>,
    /// The start state.
    pub initial: String,
    /// The final states.
    #[serde(rename = "final")]
    pub finals: Vec<String>,
    /// The transitions.
    pub transitions: Vec<TransitionDecl>,
    /// The delivery limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<LimitsDecl>,
}

/// A message type as written.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageDecl {
    /// The message type name.
    pub id: String,
    /// The payload shape; the payload must be empty without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<ShapeDecl>,
}

/// A transition as written.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionDecl {
    /// The state it leaves.
    pub from: String,
    /// The role that sends.
    pub sender: String,
    /// The roles that receive.
    pub receivers: Vec<String>,
    /// The message type.
    pub message: String,
    /// The state it enters.
    pub to: String,
}

/// The limits as written.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsDecl {
    /// The causal-delivery buffer size.
    #[serde(
        default,
        rename = "bufferSize",
        skip_serializing_if = "Option::is_none"
    )]
    pub buffer_size: Option<u32>,
    /// The causal-delivery timeout in milliseconds.
    #[serde(default, rename = "timeoutMs", skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

/// Delivery limits a protocol declares, so both sides know them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// How many messages a receiver holds while waiting for dependencies.
    pub buffer_size: u32,
    /// How long a held message waits, in milliseconds.
    pub timeout_ms: u64,
}

/// A message type of a checked protocol.
#[derive(Clone, Debug, PartialEq)]
pub struct MessageType {
    /// The name.
    pub id: String,
    /// The payload shape, if declared.
    pub shape: Option<Shape>,
}

/// A transition of a checked protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    /// The state it leaves.
    pub from: String,
    /// The sending role.
    pub sender: String,
    /// The receiving roles.
    pub receivers: BTreeSet<String>,
    /// The message type.
    pub message: String,
    /// The state it enters.
    pub to: String,
}

/// Why a protocol is rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtocolError {
    /// The bytes are not a JSON document of the profile's shape.
    Json(String),
    /// `@context` is not exactly [`KOLYADA_CONTEXT`]; the text names what was found.
    Context(String),
    /// A member the profile does not define.
    UnknownTerm(String),
    /// The profile version is not `1.0`.
    Version(String),
    /// A payload shape is outside the subset.
    Shape(ShapeError),
    /// The document does not expand to a graph.
    Graph(String),
    /// The choreography is not well formed.
    WellFormed(WellFormedError),
}

/// Why a choreography is not well formed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WellFormedError {
    /// The state concerned, if one is.
    pub state: Option<String>,
    /// What is wrong.
    pub reason: String,
}

impl fmt::Display for WellFormedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.state {
            Some(s) => write!(f, "state `{s}`: {}", self.reason),
            None => f.write_str(&self.reason),
        }
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(m) => write!(f, "not a protocol document: {m}"),
            Self::Context(m) => write!(f, "context refused: {m}"),
            Self::UnknownTerm(t) => write!(f, "term `{t}` is not in the vk: profile"),
            Self::Version(v) => write!(f, "profile version `{v}` is not {PROFILE_VERSION}"),
            Self::Shape(e) => write!(f, "payload shape outside the subset: {e}"),
            Self::Graph(m) => write!(f, "document does not expand: {m}"),
            Self::WellFormed(e) => write!(f, "not well formed: {e}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

/// A parsed, well-formed protocol with its identity.
#[derive(Clone, Debug, PartialEq)]
pub struct Protocol {
    id: ProtocolId,
    roles: Vec<String>,
    messages: BTreeMap<String, MessageType>,
    initial: String,
    finals: BTreeSet<String>,
    transitions: Vec<Transition>,
    limits: Limits,
    graph: Arc<Graph>,
}

fn wf(state: Option<&str>, reason: impl Into<String>) -> ProtocolError {
    ProtocolError::WellFormed(WellFormedError {
        state: state.map(str::to_owned),
        reason: reason.into(),
    })
}

/// Reads a JSON-LD document into a graph. Only the bundled kolyada context is served; any other
/// remote document is refused without a request.
fn expand(json: &[u8]) -> Result<Graph, ProtocolError> {
    let parser = oxjsonld::JsonLdParser::new()
        .for_slice(json)
        .with_load_document_callback(|url, _opts| {
            if url == KOLYADA_CONTEXT {
                Ok(oxjsonld::JsonLdRemoteDocument {
                    document: KOLYADA_CONTEXT_JSON.as_bytes().to_vec(),
                    document_url: url.to_owned(),
                })
            } else {
                Err(format!("remote JSON-LD document {url} is not allowed").into())
            }
        });
    let mut g = Graph::new();
    for quad in parser {
        let q = quad.map_err(|e| ProtocolError::Graph(e.to_string()))?;
        g.insert(q.as_ref());
    }
    Ok(g)
}

fn check_context(ctx: &Value) -> Result<(), ProtocolError> {
    match ctx {
        Value::String(s) if s == KOLYADA_CONTEXT => Ok(()),
        Value::Array(items)
            if items.len() == 1 && items[0] == Value::String(KOLYADA_CONTEXT.into()) =>
        {
            Ok(())
        }
        Value::String(s) => Err(ProtocolError::Context(s.clone())),
        Value::Array(items) => Err(ProtocolError::Context(
            items
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        )),
        other => Err(ProtocolError::Context(format!("an inline context {other}"))),
    }
}

impl Protocol {
    /// Parses a protocol document.
    ///
    /// # Errors
    ///
    /// [`ProtocolError`] naming the context, term, shape or state at fault.
    // @tg: implements:: [[openspec:agent-protocols#Protocol Document]]
    // @tg: implements:: [[openspec:agent-protocols#Protocol Identity]]
    // @tg: implements:: [[openspec:agent-protocols#Well-Formed Choreography]]
    pub fn parse(json: &[u8]) -> Result<Self, ProtocolError> {
        // The context is checked on the raw value first, so a foreign context is named even
        // when the rest of the document would also be refused.
        let raw: Value =
            serde_json::from_slice(json).map_err(|e| ProtocolError::Json(e.to_string()))?;
        if let Some(ctx) = raw.get("@context") {
            check_context(ctx)?;
        } else {
            return Err(ProtocolError::Context("missing @context".into()));
        }
        let doc: Document = serde_json::from_value(raw).map_err(|e| {
            let m = e.to_string();
            match m
                .strip_prefix("unknown field `")
                .and_then(|r| r.split('`').next())
            {
                Some(term) => ProtocolError::UnknownTerm(term.to_owned()),
                None => ProtocolError::Json(m),
            }
        })?;
        if doc.kind != "Protocol" {
            return Err(ProtocolError::Json(format!(
                "type is `{}`, not `Protocol`",
                doc.kind
            )));
        }
        if doc.version != PROFILE_VERSION {
            return Err(ProtocolError::Version(doc.version));
        }
        let graph = expand(json)?;
        let nquads = vertep_rdf::canon::canonical_nquads_of_graph(&graph)
            .map_err(|e| ProtocolError::Graph(e.to_string()))?;
        let id = ProtocolId::from_digest(&Sha256::digest(nquads.as_bytes()).into());
        Self::build(id, doc, graph)
    }

    fn build(id: ProtocolId, doc: Document, graph: Graph) -> Result<Self, ProtocolError> {
        if doc.roles.is_empty() {
            return Err(wf(None, "no roles"));
        }
        let mut role_set = BTreeSet::new();
        for r in &doc.roles {
            if r.is_empty() || !role_set.insert(r.clone()) {
                return Err(wf(None, format!("role `{r}` is empty or declared twice")));
            }
        }
        let mut messages = BTreeMap::new();
        for m in &doc.messages {
            if m.id.is_empty() || m.id.starts_with(RESERVED_PREFIX) {
                return Err(wf(
                    None,
                    format!("message id `{}` is empty or uses the reserved prefix", m.id),
                ));
            }
            let shape = m
                .payload
                .as_ref()
                .map(Shape::from_decl)
                .transpose()
                .map_err(ProtocolError::Shape)?;
            if messages
                .insert(
                    m.id.clone(),
                    MessageType {
                        id: m.id.clone(),
                        shape,
                    },
                )
                .is_some()
            {
                return Err(wf(None, format!("message `{}` is declared twice", m.id)));
            }
        }
        let finals: BTreeSet<String> = doc.finals.iter().cloned().collect();
        if finals.is_empty() {
            return Err(wf(None, "no final state"));
        }
        let mut transitions = Vec::new();
        let mut states: BTreeSet<String> = finals.clone();
        states.insert(doc.initial.clone());
        let mut keys = BTreeSet::new();
        for t in &doc.transitions {
            if !role_set.contains(&t.sender) {
                return Err(wf(
                    Some(&t.from),
                    format!("sender `{}` is not a role", t.sender),
                ));
            }
            let receivers: BTreeSet<String> = t.receivers.iter().cloned().collect();
            if receivers.is_empty() {
                return Err(wf(
                    Some(&t.from),
                    format!("`{}` has no receiver", t.message),
                ));
            }
            if let Some(bad) = receivers.iter().find(|r| !role_set.contains(*r)) {
                return Err(wf(Some(&t.from), format!("receiver `{bad}` is not a role")));
            }
            if receivers.contains(&t.sender) {
                return Err(wf(
                    Some(&t.from),
                    format!("`{}` sends `{}` to itself", t.sender, t.message),
                ));
            }
            if !messages.contains_key(&t.message) {
                return Err(wf(
                    Some(&t.from),
                    format!("message `{}` is not declared", t.message),
                ));
            }
            if finals.contains(&t.from) {
                return Err(wf(
                    Some(&t.from),
                    "a final state has an outgoing transition",
                ));
            }
            if !keys.insert((t.from.clone(), t.message.clone())) {
                return Err(wf(
                    Some(&t.from),
                    format!("two transitions send `{}`", t.message),
                ));
            }
            states.insert(t.from.clone());
            states.insert(t.to.clone());
            transitions.push(Transition {
                from: t.from.clone(),
                sender: t.sender.clone(),
                receivers,
                message: t.message.clone(),
                to: t.to.clone(),
            });
        }
        // One role decides at every state.
        let mut decider: BTreeMap<&str, &str> = BTreeMap::new();
        for t in &transitions {
            match decider.get(t.from.as_str()) {
                Some(prev) if *prev != t.sender => {
                    return Err(wf(
                        Some(&t.from),
                        format!(
                            "two deciders: `{prev}` and `{}` both send from this state",
                            t.sender
                        ),
                    ));
                }
                _ => {
                    decider.insert(&t.from, &t.sender);
                }
            }
        }
        // Every state reaches a final state.
        let mut live: BTreeSet<&str> = finals.iter().map(String::as_str).collect();
        loop {
            let before = live.len();
            for t in &transitions {
                if live.contains(t.to.as_str()) {
                    live.insert(&t.from);
                }
            }
            if live.len() == before {
                break;
            }
        }
        if let Some(dead) = states.iter().find(|s| !live.contains(s.as_str())) {
            return Err(wf(Some(dead), "no path to a final state"));
        }
        let limits = match &doc.limits {
            None => Limits {
                buffer_size: DEFAULT_BUFFER_SIZE,
                timeout_ms: DEFAULT_TIMEOUT_MS,
            },
            Some(l) => Limits {
                buffer_size: l.buffer_size.unwrap_or(DEFAULT_BUFFER_SIZE),
                timeout_ms: l.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
            },
        };
        if limits.buffer_size == 0 || limits.timeout_ms == 0 {
            return Err(wf(None, "limits must be positive"));
        }
        Ok(Self {
            id,
            roles: doc.roles,
            messages,
            initial: doc.initial,
            finals,
            transitions,
            limits,
            graph: Arc::new(graph),
        })
    }

    /// The identity.
    #[must_use]
    pub fn id(&self) -> &ProtocolId {
        &self.id
    }

    /// The role names, in document order.
    #[must_use]
    pub fn roles(&self) -> &[String] {
        &self.roles
    }

    /// Whether `role` is declared.
    #[must_use]
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }

    /// The message types by name.
    #[must_use]
    pub fn messages(&self) -> &BTreeMap<String, MessageType> {
        &self.messages
    }

    /// The start state.
    #[must_use]
    pub fn initial(&self) -> &str {
        &self.initial
    }

    /// The final states.
    #[must_use]
    pub fn finals(&self) -> &BTreeSet<String> {
        &self.finals
    }

    /// The transitions.
    #[must_use]
    pub fn transitions(&self) -> &[Transition] {
        &self.transitions
    }

    /// The delivery limits.
    #[must_use]
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// The expanded graph the identity is computed from.
    #[must_use]
    pub fn graph(&self) -> &Graph {
        &self.graph
    }
}
