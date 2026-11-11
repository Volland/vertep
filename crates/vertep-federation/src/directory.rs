//! DID resolution as address resolution: a DID address resolves to routes (the mailboxes and
//! transports named by the DID document's services, in preference order) plus the keys to
//! encrypt to the actor and to verify it.
//!
//! The runtime registers a [`DidDirectory`] for the `did` address scheme; `vt:` addresses never
//! reach it (design D1). Failures come back as [`CommunicationError`]s, which the shell journals
//! in the sender and raises as `error.communication`.

use std::fmt;
use std::sync::Arc;

use crate::did::{Did, DidDocument, ServiceEndpoint, VerificationMethod, is_did_address};
use crate::error::{CommunicationError, CommunicationFailure};
use crate::resolver::{DidResolver, Resolution};
use crate::vocab;

/// The relay protocols a route can use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TransportKind {
    /// The ToIP Trust Spanning Protocol (service type `TSPTransport`).
    Tsp,
    /// DIDComm Messaging v2 (service type `DIDCommMessaging`).
    DidComm,
}

impl TransportKind {
    /// The DID document service types this transport serves.
    pub fn service_types(self) -> &'static [&'static str] {
        match self {
            TransportKind::Tsp => &["TSPTransport", "TspTransport"],
            TransportKind::DidComm => &["DIDCommMessaging"],
        }
    }

    /// The SCXML Event I/O Processor type IRI, which becomes `_event.origintype` of events it
    /// delivers.
    pub fn io_processor(self) -> &'static str {
        match self {
            TransportKind::Tsp => vocab::TSP_PROCESSOR,
            TransportKind::DidComm => vocab::DIDCOMM_PROCESSOR,
        }
    }
}

/// One way to reach a DID actor: a service entry's endpoint and the transport that speaks it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    /// The transport.
    pub transport: TransportKind,
    /// The id of the service entry.
    pub service_id: String,
    /// The endpoint: a mediator or intermediary URI (or DID), routing keys and accepted
    /// profiles.
    pub endpoint: ServiceEndpoint,
}

impl Route {
    /// True when the route goes through at least one mediator (DIDComm routing keys).
    pub fn is_mediated(&self) -> bool {
        !self.endpoint.routing_keys.is_empty()
    }
}

/// A resolved DID address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedAddress {
    /// The DID.
    pub did: Did,
    /// The resolved document.
    pub document: DidDocument,
    /// The document version, when the method has one.
    pub version_id: Option<String>,
    /// The usable routes, most preferred first.
    pub routes: Vec<Route>,
}

impl ResolvedAddress {
    /// The keys to encrypt to the actor (its `keyAgreement` methods).
    pub fn encryption_keys(&self) -> Vec<&VerificationMethod> {
        self.document.key_agreement_methods()
    }

    /// The keys that authenticate the actor (its `authentication` methods).
    pub fn authentication_keys(&self) -> Vec<&VerificationMethod> {
        self.document.authentication_methods()
    }
}

/// The directory for DID addresses: resolves through a [`DidResolver`] and turns service entries
/// into routes, tried in a configurable transport preference (TSP first by default, then
/// DIDComm).
// @tg: implements:: [[openspec:did-identity#DID Resolution As Address Resolution]]
// @tg: implements:: [[openspec:relay-mailboxes#Transport Selection From The DID Document]] {note: "route order by preference; fallback and its recording run in the relay outbox"}
#[derive(Clone)]
pub struct DidDirectory {
    resolver: Arc<dyn DidResolver>,
    preference: Vec<TransportKind>,
}

impl fmt::Debug for DidDirectory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DidDirectory")
            .field("preference", &self.preference)
            .finish()
    }
}

impl DidDirectory {
    /// A directory over `resolver`, preferring TSP, then DIDComm.
    pub fn new(resolver: Arc<dyn DidResolver>) -> Self {
        DidDirectory {
            resolver,
            preference: vec![TransportKind::Tsp, TransportKind::DidComm],
        }
    }

    /// The same directory with the transports tried in `preference` order. Transports left out
    /// are not used.
    pub fn with_preference(mut self, preference: Vec<TransportKind>) -> Self {
        self.preference = preference;
        self
    }

    /// True when this directory resolves `address` (it is a DID).
    pub fn handles(&self, address: &str) -> bool {
        is_did_address(address)
    }

    /// The routes of a resolved document, by preference then document order.
    pub fn routes(&self, doc: &DidDocument) -> Vec<Route> {
        let mut out = Vec::new();
        for kind in &self.preference {
            for svc in &doc.service {
                if kind.service_types().iter().any(|t| svc.has_type(t)) {
                    for ep in &svc.endpoints {
                        out.push(Route {
                            transport: *kind,
                            service_id: svc.id.clone(),
                            endpoint: ep.clone(),
                        });
                    }
                }
            }
        }
        out
    }

    /// Resolves a DID address to its routes and keys.
    ///
    /// # Errors
    ///
    /// [`CommunicationError`] for invalid DIDs, unsupported methods, failed resolutions and
    /// documents without a compatible service.
    pub fn resolve(&self, address: &str) -> Result<ResolvedAddress, CommunicationError> {
        let did = Did::parse(address)
            .map_err(|_| CommunicationFailure::InvalidAddress(address.to_owned()))?;
        let Resolution {
            document,
            version_id,
        } = self
            .resolver
            .resolve(&did)
            .map_err(|e| CommunicationError::new(e.to_failure()))?;
        let routes = self.routes(&document);
        if routes.is_empty() {
            return Err(CommunicationFailure::NoCompatibleEndpoint.into());
        }
        Ok(ResolvedAddress {
            did,
            document,
            version_id,
            routes,
        })
    }
}
