//! Transports and address resolution.
//!
//! A [`Transport`] carries a message into a mailbox and is registered as an SCXML Event I/O
//! Processor under its type IRI, which `<send type="...">` selects; the runtime lists every
//! registered transport in `_ioprocessors`. M2 ships the in-process [`LocalTransport`].
//!
//! The [`Directory`] maps an address to the mailbox that serves it. Addresses hosted on this
//! node are resolved first; any other address goes to the [`Resolver`] registered for its IRI
//! scheme. Resolution depends only on the address.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use vertep_core::session::SCXML_EVENT_PROCESSOR;

use crate::address::Address;
use crate::mailbox::{Envelope, Mailbox};

/// A delivery failure; the outbox keeps the send and retries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportError(pub String);

impl core::fmt::Display for TransportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "transport error: {}", self.0)
    }
}

impl std::error::Error for TransportError {}

/// The transport contract: an SCXML Event I/O Processor that delivers into mailboxes.
// @tg: implements:: [[openspec:messaging#Pluggable Transports]]
pub trait Transport: Send + Sync {
    /// The Event I/O Processor type IRI.
    fn type_iri(&self) -> &str;
    /// Other names `<send type>` may use for this transport.
    fn aliases(&self) -> Vec<String> {
        Vec::new()
    }
    /// The location listed in `_ioprocessors` for an actor at `address`.
    fn location(&self, address: &Address) -> String {
        address.as_str().to_owned()
    }
    /// Delivers `env` into `mailbox`.
    ///
    /// # Errors
    ///
    /// [`TransportError`] when the message was not accepted.
    fn deliver(&self, mailbox: &dyn Mailbox, env: Envelope) -> Result<(), TransportError>;
}

/// The in-process transport: the SCXML Event I/O Processor
/// (`http://www.w3.org/TR/scxml/#SCXMLEventProcessor`, alias `scxml`).
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalTransport;

impl Transport for LocalTransport {
    fn type_iri(&self) -> &str {
        SCXML_EVENT_PROCESSOR
    }

    fn aliases(&self) -> Vec<String> {
        vec!["scxml".to_owned()]
    }

    fn deliver(&self, mailbox: &dyn Mailbox, env: Envelope) -> Result<(), TransportError> {
        mailbox
            .enqueue(env)
            .map_err(|e| TransportError(e.to_string()))
    }
}

/// Resolves addresses of one IRI scheme to mailboxes.
pub trait Resolver: Send + Sync {
    /// The mailbox serving `address`, if this resolver knows it.
    fn resolve(&self, address: &Address) -> Option<Arc<dyn Mailbox>>;
}

/// An address the directory could not resolve.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveError(pub String);

impl core::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "cannot resolve {}", self.0)
    }
}

impl std::error::Error for ResolveError {}

/// The resolvers of non-local addresses, keyed by IRI scheme.
// @tg: implements:: [[openspec:messaging#Address Resolution]]
#[derive(Default)]
pub struct Directory {
    resolvers: RwLock<BTreeMap<String, Arc<dyn Resolver>>>,
}

impl std::fmt::Debug for Directory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let schemes: Vec<String> = self
            .resolvers
            .read()
            .map(|r| r.keys().cloned().collect())
            .unwrap_or_default();
        f.debug_struct("Directory")
            .field("schemes", &schemes)
            .finish()
    }
}

impl Directory {
    /// Registers `resolver` for addresses whose scheme is `scheme`.
    pub fn register(&self, scheme: &str, resolver: Arc<dyn Resolver>) {
        if let Ok(mut r) = self.resolvers.write() {
            r.insert(scheme.to_ascii_lowercase(), resolver);
        }
    }

    /// The mailbox of a non-local `address`, from the resolver of its scheme.
    ///
    /// # Errors
    ///
    /// [`ResolveError`] when no resolver knows the address.
    pub fn resolve(&self, address: &Address) -> Result<Arc<dyn Mailbox>, ResolveError> {
        let r = self
            .resolvers
            .read()
            .ok()
            .and_then(|r| r.get(&address.scheme().to_ascii_lowercase()).cloned());
        r.and_then(|r| r.resolve(address))
            .ok_or_else(|| ResolveError(address.as_str().to_owned()))
    }
}

/// The registered transports, by type IRI and alias.
#[derive(Clone, Default)]
pub struct Transports {
    by_type: BTreeMap<String, Arc<dyn Transport>>,
    primary: Vec<Arc<dyn Transport>>,
}

impl std::fmt::Debug for Transports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transports")
            .field("types", &self.by_type.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Transports {
    /// Only the local transport.
    pub fn local() -> Self {
        let mut t = Transports::default();
        t.register(Arc::new(LocalTransport));
        t
    }

    /// Registers `t` under its type IRI and aliases.
    pub fn register(&mut self, t: Arc<dyn Transport>) {
        self.by_type.insert(t.type_iri().to_owned(), t.clone());
        for a in t.aliases() {
            self.by_type.insert(a, t.clone());
        }
        self.primary.push(t);
    }

    /// The transport for a send type.
    pub fn get(&self, type_: &str) -> Option<&Arc<dyn Transport>> {
        self.by_type.get(type_)
    }

    /// Every type name a `<send>` may use.
    pub fn send_types(&self) -> Vec<String> {
        self.by_type.keys().cloned().collect()
    }

    /// The `_ioprocessors` value for an actor at `address`: each transport's type IRI with its
    /// location.
    pub fn ioprocessors(&self, address: &Address) -> vertep_core::datamodel::Value {
        use vertep_core::datamodel::Value;
        let mut entries = Vec::new();
        for (name, t) in &self.by_type {
            entries.push((
                name.clone(),
                Value::Object(vec![(
                    "location".to_owned(),
                    Value::String(t.location(address)),
                )]),
            ));
        }
        Value::Object(entries)
    }
}
