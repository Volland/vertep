//! Test relays (feature `testkit`): a DIDComm v2 mediator and a TSP intermediary that run
//! in-process on a [`LocalNetwork`](crate::net::LocalNetwork), for CI and examples.
//!
//! Vertep is a client of existing mediators and intermediaries; these exist only so tests can
//! exercise forward routing, store-and-forward, pickup, acknowledgement and routed mode offline.
//! They see exactly what a real relay sees: routing data and encrypted bytes. The mediator also
//! answers the DIDComm wire protocols of [`crate::pickup`] (Mediator Coordination 2.0, Message
//! Pickup 3.0) on the request's connection, as the third-party mediators do.

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
    /// Mediated DIDs and the recipient DIDs each registered (`keylist-update`).
    keylists: std::sync::Mutex<std::collections::BTreeMap<String, Vec<String>>>,
    me: std::sync::Weak<TestMediator>,
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
        Ok(Arc::new_cyclic(|me| TestMediator {
            store: DropBox::new(identity.did.clone()),
            agent: crate::didcomm::DidCommAgent::new(identity, km),
            net,
            resolver,
            refuse: AtomicBool::new(false),
            keylists: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            me: me.clone(),
        }))
    }

    /// The DIDs whose messages `from` picks up: itself and the recipients it registered.
    fn recipients_of(&self, from: &str) -> Vec<String> {
        let k = self
            .keylists
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut v = k.get(from).cloned().unwrap_or_default();
        v.push(from.to_owned());
        v
    }

    /// Pushes what is held for `recipient` to `from` on a live session, as a `delivery`.
    fn push_delivery(&self, from: &str, recipient: &str, to: &dyn crate::live::FrameSink) {
        use serde_json::{Value, json};
        let held = self.store.held_for(recipient);
        if held.is_empty() {
            return;
        }
        let attachments: Vec<Value> = held
            .iter()
            .map(|m| {
                let json: Value = serde_json::from_slice(&m.bytes).unwrap_or(Value::Null);
                json!({"id": m.id, "data": {"json": json}})
            })
            .collect();
        let mut msg = crate::pickup::protocol_message(
            &format!("live-{}", crate::live::frame_id()),
            crate::pickup::DELIVERY,
            self.did(),
            from,
            json!({ "recipient_did": recipient }),
        );
        if let Some(o) = msg.as_object_mut() {
            o.remove("return_route");
            o.insert("attachments".into(), Value::Array(attachments));
        }
        let Ok(did) = crate::Did::parse(from) else {
            return;
        };
        let Ok(doc) = self.resolver.resolve(&did) else {
            return;
        };
        if let Ok(jwe) = self.agent.seal_message(&msg, &doc.document) {
            let _ = to.send(jwe.as_bytes());
        }
    }

    /// Answers one Mediator Coordination 2.0 or Message Pickup 3.0 request from the
    /// authenticated `from`: the reply, or a problem report.
    fn answer(&self, from: &str, msg: &serde_json::Value) -> serde_json::Value {
        use crate::pickup::{
            DELIVERY, DELIVERY_REQUEST, KEYLIST_UPDATE, KEYLIST_UPDATE_RESPONSE, MEDIATE_GRANT,
            MEDIATE_REQUEST, MESSAGES_RECEIVED, PROBLEM_REPORT, STATUS, STATUS_REQUEST,
        };
        use serde_json::{Value, json};
        let mut keylists = self
            .keylists
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let problem = |code: &str, comment: &str| {
            (
                PROBLEM_REPORT,
                json!({"code": code, "comment": comment}),
                Vec::new(),
            )
        };
        let mediated = keylists.contains_key(from);
        // A requester may pick up for itself and for the recipients it registered.
        let allowed = |r: &str, k: &std::collections::BTreeMap<String, Vec<String>>| {
            r == from || k.get(from).is_some_and(|l| l.iter().any(|x| x == r))
        };
        let recipients = |k: &std::collections::BTreeMap<String, Vec<String>>| {
            let mut v = k.get(from).cloned().unwrap_or_default();
            v.push(from.to_owned());
            v
        };
        let body = msg.get("body").cloned().unwrap_or(Value::Null);
        let asked = body.get("recipient_did").and_then(Value::as_str);
        let (type_, body, attachments) = match msg.get("type").and_then(Value::as_str) {
            Some(MEDIATE_REQUEST) => {
                keylists.entry(from.to_owned()).or_default();
                (
                    MEDIATE_GRANT,
                    json!({"routing_did": [self.did()]}),
                    Vec::new(),
                )
            }
            Some(_) if !mediated => problem("e.p.req.not_enrolled", "request mediation first"),
            Some(KEYLIST_UPDATE) => {
                let mut updated = Vec::new();
                for u in body
                    .get("updates")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let did = u
                        .get("recipient_did")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let action = u.get("action").and_then(Value::as_str).unwrap_or_default();
                    let list = keylists.entry(from.to_owned()).or_default();
                    match action {
                        "add" if !list.iter().any(|x| x == did) => list.push(did.to_owned()),
                        "remove" => list.retain(|x| x != did),
                        _ => {}
                    }
                    updated
                        .push(json!({"recipient_did": did, "action": action, "result": "success"}));
                }
                (
                    KEYLIST_UPDATE_RESPONSE,
                    json!({ "updated": updated }),
                    Vec::new(),
                )
            }
            Some(STATUS_REQUEST | DELIVERY_REQUEST | MESSAGES_RECEIVED)
                if asked.is_some_and(|r| !allowed(r, &keylists)) =>
            {
                problem(
                    "e.p.req.not_authorized",
                    "not a recipient of this requester",
                )
            }
            Some(STATUS_REQUEST) => {
                let rs = asked.map_or_else(|| recipients(&keylists), |r| vec![r.to_owned()]);
                let count = self
                    .store
                    .held()
                    .iter()
                    .filter(|m| rs.contains(&m.recipient))
                    .count();
                (
                    STATUS,
                    json!({"message_count": count, "live_delivery": false}),
                    Vec::new(),
                )
            }
            Some(DELIVERY_REQUEST) => {
                let rs = asked.map_or_else(|| recipients(&keylists), |r| vec![r.to_owned()]);
                let limit = body
                    .get("limit")
                    .and_then(Value::as_u64)
                    .and_then(|l| usize::try_from(l).ok())
                    .unwrap_or(10);
                let held = self.store.fetch(&rs, limit).unwrap_or_default();
                if held.is_empty() {
                    (
                        STATUS,
                        json!({"message_count": 0, "live_delivery": false}),
                        Vec::new(),
                    )
                } else {
                    let attachments = held
                        .iter()
                        .map(|m| {
                            let json: Value =
                                serde_json::from_slice(&m.bytes).unwrap_or(Value::Null);
                            json!({"id": m.id, "data": {"json": json}})
                        })
                        .collect();
                    (DELIVERY, json!({}), attachments)
                }
            }
            Some(MESSAGES_RECEIVED) => {
                // Only copies held for this requester's recipients are released.
                let rs = recipients(&keylists);
                let mine: Vec<String> = self
                    .store
                    .held()
                    .into_iter()
                    .filter(|m| rs.contains(&m.recipient))
                    .map(|m| m.id)
                    .collect();
                let ids: Vec<String> = body
                    .get("message_id_list")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .filter(|id| mine.contains(id))
                    .collect();
                let _ = self.store.ack(&ids);
                let left = mine.len() - ids.len();
                (
                    STATUS,
                    json!({"message_count": left, "live_delivery": false}),
                    Vec::new(),
                )
            }
            other => problem(
                "e.p.msg.unsupported",
                &format!("unsupported message type {other:?}"),
            ),
        };
        let mut reply = crate::pickup::protocol_message(
            &format!(
                "{}-reply",
                msg.get("id").and_then(Value::as_str).unwrap_or("")
            ),
            type_,
            self.did(),
            from,
            body,
        );
        if let Some(o) = reply.as_object_mut() {
            o.remove("return_route");
            o.insert("thid".into(), msg.get("id").cloned().unwrap_or(Value::Null));
            if !attachments.is_empty() {
                o.insert("attachments".into(), Value::Array(attachments));
            }
        }
        reply
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
        let now = crate::Timestamp::now();
        // A protocol request from an authenticated DID is answered on the same connection.
        if let Ok(plain) = self.agent.decrypt(jwe, self.resolver.as_ref(), now)
            && let Some((from, _)) = &plain.sender
            && plain
                .message
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|t| {
                    t.starts_with("https://didcomm.org/coordinate-mediation/2.0/")
                        || t.starts_with("https://didcomm.org/messagepickup/3.0/")
                })
        {
            let reply = self.answer(from.as_str(), &plain.message);
            let doc = self
                .resolver
                .resolve(from)
                .map_err(|e| NetError::Refused(e.to_string()))?
                .document;
            let sealed = self
                .agent
                .seal_message(&reply, &doc)
                .map_err(|e| NetError::Refused(e.to_string()))?;
            return Ok(sealed.into_bytes());
        }
        match self.agent.open(jwe, self.resolver.as_ref(), now) {
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

/// One live session at the test mediator: after `live-delivery-change` from an authenticated
/// DID, what is held for its recipients is pushed at once and whenever more arrives.
#[cfg(feature = "didcomm")]
struct MediatorSession {
    mediator: std::sync::Weak<TestMediator>,
    to_client: Arc<dyn crate::live::FrameSink>,
}

#[cfg(feature = "didcomm")]
impl crate::live::FrameSink for MediatorSession {
    fn send(&self, frame: &[u8]) -> Result<(), NetError> {
        let Some(m) = self.mediator.upgrade() else {
            return Err(NetError::Unreachable("mediator gone".into()));
        };
        let jwe = std::str::from_utf8(frame).map_err(|e| NetError::Refused(e.to_string()))?;
        let plain = m
            .agent
            .decrypt(jwe, m.resolver.as_ref(), crate::Timestamp::now())
            .map_err(|e| NetError::Refused(e.to_string()))?;
        let Some((from, _)) = &plain.sender else {
            return Err(NetError::Refused("anonymous live request".into()));
        };
        let from = from.as_str().to_owned();
        let is_change = plain
            .message
            .get("type")
            .and_then(serde_json::Value::as_str)
            == Some(crate::pickup::LIVE_DELIVERY_CHANGE);
        let on = plain
            .message
            .pointer("/body/live_delivery")
            .and_then(serde_json::Value::as_bool)
            == Some(true);
        if !(is_change && on) {
            return Ok(());
        }
        let recipients = m.recipients_of(&from);
        for r in &recipients {
            m.push_delivery(&from, r, self.to_client.as_ref());
        }
        let weak = self.mediator.clone();
        let to = self.to_client.clone();
        m.store.live(
            &recipients,
            Arc::new(move |recipient: &str| {
                if let Some(m) = weak.upgrade() {
                    m.push_delivery(&from, recipient, to.as_ref());
                }
            }),
        );
        Ok(())
    }
}

#[cfg(feature = "didcomm")]
impl crate::live::LiveEndpoint for TestMediator {
    fn accept(
        &self,
        to_client: Arc<dyn crate::live::FrameSink>,
    ) -> Arc<dyn crate::live::FrameSink> {
        Arc::new(MediatorSession {
            mediator: self.me.clone(),
            to_client,
        })
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
