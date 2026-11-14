//! The exchange engine (design D2): role charts as journaled agent actors, connected to the
//! wallet's operations, a protocol codec and a wire.
//!
//! Every exchange is an actor running its protocol's role chart in an [`AgentHost`]: the chart
//! holds the order of steps and their single-use rules, and its consent state is a decide state.
//! The engine turns inbound wire messages into chart events (after the wallet's checks, so
//! events carry metadata only, never claim values) and the chart's `wire.*` sends into wire
//! messages, and records each vocabulary step in an optional [`History`]. The wallet actor
//! `…/charts/wallet/<DID>` journals credential metadata.

pub mod charts;

use std::collections::BTreeMap;
use std::sync::Arc;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Map, Value, json};
use vertep_agent::decider::Decider;
use vertep_agent::host::{AgentHost, InstantiateError};
use vertep_agent::journal::AgentJournal;
use vertep_core::datamodel::{Event, EventType};
use vertep_federation::PublicKey;
use vertep_rdf::ChartVersion;

use crate::consent::PolicyDecider;
use crate::credential::{
    Credential, CredentialMeta, IssueRequest, Presentation, PresentationRequest, StatusRef,
};
use crate::error::WalletError;
use crate::history::{Claims, History, Line};
use crate::jose::b64;
use crate::message::{Body, ExchangeMessage};
use crate::plugin::ExchangeProtocol;
use crate::vocab::Role;
use crate::wallet::Wallet;

/// Carries encoded exchange messages to their recipient.
pub trait Wire: Send + Sync {
    /// Sends `bytes` of protocol `protocol` from `from` to `to` as message `msg_id`.
    ///
    /// # Errors
    ///
    /// [`WalletError::Protocol`] when the message cannot be handed over.
    fn send(
        &self,
        protocol: &str,
        from: &str,
        to: &str,
        msg_id: &str,
        bytes: &[u8],
    ) -> Result<(), WalletError>;
}

/// What an issuer will issue once possession is proven.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    /// The format IRI.
    pub format: String,
    /// Credential types.
    pub types: Vec<String>,
    /// The claims, with values.
    pub claims: Map<String, Value>,
    /// Valid until (milliseconds).
    pub valid_until: Option<i64>,
    /// The status entry to embed.
    pub status: Option<StatusRef>,
}

/// One exchange the engine runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExchangeInfo {
    /// The exchange id.
    pub id: String,
    /// This party's role.
    pub role: Role,
    /// The protocol IRI.
    pub protocol: String,
    /// The other party.
    pub peer: String,
    /// The actor running the role chart.
    pub actor: String,
}

/// The outcome of a verifier's exchange.
#[derive(Clone, Debug, PartialEq)]
pub struct Verified {
    /// The issuer.
    pub issuer: String,
    /// The credential digest.
    pub credential: String,
    /// The disclosed claims, with values.
    pub disclosed: Map<String, Value>,
    /// The key that presented it (the holder), as a `did:key`.
    pub holder: String,
    /// The credential types.
    pub types: Vec<String>,
    /// The credential's status entry.
    pub status: Option<StatusRef>,
}

fn pct(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

fn event(name: &str, data: Value) -> Event {
    Event {
        name: name.to_owned(),
        type_: EventType::External,
        data: vertep_agent::json::from_json(&data),
        ..Event::default()
    }
}

fn nonce() -> Result<String, WalletError> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| WalletError::Protocol(e.to_string()))?;
    Ok(b64(&b))
}

/// The agent (DID) an audience names: OpenID4VP `decentralized_identifier:` client ids map to
/// their DID, as histories name agents by DID.
pub fn agent_of(audience: &str) -> String {
    audience
        .strip_prefix(crate::protocol::oid4vc::DID_CLIENT_PREFIX)
        .unwrap_or(audience)
        .to_owned()
}

fn claims_of(v: &Value) -> Claims {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The exchange engine of one wallet.
pub struct Exchanges {
    wallet: Wallet,
    host: AgentHost,
    wire: Arc<dyn Wire>,
    history: Option<History>,
    versions: BTreeMap<(String, Role), ChartVersion>,
    wallet_actor: String,
    exchanges: BTreeMap<(Role, String), ExchangeInfo>,
    offers: BTreeMap<String, Offer>,
    requests: BTreeMap<String, PresentationRequest>,
    issued: BTreeMap<String, CredentialMeta>,
    verified: BTreeMap<String, Verified>,
    outbox_seen: usize,
    tag: String,
    counter: u64,
    auto_accept_offers: bool,
}

impl std::fmt::Debug for Exchanges {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Exchanges")
            .field("wallet", &self.wallet.did())
            .field("exchanges", &self.exchanges.len())
            .finish_non_exhaustive()
    }
}

/// How to build an [`Exchanges`].
pub struct ExchangesBuilder {
    clock: Arc<dyn vertep_runtime::Clock>,
    journal: Option<Arc<dyn AgentJournal>>,
    consent: Arc<dyn Decider>,
    history: Option<History>,
}

impl ExchangesBuilder {
    /// The agent journal (default: in memory).
    #[must_use]
    pub fn journal(mut self, j: Arc<dyn AgentJournal>) -> Self {
        self.journal = Some(j);
        self
    }

    /// The consent decider (default: [`PolicyDecider`]).
    #[must_use]
    pub fn consent(mut self, d: Arc<dyn Decider>) -> Self {
        self.consent = d;
        self
    }

    /// Records every step in `history`.
    #[must_use]
    pub fn history(mut self, h: History) -> Self {
        self.history = Some(h);
        self
    }

    fn host(&self) -> AgentHost {
        let mut b = AgentHost::builder(self.clock.clone()).decider("consent", self.consent.clone());
        if let Some(j) = &self.journal {
            b = b.journal(j.clone());
        }
        b.build()
    }

    fn engine(self, wallet: Wallet, wire: Arc<dyn Wire>) -> Result<Exchanges, WalletError> {
        let host = self.host();
        let tag = crate::credential::digest(wallet.did().as_bytes())[7..15].to_owned();
        let wallet_actor = format!("{}/{}", charts::wallet_type(), pct(wallet.did()));
        let mut e = Exchanges {
            wallet,
            host,
            wire,
            history: self.history,
            versions: BTreeMap::new(),
            wallet_actor,
            exchanges: BTreeMap::new(),
            offers: BTreeMap::new(),
            requests: BTreeMap::new(),
            issued: BTreeMap::new(),
            verified: BTreeMap::new(),
            outbox_seen: 0,
            tag,
            counter: 0,
            auto_accept_offers: true,
        };
        e.register_wallet_chart()?;
        Ok(e)
    }

    /// Creates the wallet of `wallet`'s identity.
    ///
    /// # Errors
    ///
    /// [`WalletError::Exists`] when the identity already has a wallet in the journal.
    // @tg: implements:: [[openspec:agent-wallet#Wallet Actor]]
    pub fn create(self, wallet: Wallet, wire: Arc<dyn Wire>) -> Result<Exchanges, WalletError> {
        let did = wallet.did().to_owned();
        let mut e = self.engine(wallet, wire)?;
        let v = e.versions[&("wallet".to_owned(), Role::Holder)].clone();
        match e.host.spawn(&e.wallet_actor, &v) {
            Ok(()) => {}
            Err(InstantiateError::Exists(a)) => return Err(WalletError::Exists(a)),
            Err(other) => return Err(WalletError::Protocol(other.to_string())),
        }
        e.host.send(
            &e.wallet_actor.clone(),
            event("wallet.created", json!({"did": did})),
        );
        e.settle()?;
        Ok(e)
    }

    /// Opens the existing wallet of `wallet`'s identity (after a restart).
    ///
    /// # Errors
    ///
    /// [`WalletError::UnknownCredential`] when the identity has no wallet in the journal.
    pub fn open(self, wallet: Wallet, wire: Arc<dyn Wire>) -> Result<Exchanges, WalletError> {
        let mut e = self.engine(wallet, wire)?;
        e.host.activate(&e.wallet_actor.clone()).map_err(|err| {
            WalletError::Storage(format!("no wallet for {}: {err}", e.wallet.did()))
        })?;
        Ok(e)
    }
}

impl Exchanges {
    /// A builder on `clock`.
    pub fn builder(clock: Arc<dyn vertep_runtime::Clock>) -> ExchangesBuilder {
        ExchangesBuilder {
            clock,
            journal: None,
            consent: Arc::new(PolicyDecider),
            history: None,
        }
    }

    fn register_wallet_chart(&mut self) -> Result<(), WalletError> {
        let chart = read_chart(charts::WALLET)?;
        let v = self
            .host
            .register(chart)
            .map_err(|e| WalletError::Protocol(format!("wallet chart: {e:?}")))?;
        self.versions.insert(("wallet".to_owned(), Role::Holder), v);
        Ok(())
    }

    fn version(
        &mut self,
        protocol: &dyn ExchangeProtocol,
        role: Role,
    ) -> Result<ChartVersion, WalletError> {
        let key = (protocol.iri().to_owned(), role);
        if let Some(v) = self.versions.get(&key) {
            return Ok(v.clone());
        }
        let chart = read_chart(protocol.role_chart(role))?;
        let v = self
            .host
            .register(chart)
            .map_err(|e| WalletError::Protocol(format!("{} chart: {e:?}", role.as_str())))?;
        self.versions.insert(key, v.clone());
        Ok(v)
    }

    /// The wallet.
    pub fn wallet(&self) -> &Wallet {
        &self.wallet
    }

    /// The wallet, mutably (to trust issuers).
    pub fn wallet_mut(&mut self) -> &mut Wallet {
        &mut self.wallet
    }

    /// The agent host running the wallet actor and the role charts.
    pub fn host(&mut self) -> &mut AgentHost {
        &mut self.host
    }

    /// The wallet actor's id.
    pub fn wallet_actor(&self) -> &str {
        &self.wallet_actor
    }

    /// Whether offers are accepted without asking (default true).
    pub fn set_auto_accept_offers(&mut self, yes: bool) {
        self.auto_accept_offers = yes;
    }

    /// The exchanges this engine runs.
    pub fn exchanges(&self) -> Vec<ExchangeInfo> {
        self.exchanges.values().cloned().collect()
    }

    /// The exchange `id` in `role`.
    pub fn exchange(&self, role: Role, id: &str) -> Option<&ExchangeInfo> {
        self.exchanges.get(&(role, id.to_owned()))
    }

    /// The active states of the exchange `id` in `role`, or its final state once it ended.
    ///
    /// # Errors
    ///
    /// [`WalletError::Protocol`] for an unknown exchange.
    pub fn state(&mut self, role: Role, id: &str) -> Result<Vec<String>, WalletError> {
        let actor = self
            .exchange(role, id)
            .ok_or_else(|| WalletError::Protocol(format!("unknown exchange {id}")))?
            .actor
            .clone();
        let st = self
            .host
            .state(&actor)
            .map_err(|e| WalletError::Protocol(e.to_string()))?;
        if st.terminated {
            let ctx = vertep_agent::decider::ContextView::new(st.datamodel);
            let outcome = ctx
                .get("outcome")
                .map(vertep_agent::json::to_json)
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default();
            return Ok(vec![outcome]);
        }
        Ok(st.configuration)
    }

    /// The verifier's outcome of exchange `id`, once a presentation was accepted.
    pub fn verified(&self, id: &str) -> Option<&Verified> {
        self.verified.get(id)
    }

    /// The credential issued in exchange `id`.
    pub fn issued(&self, id: &str) -> Option<&CredentialMeta> {
        self.issued.get(id)
    }

    fn next_msg(&mut self) -> String {
        self.counter += 1;
        format!("{}-{}", self.tag, self.counter)
    }

    fn record(&self, line: Line) {
        if let Some(h) = &self.history {
            h.push(line);
        }
    }

    fn start(
        &mut self,
        protocol: &str,
        role: Role,
        id: &str,
        peer: &str,
        first: Event,
    ) -> Result<(), WalletError> {
        let p = self.wallet.registry().protocol(protocol)?;
        let v = self.version(p.as_ref(), role)?;
        let actor = format!(
            "{}/{}/{}",
            charts::chart_type(role),
            pct(self.wallet.did()),
            pct(id)
        );
        self.host
            .spawn(&actor, &v)
            .map_err(|e| WalletError::Protocol(e.to_string()))?;
        self.exchanges.insert(
            (role, id.to_owned()),
            ExchangeInfo {
                id: id.to_owned(),
                role,
                protocol: protocol.to_owned(),
                peer: peer.to_owned(),
                actor: actor.clone(),
            },
        );
        self.host.send(
            &self.wallet_actor.clone(),
            event(
                "exchange.started",
                json!({"exchange": id, "role": role.as_str()}),
            ),
        );
        self.host.send(&actor, first);
        self.settle()
    }

    /// As issuer: offers `offer` to `holder` over `protocol`; returns the exchange id.
    ///
    /// # Errors
    ///
    /// [`WalletError`] when the protocol is unknown or the message cannot be sent.
    pub fn offer(
        &mut self,
        protocol: &str,
        holder: &str,
        offer: Offer,
    ) -> Result<String, WalletError> {
        self.wallet.registry().format(&offer.format)?;
        let id = format!("x-{}", nonce()?);
        let n = nonce()?;
        self.offers.insert(id.clone(), offer);
        self.start(
            protocol,
            Role::Issuer,
            &id,
            holder,
            event(
                "exchange.start",
                json!({"exchange": id, "peer": holder, "nonce": n}),
            ),
        )?;
        Ok(id)
    }

    /// As verifier: asks `holder` for `claims` (from credentials of `types`) over `protocol`;
    /// returns the exchange id.
    ///
    /// # Errors
    ///
    /// [`WalletError`] when the protocol is unknown or the message cannot be sent.
    pub fn request(
        &mut self,
        protocol: &str,
        holder: &str,
        claims: &[&str],
        types: &[&str],
        purpose: Option<&str>,
    ) -> Result<String, WalletError> {
        self.request_for(protocol, holder, claims, types, purpose, None)
    }

    /// As [`Exchanges::request`], naming the verifier by `audience` (an OpenID4VP client
    /// identifier) instead of the protocol's default.
    ///
    /// # Errors
    ///
    /// [`WalletError`] when the protocol is unknown or the message cannot be sent.
    pub fn request_for(
        &mut self,
        protocol: &str,
        holder: &str,
        claims: &[&str],
        types: &[&str],
        purpose: Option<&str>,
        audience: Option<&str>,
    ) -> Result<String, WalletError> {
        let id = format!("x-{}", nonce()?);
        let n = nonce()?;
        let claims: Claims = claims.iter().map(|c| (*c).to_owned()).collect();
        let audience = match audience {
            Some(a) => a.to_owned(),
            None => self
                .wallet
                .registry()
                .protocol(protocol)?
                .audience(self.wallet.did()),
        };
        self.requests.insert(
            id.clone(),
            PresentationRequest {
                verifier: audience,
                nonce: n.clone(),
                claims: claims.clone(),
                types: types.iter().map(|t| (*t).to_owned()).collect(),
                purpose: purpose.map(str::to_owned),
            },
        );
        self.start(
            protocol,
            Role::Verifier,
            &id,
            holder,
            event(
                "exchange.start",
                json!({"exchange": id, "peer": holder, "nonce": n, "claims": claims}),
            ),
        )?;
        Ok(id)
    }

    /// As issuer: revokes the credential issued in exchange `id` through `revoke`, which sets
    /// its status entry, and records the revocation.
    ///
    /// # Errors
    ///
    /// [`WalletError::UnknownCredential`] when nothing was issued in `id`.
    pub fn revoke(&mut self, id: &str, revoke: impl FnOnce(&StatusRef)) -> Result<(), WalletError> {
        let meta = self
            .issued
            .get(id)
            .cloned()
            .ok_or_else(|| WalletError::UnknownCredential(id.to_owned()))?;
        if let Some(s) = &meta.status {
            revoke(s);
        }
        self.record(Line::Revoke {
            issuer: self.wallet.did().to_owned(),
            credential: meta.digest,
        });
        Ok(())
    }

    /// As holder: deletes a credential and journals the deletion (digest only).
    ///
    /// # Errors
    ///
    /// [`WalletError::Storage`].
    pub fn delete(&mut self, digest: &str) -> Result<(), WalletError> {
        self.wallet.delete(digest)?;
        self.host.send(
            &self.wallet_actor.clone(),
            event("credential.deleted", json!({"digest": digest})),
        );
        self.settle()
    }

    /// Runs the host until idle and handles every new `wire.*` send.
    ///
    /// # Errors
    ///
    /// [`WalletError`] when an outbound step fails.
    pub fn settle(&mut self) -> Result<(), WalletError> {
        loop {
            self.host.run_until_idle();
            let sent: Vec<_> = self.host.outbox()[self.outbox_seen..].to_vec();
            if sent.is_empty() {
                return Ok(());
            }
            self.outbox_seen += sent.len();
            for s in sent {
                if s.target == "wire:out" {
                    let data = vertep_agent::json::to_json(&s.event.data);
                    self.outbound(&s.from, &s.event.name, &data)?;
                }
            }
        }
    }

    fn info_by_actor(&self, actor: &str) -> Result<ExchangeInfo, WalletError> {
        self.exchanges
            .values()
            .find(|i| i.actor == actor)
            .cloned()
            .ok_or_else(|| WalletError::Protocol(format!("send from unknown actor {actor}")))
    }

    fn ctx(&mut self, actor: &str) -> Result<serde_json::Value, WalletError> {
        let s = self
            .host
            .state(actor)
            .map_err(|e| WalletError::Protocol(e.to_string()))?;
        let dm = vertep_agent::decider::ContextView::new(s.datamodel);
        Ok(vertep_agent::json::to_json(dm.data()))
    }

    fn send(&mut self, info: &ExchangeInfo, msg: &str, body: Body) -> Result<(), WalletError> {
        let p = self.wallet.registry().protocol(&info.protocol)?;
        let m = ExchangeMessage {
            exchange: info.id.clone(),
            from: self.wallet.did().to_owned(),
            to: info.peer.clone(),
            body,
        };
        let bytes = p.encode(&m)?;
        self.wire.send(&info.protocol, &m.from, &m.to, msg, &bytes)
    }

    fn outbound(&mut self, actor: &str, name: &str, data: &Value) -> Result<(), WalletError> {
        let info = self.info_by_actor(actor)?;
        let ctx = self.ctx(actor)?;
        let me = self.wallet.did().to_owned();
        let s = |v: &Value, k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let msg = self.next_msg();
        match name {
            "wire.offer" => {
                let offer = self
                    .offers
                    .get(&info.id)
                    .cloned()
                    .ok_or_else(|| WalletError::Protocol("offer without terms".into()))?;
                let n = s(&ctx, "nonce");
                self.record(Line::Offer {
                    issuer: me.clone(),
                    to: info.peer.clone(),
                    nonce: n.clone(),
                    msg: msg.clone(),
                });
                self.send(
                    &info,
                    &msg,
                    Body::Offer {
                        issuer: me,
                        nonce: n,
                        types: offer.types,
                        format: offer.format,
                    },
                )
            }
            "wire.pop" => {
                let issuer = s(&ctx, "peer");
                let n = s(&ctx, "nonce");
                let proof = self.wallet.prove_possession(&issuer, &n)?;
                self.record(Line::ProvePossession {
                    holder: me,
                    key: self.wallet.holder()?.did_key_id(),
                    issuer,
                    nonce: n,
                    msg: msg.clone(),
                });
                self.send(&info, &msg, Body::CredentialRequest { proof })
            }
            "wire.issue" => {
                let offer = self
                    .offers
                    .get(&info.id)
                    .cloned()
                    .ok_or_else(|| WalletError::Protocol("offer without terms".into()))?;
                let key = s(data, "key");
                let holder_key = crate::credential::DidKeyOnly.key_of(&key)?;
                let (meta, cred) = self.wallet.issue(
                    &offer.format,
                    &IssueRequest {
                        types: offer.types,
                        claims: offer.claims,
                        holder_key,
                        valid_from: self.wallet.now(),
                        valid_until: offer.valid_until,
                        status: offer.status,
                    },
                )?;
                self.record(Line::Issue {
                    issuer: me,
                    ikey: meta.issuer_key.clone(),
                    credential: meta.digest.clone(),
                    key,
                    nonce: s(&ctx, "nonce"),
                    claims: meta.claims.clone(),
                    msg: msg.clone(),
                });
                self.issued.insert(info.id.clone(), meta);
                self.offers.remove(&info.id);
                self.send(
                    &info,
                    &msg,
                    Body::Credential {
                        format: cred.format,
                        credential: cred.raw,
                    },
                )
            }
            "wire.request" => {
                let req = self
                    .requests
                    .get(&info.id)
                    .cloned()
                    .ok_or_else(|| WalletError::Protocol("request without terms".into()))?;
                self.record(Line::RequestPresentation {
                    verifier: me,
                    to: info.peer.clone(),
                    nonce: req.nonce.clone(),
                    claims: req.claims.clone(),
                    msg: msg.clone(),
                });
                self.send(
                    &info,
                    &msg,
                    Body::PresentationRequest {
                        verifier: req.verifier,
                        nonce: req.nonce,
                        claims: req.claims,
                        types: req.types,
                        purpose: req.purpose,
                    },
                )
            }
            "wire.present" => {
                let verifier = s(&ctx, "peer");
                let n = s(&ctx, "nonce");
                let digest = s(data, "credential");
                let claims = claims_of(data.get("claims").unwrap_or(&Value::Null));
                let agent = agent_of(&verifier);
                self.record(Line::Consent {
                    holder: me.clone(),
                    verifier: agent.clone(),
                    nonce: n.clone(),
                    claims: claims.clone(),
                });
                match self.wallet.present(&digest, &claims, &verifier, &n) {
                    Ok((meta, p)) => {
                        self.record(Line::Present {
                            holder: me,
                            verifier: agent,
                            nonce: n,
                            credential: meta.digest,
                            claims,
                            msg: msg.clone(),
                        });
                        self.send(
                            &info,
                            &msg,
                            Body::Presentation {
                                format: p.format,
                                presentation: p.raw,
                            },
                        )
                    }
                    Err(e) => {
                        self.record(Line::Send {
                            from: me,
                            msg: msg.clone(),
                        });
                        self.send(
                            &info,
                            &msg,
                            Body::Problem {
                                reason: e.check().into(),
                            },
                        )
                    }
                }
            }
            "wire.refuse" | "wire.problem" => {
                self.record(Line::Send {
                    from: me,
                    msg: msg.clone(),
                });
                self.send(
                    &info,
                    &msg,
                    Body::Problem {
                        reason: s(data, "reason"),
                    },
                )
            }
            "wire.verify" => {
                let result = s(data, "result");
                let reason = s(data, "reason");
                self.record(Line::Verify {
                    verifier: me.clone(),
                    msg: s(data, "msg"),
                    result: result.clone(),
                    reason: (!reason.is_empty()).then(|| reason.clone()),
                });
                self.record(Line::Send {
                    from: me,
                    msg: msg.clone(),
                });
                self.send(
                    &info,
                    &msg,
                    Body::Result {
                        accepted: result == "accept",
                        reason: (!reason.is_empty()).then_some(reason),
                    },
                )
            }
            other => Err(WalletError::Protocol(format!(
                "unknown wire command {other}"
            ))),
        }
    }

    fn live_actor(&mut self, role: Role, id: &str) -> Option<ExchangeInfo> {
        let info = self.exchange(role, id)?.clone();
        let terminated = self
            .host
            .state(&info.actor)
            .map(|s| s.terminated)
            .unwrap_or(true);
        (!terminated).then_some(info)
    }

    /// Handles wire message `msg_id` of `protocol`.
    ///
    /// # Errors
    ///
    /// [`WalletError::Unmapped`] when the protocol does not map the message onto the exchange
    /// vocabulary (nothing is journaled), and other errors when the message cannot be handled.
    pub fn receive(
        &mut self,
        protocol: &str,
        msg_id: &str,
        bytes: &[u8],
    ) -> Result<(), WalletError> {
        let p = self.wallet.registry().protocol(protocol)?;
        let m = p.decode(bytes)?;
        if m.to != self.wallet.did() {
            // misdelivered: the envelope names another recipient; the content is judged below
            // only by the roles' own checks, never trusted for routing elsewhere
        }
        let me = self.wallet.did().to_owned();
        match m.body {
            Body::Offer {
                issuer,
                nonce,
                types,
                format,
            } => {
                if !self.auto_accept_offers || self.exchange(Role::Holder, &m.exchange).is_some() {
                    return Ok(());
                }
                self.start(
                    protocol,
                    Role::Holder,
                    &m.exchange,
                    &issuer,
                    event(
                        "exchange.offer",
                        json!({"exchange": m.exchange, "issuer": issuer, "nonce": nonce,
                               "types": types, "format": format, "msg": msg_id}),
                    ),
                )
            }
            Body::CredentialRequest { proof } => {
                let Some(info) = self.live_actor(Role::Issuer, &m.exchange) else {
                    let msg = self.next_msg();
                    self.record(Line::Send {
                        from: me,
                        msg: msg.clone(),
                    });
                    // an unknown or finished offer: its nonce is unknown or used
                    return self.reply_problem(protocol, &m.exchange, &m.from, &msg, "nonce");
                };
                let ctx = self.ctx(&info.actor)?;
                let n = ctx
                    .get("nonce")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let ev = match self.wallet.check_possession(&proof, &n) {
                    // an offer is unsigned and can be replayed to anyone: a did:key holder must
                    // prove possession of its own key
                    Ok(key)
                        if info.peer.starts_with("did:key:")
                            && vertep_federation::resolver::key::did_key(&key) != info.peer =>
                    {
                        event(
                            "possession.invalid",
                            json!({"reason": "holder", "msg": msg_id}),
                        )
                    }
                    Ok(key) => event(
                        "possession.valid",
                        json!({"key": vertep_federation::resolver::key::did_key(&key), "msg": msg_id}),
                    ),
                    Err(e) => event(
                        "possession.invalid",
                        json!({"reason": e.check(), "msg": msg_id}),
                    ),
                };
                self.host.send(&info.actor, ev);
                self.settle()
            }
            Body::Credential { format, credential } => {
                let Some(info) = self.live_actor(Role::Holder, &m.exchange) else {
                    return Ok(());
                };
                let ctx = self.ctx(&info.actor)?;
                let issuer = ctx
                    .get("peer")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let cred = Credential {
                    format,
                    raw: credential,
                };
                let ev = match self.wallet.accept(&cred, &issuer) {
                    Ok(meta) => {
                        self.record(Line::Store {
                            holder: me,
                            credential: meta.digest.clone(),
                        });
                        self.host.send(
                            &self.wallet_actor.clone(),
                            event("credential.stored", meta_json(&meta)),
                        );
                        event(
                            "credential.valid",
                            json!({"credential": meta.digest, "msg": msg_id}),
                        )
                    }
                    Err(e) => event(
                        "credential.invalid",
                        json!({"reason": e.check(), "msg": msg_id}),
                    ),
                };
                self.host.send(&info.actor, ev);
                self.settle()
            }
            Body::PresentationRequest {
                verifier,
                nonce,
                claims,
                types,
                purpose,
            } => {
                if self.exchange(Role::Holder, &m.exchange).is_some() {
                    return Ok(());
                }
                let req = PresentationRequest {
                    verifier: verifier.clone(),
                    nonce: nonce.clone(),
                    claims: claims.clone(),
                    types,
                    purpose: purpose.clone(),
                };
                let options = self.wallet.options(&req)?;
                // messages go to the verifier's DID; the chart keeps the audience (client id)
                let peer = agent_of(&verifier);
                self.start(
                    protocol,
                    Role::Holder,
                    &m.exchange,
                    &peer,
                    event(
                        "exchange.request",
                        json!({"exchange": m.exchange, "verifier": verifier, "nonce": nonce,
                               "claims": claims, "purpose": purpose.unwrap_or_default(),
                               "options": options, "msg": msg_id}),
                    ),
                )
            }
            Body::Presentation {
                format,
                presentation,
            } => {
                let Some(info) = self.live_actor(Role::Verifier, &m.exchange) else {
                    self.record(Line::Verify {
                        verifier: me,
                        msg: msg_id.to_owned(),
                        result: "refuse".into(),
                        reason: Some("nonce".into()),
                    });
                    return Ok(());
                };
                let req = self
                    .requests
                    .get(&m.exchange)
                    .cloned()
                    .ok_or_else(|| WalletError::Protocol("request without terms".into()))?;
                let pres = Presentation {
                    format,
                    raw: presentation,
                };
                let ev = match self.wallet.verify_presentation(&pres, &req) {
                    Ok(v) => {
                        let names: Vec<String> = v.disclosed.keys().cloned().collect();
                        self.verified.insert(
                            m.exchange.clone(),
                            Verified {
                                issuer: v.credential.issuer.clone(),
                                credential: v.credential.digest.clone(),
                                disclosed: v.disclosed,
                                holder: v.signer.clone(),
                                types: v.credential.types.clone(),
                                status: v.credential.status.clone(),
                            },
                        );
                        event(
                            "presentation.valid",
                            json!({"msg": msg_id, "credential": v.credential.digest, "claims": names}),
                        )
                    }
                    Err(e) => event(
                        "presentation.invalid",
                        json!({"msg": msg_id, "reason": e.check()}),
                    ),
                };
                self.host.send(&info.actor, ev);
                self.settle()
            }
            Body::Problem { reason } => {
                for role in [Role::Holder, Role::Verifier] {
                    if let Some(info) = self.live_actor(role, &m.exchange)
                        && info.peer == m.from
                    {
                        self.host.send(
                            &info.actor,
                            event("exchange.problem", json!({"reason": reason})),
                        );
                    }
                }
                self.settle()
            }
            Body::Result { .. } => Ok(()),
        }
    }

    fn reply_problem(
        &mut self,
        protocol: &str,
        exchange: &str,
        to: &str,
        msg: &str,
        reason: &str,
    ) -> Result<(), WalletError> {
        let p = self.wallet.registry().protocol(protocol)?;
        let m = ExchangeMessage {
            exchange: exchange.to_owned(),
            from: self.wallet.did().to_owned(),
            to: to.to_owned(),
            body: Body::Problem {
                reason: reason.to_owned(),
            },
        };
        let bytes = p.encode(&m)?;
        self.wire.send(protocol, &m.from, to, msg, &bytes)
    }
}

/// A credential's metadata as journaled by the wallet actor (no claim values).
pub fn meta_json(meta: &CredentialMeta) -> Value {
    json!({
        "digest": meta.digest,
        "format": meta.format,
        "types": meta.types,
        "issuer": meta.issuer,
        "validFrom": meta.valid_from,
        "validUntil": meta.valid_until,
        "status": meta.status.as_ref().map(|s| json!({"list": s.list, "index": s.index, "purpose": s.purpose})),
    })
}

/// Reads a built-in chart.
///
/// # Errors
///
/// [`WalletError::Protocol`] when the chart does not read.
pub fn read_chart(xml: &str) -> Result<vertep_core::ir::Chart, WalletError> {
    let opts = vertep_scxml::ReadOptions::new()
        .with_datamodels(&vertep_scxml::AnyDatamodel)
        .with_resolver(&vertep_scxml::DeferResolver);
    vertep_scxml::read(xml, &opts)
        .map(|r| r.chart)
        .map_err(|e| WalletError::Protocol(format!("chart: {e:?}")))
}

impl crate::credential::DidKeyOnly {
    /// The public key of a `did:key`.
    ///
    /// # Errors
    ///
    /// [`WalletError`] when it is not a `did:key`.
    pub fn key_of(&self, did: &str) -> Result<PublicKey, WalletError> {
        use crate::credential::KeyLookup;
        self.key(did)
    }
}
