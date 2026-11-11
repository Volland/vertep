//! Test relays (feature `testkit`): a DIDComm v2 mediator and a TSP intermediary that run
//! in-process on a [`LocalNetwork`](crate::net::LocalNetwork), for CI and examples.
//!
//! Vertep is a client of existing mediators and intermediaries; these exist only so tests can
//! exercise forward routing, store-and-forward, pickup, acknowledgement and routed mode offline.
//! They see exactly what a real relay sees: routing data and encrypted bytes.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::net::{DropBox, NetError, PickupSource, RelayEndpoint, RelayNetwork, StoredMessage};
use crate::resolver::DidResolver;

/// A DIDComm v2 mediator: opens its own forward layer, passes the inner message on to the next
/// mediator or stores it for the recipient DID, and serves message pickup.
#[cfg(feature = "didcomm")]
pub struct TestMediator {
    agent: crate::didcomm::DidCommAgent,
    store: Arc<DropBox>,
    net: Arc<dyn RelayNetwork>,
    resolver: Arc<dyn DidResolver>,
    refuse: AtomicBool,
}

#[cfg(feature = "didcomm")]
impl fmt::Debug for TestMediator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestMediator")
            .field("did", &self.agent.identity().did)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "didcomm")]
impl TestMediator {
    /// A mediator with a fresh `did:peer` whose DIDComm service is `uri`.
    ///
    /// # Errors
    ///
    /// [`crate::KeyError`] when the keys cannot be created.
    pub fn new(
        km: Arc<dyn crate::KeyManager>,
        uri: &str,
        net: Arc<dyn RelayNetwork>,
        resolver: Arc<dyn DidResolver>,
    ) -> Result<Arc<Self>, crate::KeyError> {
        use crate::did::{Service, ServiceEndpoint};
        let identity = crate::ActorIdentity::provision(
            km.as_ref(),
            &crate::IdentityConfig::new(crate::DidMethodChoice::Peer {
                services: vec![Service {
                    id: "#didcomm".into(),
                    types: vec!["DIDCommMessaging".into()],
                    endpoints: vec![ServiceEndpoint::uri(uri)],
                }],
            }),
        )?;
        Ok(Arc::new(TestMediator {
            store: DropBox::new(identity.did.clone()),
            agent: crate::didcomm::DidCommAgent::new(identity, km),
            net,
            resolver,
            refuse: AtomicBool::new(false),
        }))
    }

    /// The mediator's DID.
    pub fn did(&self) -> &str {
        &self.agent.identity().did
    }

    /// The routing key recipients list in their DIDComm service to route through this mediator.
    pub fn routing_key(&self) -> String {
        self.agent.identity().agreement.kid.clone()
    }

    /// What the mediator holds.
    pub fn store(&self) -> &Arc<DropBox> {
        &self.store
    }

    /// Makes the mediator refuse (`true`) or accept new messages.
    pub fn set_refusing(&self, refuse: bool) {
        self.refuse.store(refuse, Ordering::SeqCst);
    }
}

#[cfg(feature = "didcomm")]
impl RelayEndpoint for TestMediator {
    fn receive(&self, body: &[u8]) -> Result<Vec<u8>, NetError> {
        use crate::didcomm::DidCommReceived;
        if self.refuse.load(Ordering::SeqCst) {
            return Err(NetError::Refused("mediator refuses".into()));
        }
        let jwe = std::str::from_utf8(body).map_err(|e| NetError::Refused(e.to_string()))?;
        match self
            .agent
            .open(jwe, self.resolver.as_ref(), crate::Timestamp::now())
        {
            Ok(DidCommReceived::Forward { next, message }) => {
                if next.contains('#') {
                    // The next hop is another mediator's routing key.
                    let did = crate::Did::from_did_url(&next)
                        .map_err(|e| NetError::Refused(e.to_string()))?;
                    let doc = self
                        .resolver
                        .resolve(&did)
                        .map_err(|e| NetError::Refused(e.to_string()))?
                        .document;
                    let uri = doc
                        .service
                        .iter()
                        .filter(|s| s.has_type("DIDCommMessaging"))
                        .flat_map(|s| s.endpoints.iter())
                        .map(|e| e.uri.clone())
                        .next()
                        .ok_or_else(|| NetError::Refused(format!("{next}: no endpoint")))?;
                    self.net.post(&uri, message.as_bytes())
                } else {
                    self.store.store(&next, message.into_bytes());
                    Ok(Vec::new())
                }
            }
            Ok(DidCommReceived::Event(_)) => {
                Err(NetError::Refused("a mediator accepts forwards only".into()))
            }
            Err(e) => Err(NetError::Refused(e.to_string())),
        }
    }
}

#[cfg(feature = "didcomm")]
impl PickupSource for TestMediator {
    fn fetch(&self, recipients: &[String], limit: usize) -> Result<Vec<StoredMessage>, NetError> {
        self.store.fetch(recipients, limit)
    }

    fn ack(&self, ids: &[String]) -> Result<(), NetError> {
        self.store.ack(ids)
    }

    fn live(&self, recipients: &[String], wake: Arc<dyn Fn(&str) + Send + Sync>) -> bool {
        self.store.live(recipients, wake)
    }
}

/// A TSP intermediary: opens its own routed layer, re-seals the opaque inner message to the next
/// hop, and either posts it to the final recipient's endpoint or holds it for pickup
/// (store-and-forward).
#[cfg(feature = "tsp")]
pub struct TestIntermediary {
    me: crate::tsp::TspIdentity,
    did: String,
    store: Arc<DropBox>,
    net: Arc<dyn RelayNetwork>,
    resolver: Arc<dyn DidResolver>,
    refuse: AtomicBool,
    store_and_forward: bool,
}

#[cfg(feature = "tsp")]
impl fmt::Debug for TestIntermediary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestIntermediary")
            .field("did", &self.did)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "tsp")]
impl TestIntermediary {
    /// An intermediary with a fresh `did:peer` whose TSP endpoint is `uri`. With
    /// `store_and_forward`, the last hop holds messages for pickup instead of posting them.
    ///
    /// # Errors
    ///
    /// A description when the keys or the VID cannot be created.
    pub fn new(
        km: Arc<dyn crate::KeyManager>,
        uri: &str,
        net: Arc<dyn RelayNetwork>,
        resolver: Arc<dyn DidResolver>,
        store_and_forward: bool,
    ) -> Result<Arc<Self>, String> {
        use crate::did::{Service, ServiceEndpoint};
        let identity = crate::ActorIdentity::provision(
            km.as_ref(),
            &crate::IdentityConfig::new(crate::DidMethodChoice::Peer {
                services: vec![Service {
                    id: "#tsp".into(),
                    types: vec!["TSPTransport".into()],
                    endpoints: vec![ServiceEndpoint::uri(uri)],
                }],
            }),
        )
        .map_err(|e| e.to_string())?;
        let url = url::Url::parse(uri).map_err(|e| e.to_string())?;
        let me = crate::tsp::TspIdentity::new(&identity, km, url).map_err(|e| e.to_string())?;
        Ok(Arc::new(TestIntermediary {
            me,
            store: DropBox::new(identity.did.clone()),
            did: identity.did,
            net,
            resolver,
            refuse: AtomicBool::new(false),
            store_and_forward,
        }))
    }

    /// The intermediary's DID (its VID).
    pub fn did(&self) -> &str {
        &self.did
    }

    /// What it holds for pickup.
    pub fn store(&self) -> &Arc<DropBox> {
        &self.store
    }

    /// Makes the intermediary refuse (`true`) or route messages.
    pub fn set_refusing(&self, refuse: bool) {
        self.refuse.store(refuse, Ordering::SeqCst);
    }
}

#[cfg(feature = "tsp")]
impl RelayEndpoint for TestIntermediary {
    fn receive(&self, body: &[u8]) -> Result<Vec<u8>, NetError> {
        if self.refuse.load(Ordering::SeqCst) {
            return Err(NetError::Refused(format!("{} refuses to route", self.did)));
        }
        let mut copy = body.to_vec();
        let hop = crate::tsp::forward_routed(&self.me, &mut copy, self.resolver.as_ref())
            .map_err(|e| NetError::Refused(e.to_string()))?;
        if hop.last && self.store_and_forward {
            self.store.store(&hop.next, hop.bytes);
            return Ok(Vec::new());
        }
        let did = crate::Did::parse(&hop.next).map_err(|e| NetError::Refused(e.to_string()))?;
        let doc = self
            .resolver
            .resolve(&did)
            .map_err(|e| NetError::Refused(e.to_string()))?
            .document;
        let uri = crate::tsp::tsp_uri(&doc)
            .ok_or_else(|| NetError::Refused(format!("{}: no TSP endpoint", hop.next)))?;
        self.net.post(&uri, &hop.bytes)
    }
}

#[cfg(feature = "tsp")]
impl PickupSource for TestIntermediary {
    fn fetch(&self, recipients: &[String], limit: usize) -> Result<Vec<StoredMessage>, NetError> {
        self.store.fetch(recipients, limit)
    }

    fn ack(&self, ids: &[String]) -> Result<(), NetError> {
        self.store.ack(ids)
    }

    fn live(&self, recipients: &[String], wake: Arc<dyn Fn(&str) + Send + Sync>) -> bool {
        self.store.live(recipients, wake)
    }
}
