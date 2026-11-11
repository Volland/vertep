//! The protocol-neutral relay envelope: one canonical JSON-LD body that carries an SCXML event
//! over TSP and DIDComm alike (design D8).
//!
//! The envelope holds the event `name`, `sendid`, typed `data` (with its declared type IRI when
//! the chart has one), the sender's chart version IRI, a correlation id and the message id used
//! for deduplication. It never holds `origin` or `origintype`: the receiver derives them from the
//! verified sender and the I/O processor that delivered the message ([`crate::origin`]).
//! Unknown members (extension headers from newer senders) are ignored on read.

use std::fmt;

use oxrdf::Graph;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};
use vertep_core::datamodel::{Event, EventType, Value};

use crate::vocab::RELAY_CONTEXT;

/// The JSON-LD context of the relay envelope, served at [`RELAY_CONTEXT`].
pub const RELAY_CONTEXT_JSON: &str = include_str!("../ontology/relay-v1.jsonld");

/// The JSON-LD `type` of a relay envelope.
pub const ENVELOPE_TYPE: &str = "RelayEnvelope";

/// A [`Value`] serialized as plain JSON with object members in insertion order. `undefined`
/// inside arrays and objects is written as `null`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JsonValue(pub Value);

impl Serialize for JsonValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match &self.0 {
            Value::Undefined | Value::Null => s.serialize_unit(),
            Value::Bool(b) => s.serialize_bool(*b),
            Value::Number(n) => {
                if !n.is_finite() {
                    s.serialize_unit()
                } else if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
                    // An integral value within the exact f64 range writes without a fraction.
                    #[allow(clippy::cast_possible_truncation)]
                    s.serialize_i64(*n as i64)
                } else {
                    s.serialize_f64(*n)
                }
            }
            Value::String(t) => s.serialize_str(t),
            Value::Array(items) => {
                let mut seq = s.serialize_seq(Some(items.len()))?;
                for i in items {
                    seq.serialize_element(&JsonValueRef(i))?;
                }
                seq.end()
            }
            Value::Object(members) => {
                let mut map = s.serialize_map(Some(members.len()))?;
                for (k, v) in members {
                    map.serialize_entry(k, &JsonValueRef(v))?;
                }
                map.end()
            }
        }
    }
}

struct JsonValueRef<'a>(&'a Value);

impl Serialize for JsonValueRef<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        JsonValue(self.0.clone()).serialize(s)
    }
}

impl<'de> Deserialize<'de> for JsonValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = JsonValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON value")
            }
            fn visit_unit<E: de::Error>(self) -> Result<JsonValue, E> {
                Ok(JsonValue(Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> Result<JsonValue, E> {
                Ok(JsonValue(Value::Null))
            }
            fn visit_bool<E: de::Error>(self, b: bool) -> Result<JsonValue, E> {
                Ok(JsonValue(Value::Bool(b)))
            }
            #[allow(clippy::cast_precision_loss)]
            fn visit_i64<E: de::Error>(self, n: i64) -> Result<JsonValue, E> {
                Ok(JsonValue(Value::Number(n as f64)))
            }
            #[allow(clippy::cast_precision_loss)]
            fn visit_u64<E: de::Error>(self, n: u64) -> Result<JsonValue, E> {
                Ok(JsonValue(Value::Number(n as f64)))
            }
            fn visit_f64<E: de::Error>(self, n: f64) -> Result<JsonValue, E> {
                Ok(JsonValue(Value::Number(n)))
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<JsonValue, E> {
                Ok(JsonValue(Value::String(s.to_owned())))
            }
            fn visit_string<E: de::Error>(self, s: String) -> Result<JsonValue, E> {
                Ok(JsonValue(Value::String(s)))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<JsonValue, A::Error> {
                let mut out = Vec::new();
                while let Some(JsonValue(v)) = seq.next_element()? {
                    out.push(v);
                }
                Ok(JsonValue(Value::Array(out)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<JsonValue, A::Error> {
                let mut out: Vec<(String, Value)> = Vec::new();
                while let Some((k, JsonValue(v))) = map.next_entry::<String, JsonValue>()? {
                    if let Some(slot) = out.iter_mut().find(|(key, _)| *key == k) {
                        slot.1 = v;
                    } else {
                        out.push((k, v));
                    }
                }
                Ok(JsonValue(Value::Object(out)))
            }
        }
        d.deserialize_any(V)
    }
}

fn is_undefined(v: &JsonValue) -> bool {
    matches!(v.0, Value::Undefined)
}

/// An SCXML event in transit between DID actors.
// @tg: implements:: [[openspec:relay-mailboxes#SCXML Event Envelope Mapping]]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RelayEnvelope {
    /// The JSON-LD context ([`RELAY_CONTEXT`]).
    #[serde(rename = "@context")]
    pub context: String,
    /// The JSON-LD type ([`ENVELOPE_TYPE`]).
    #[serde(rename = "type")]
    pub type_: String,
    /// The deduplication key, stable across redeliveries and retries.
    #[serde(rename = "messageId")]
    pub message_id: String,
    /// The event name.
    pub name: String,
    /// The `sendid` of the sender's `<send>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sendid: Option<String>,
    /// The event payload.
    #[serde(default, skip_serializing_if = "is_undefined")]
    pub data: JsonValue,
    /// The content-addressed IRI of the payload's declared type, when the sender's chart
    /// declares one.
    #[serde(rename = "dataType", default, skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    /// The sender's chart version IRI.
    #[serde(
        rename = "chartVersion",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub chart_version: Option<String>,
    /// A `vt:request` correlation id, or the correlation id a reply answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation: Option<String>,
}

/// An envelope that cannot be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvelopeError(pub String);

impl fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "malformed relay envelope: {}", self.0)
    }
}

impl std::error::Error for EnvelopeError {}

impl RelayEnvelope {
    /// The envelope of `event` sent with message id `message_id`. Only the fields the sender
    /// controls are kept: `origin`, `origintype` and `invokeid` are dropped.
    pub fn from_event(message_id: impl Into<String>, event: &Event) -> Self {
        RelayEnvelope {
            context: RELAY_CONTEXT.to_owned(),
            type_: ENVELOPE_TYPE.to_owned(),
            message_id: message_id.into(),
            name: event.name.clone(),
            sendid: event.sendid.clone(),
            data: JsonValue(event.data.clone()),
            data_type: None,
            chart_version: None,
            correlation: None,
        }
    }

    /// The same envelope naming the sender's chart version.
    pub fn with_chart_version(mut self, iri: impl Into<String>) -> Self {
        self.chart_version = Some(iri.into());
        self
    }

    /// The same envelope naming the payload's declared type IRI.
    pub fn with_data_type(mut self, iri: impl Into<String>) -> Self {
        self.data_type = Some(iri.into());
        self
    }

    /// The JSON body.
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Reads an envelope, ignoring members it does not know.
    ///
    /// # Errors
    ///
    /// [`EnvelopeError`] for invalid JSON, another `type` or missing required fields.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, EnvelopeError> {
        let env: RelayEnvelope =
            serde_json::from_slice(bytes).map_err(|e| EnvelopeError(e.to_string()))?;
        if env.type_ != ENVELOPE_TYPE {
            return Err(EnvelopeError(format!(
                "type {} is not {ENVELOPE_TYPE}",
                env.type_
            )));
        }
        if env.message_id.is_empty() || env.name.is_empty() {
            return Err(EnvelopeError("empty messageId or name".into()));
        }
        Ok(env)
    }

    /// The external event the receiver's `_event` holds: name, data and send id from the
    /// envelope; `origin` and `origintype` set by [`crate::origin::VerifiedOrigin::apply`].
    pub fn to_event(&self) -> Event {
        let mut e = Event::named(self.name.clone(), EventType::External);
        e.sendid = self.sendid.clone();
        e.data = self.data.0.clone();
        e
    }

    /// The envelope as RDF, read through its JSON-LD context: what the SHACL envelope shape
    /// validates and what the runtime stores.
    ///
    /// # Errors
    ///
    /// [`EnvelopeError`] when the JSON-LD processor rejects the body.
    pub fn to_graph(&self) -> Result<Graph, EnvelopeError> {
        jsonld_graph(&self.to_bytes())
    }
}

/// Reads a JSON-LD document that uses the relay context into a graph. The context is served from
/// the bundled copy; no other remote document is loaded.
///
/// # Errors
///
/// [`EnvelopeError`] when the document is not valid JSON-LD.
pub fn jsonld_graph(json: &[u8]) -> Result<Graph, EnvelopeError> {
    let parser = oxjsonld::JsonLdParser::new()
        .for_slice(json)
        .with_load_document_callback(|url, _opts| {
            if url == RELAY_CONTEXT {
                Ok(oxjsonld::JsonLdRemoteDocument {
                    document: RELAY_CONTEXT_JSON.as_bytes().to_vec(),
                    document_url: url.to_owned(),
                })
            } else {
                Err(format!("remote JSON-LD document {url} is not allowed").into())
            }
        });
    let mut g = Graph::new();
    for quad in parser {
        let q = quad.map_err(|e| EnvelopeError(e.to_string()))?;
        g.insert(q.as_ref());
    }
    Ok(g)
}
