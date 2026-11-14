//! The conformance kit (`credential-exchange` requirement `Plugin Conformance`, design D6).
//!
//! The kit owns the network between the parties' exchange engines. Messages are queued; a
//! scheduler delivers them honestly or, from a seed, adversarially: dropping, duplicating,
//! reordering, replaying a message to another party, and injecting garbage. Every network
//! event and every vocabulary step lands in one [`History`], which the Lean checker
//! `vertep-wallet` replays against the exchange model. A protocol plugin is conformant when the
//! checker accepts every history the kit records for it.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::error::WalletError;
use crate::exchange::{Exchanges, Wire};
use crate::history::{AgentKeys, History, Line, Term, Trust};

/// A message in flight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    /// The protocol IRI.
    pub protocol: String,
    /// The sender.
    pub from: String,
    /// The addressed recipient.
    pub to: String,
    /// The message id.
    pub msg: String,
    /// The encoded message.
    pub bytes: Vec<u8>,
}

#[derive(Default)]
struct Net {
    queue: VecDeque<Packet>,
    seen: Vec<Packet>,
}

/// The kit's side of the wire: engines send into its queue.
#[derive(Clone, Default)]
pub struct KitWire(Arc<Mutex<Net>>);

impl std::fmt::Debug for KitWire {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KitWire")
    }
}

impl Wire for KitWire {
    fn send(
        &self,
        protocol: &str,
        from: &str,
        to: &str,
        msg_id: &str,
        bytes: &[u8],
    ) -> Result<(), WalletError> {
        let p = Packet {
            protocol: protocol.to_owned(),
            from: from.to_owned(),
            to: to.to_owned(),
            msg: msg_id.to_owned(),
            bytes: bytes.to_vec(),
        };
        let mut n = self
            .0
            .lock()
            .map_err(|_| WalletError::Protocol("network poisoned".into()))?;
        n.seen.push(p.clone());
        n.queue.push_back(p);
        Ok(())
    }
}

/// What the adversary may do, with probabilities in percent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedule {
    /// The seed.
    pub seed: u64,
    /// Drop a message.
    pub drop: u8,
    /// Deliver a message twice.
    pub duplicate: u8,
    /// Deliver a message out of order.
    pub reorder: u8,
    /// Also deliver a message to another party.
    pub replay: u8,
    /// Inject a garbage message.
    pub inject: u8,
}

impl Schedule {
    /// Honest delivery, in order.
    pub fn honest() -> Self {
        Schedule {
            seed: 0,
            drop: 0,
            duplicate: 0,
            reorder: 0,
            replay: 0,
            inject: 0,
        }
    }

    /// An adversarial schedule from `seed` (no drops, so exchanges can complete).
    pub fn adversarial(seed: u64) -> Self {
        Schedule {
            seed,
            drop: 0,
            duplicate: 30,
            reorder: 30,
            replay: 30,
            inject: 15,
        }
    }

    /// An adversarial schedule that also drops messages.
    pub fn lossy(seed: u64) -> Self {
        Schedule {
            drop: 15,
            ..Schedule::adversarial(seed)
        }
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0.max(1);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn chance(&mut self, pct: u8) -> bool {
        (self.next() % 100) < u64::from(pct)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// The conformance kit: parties, the network and the history.
pub struct Kit {
    wire: KitWire,
    history: History,
    parties: BTreeMap<String, Exchanges>,
    compromised: Vec<String>,
    injected: u64,
}

impl std::fmt::Debug for Kit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kit")
            .field("parties", &self.parties.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Default for Kit {
    fn default() -> Self {
        Self::new()
    }
}

// @tg: implements:: [[openspec:credential-exchange#Plugin Conformance]]
impl Kit {
    /// An empty kit.
    pub fn new() -> Self {
        Kit {
            wire: KitWire::default(),
            history: History::new(),
            parties: BTreeMap::new(),
            compromised: Vec::new(),
            injected: 0,
        }
    }

    /// The wire engines must send through.
    pub fn wire(&self) -> Arc<dyn Wire> {
        Arc::new(self.wire.clone())
    }

    /// The history engines must record into.
    pub fn history(&self) -> History {
        self.history.clone()
    }

    /// Adds a party.
    pub fn add(&mut self, e: Exchanges) {
        self.parties.insert(e.wallet().did().to_owned(), e);
    }

    /// The party `did`.
    ///
    /// # Panics
    ///
    /// When there is no such party.
    pub fn party(&mut self, did: &str) -> &mut Exchanges {
        self.parties
            .get_mut(did)
            .unwrap_or_else(|| panic!("no party {did}"))
    }

    /// Marks a key as compromised (the adversary can sign with it).
    pub fn compromise(&mut self, key: &str) {
        self.compromised.push(key.to_owned());
    }

    /// The setup line: every party, its keys (issuer key and holder key) and trust.
    pub fn setup(&self) -> Line {
        let mut agents = Vec::new();
        let mut trust = Vec::new();
        for (did, e) in &self.parties {
            let w = e.wallet();
            let mut keys = vec![w.config().kid.clone()];
            if let Ok(h) = w.holder() {
                let id = h.did_key_id();
                if !keys.contains(&id) {
                    keys.push(id);
                }
            }
            agents.push(AgentKeys {
                id: did.clone(),
                keys,
            });
            for i in &w.config().trusted_issuers {
                trust.push(Trust {
                    verifier: did.clone(),
                    issuer: i.clone(),
                });
            }
        }
        Line::Setup {
            agents,
            trust,
            compromised: self.compromised.clone(),
        }
    }

    /// The history as JSON Lines, setup first.
    pub fn jsonl(&self) -> String {
        let mut out = serde_json::to_string(&self.setup()).unwrap_or_default();
        out.push('\n');
        out.push_str(&self.history.to_jsonl());
        out
    }

    fn deliver(&mut self, p: &Packet, to: &str) {
        self.history.push(Line::Deliver {
            msg: p.msg.clone(),
            to: to.to_owned(),
        });
        if let Some(e) = self.parties.get_mut(to) {
            // a party's refusal to handle a message is its business; the history shows it
            let _ = e.receive(&p.protocol, &p.msg, &p.bytes);
        }
    }

    /// Delivers until the network is quiet, following `schedule`; returns the number of
    /// network events.
    pub fn run(&mut self, schedule: Schedule) -> usize {
        let mut rng = Rng(schedule.seed ^ 0x9E37_79B9_7F4A_7C15);
        let mut events = 0;
        for _ in 0..10_000 {
            let next = {
                let Ok(mut n) = self.wire.0.lock() else {
                    break;
                };
                if n.queue.is_empty() {
                    None
                } else if schedule.reorder > 0 && n.queue.len() > 1 && rng.chance(schedule.reorder)
                {
                    let i = rng.below(n.queue.len());
                    n.queue.remove(i)
                } else {
                    n.queue.pop_front()
                }
            };
            let Some(p) = next else {
                break;
            };
            events += 1;
            if rng.chance(schedule.drop) {
                self.history.push(Line::Drop { msg: p.msg.clone() });
                continue;
            }
            let to = p.to.clone();
            self.deliver(&p, &to);
            if rng.chance(schedule.duplicate) {
                self.history.push(Line::Duplicate { msg: p.msg.clone() });
                self.deliver(&p, &to);
                events += 1;
            }
            if rng.chance(schedule.replay) {
                let others: Vec<String> =
                    self.parties.keys().filter(|d| **d != to).cloned().collect();
                if !others.is_empty() {
                    let other = others[rng.below(others.len())].clone();
                    self.deliver(&p, &other);
                    events += 1;
                }
            }
            if rng.chance(schedule.inject) {
                self.injected += 1;
                let g = Packet {
                    msg: format!("adv-{}", self.injected),
                    bytes: b"{\"type\":\"garbage\"}".to_vec(),
                    ..p.clone()
                };
                self.history.push(Line::Inject {
                    msg: g.msg.clone(),
                    term: Term::Other,
                });
                self.deliver(&g, &to);
                events += 1;
            }
        }
        events
    }

    /// Delivers the already-sent message `msg` again to `to` (a replay the test chooses).
    ///
    /// # Panics
    ///
    /// When `msg` was never sent.
    pub fn replay(&mut self, msg: &str, to: &str) {
        let p = self
            .wire
            .0
            .lock()
            .ok()
            .and_then(|n| n.seen.iter().find(|p| p.msg == msg).cloned())
            .unwrap_or_else(|| panic!("no message {msg}"));
        self.deliver(&p, to);
    }

    /// Every message sent so far.
    pub fn sent(&self) -> Vec<Packet> {
        self.wire
            .0
            .lock()
            .map(|n| n.seen.clone())
            .unwrap_or_default()
    }

    /// Replays the history through the Lean checker, if it is built.
    pub fn check(&self) -> Option<Result<String, String>> {
        check_history(&self.jsonl())
    }
}

/// The Lean checker `vertep-wallet`: `VT_LEAN_BIN/vertep-wallet` or the package build.
pub fn lean_checker() -> Option<PathBuf> {
    let dir = std::env::var_os("VT_LEAN_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lean/vertep-lean/.lake/build/bin")
        });
    let bin = dir.join("vertep-wallet");
    bin.is_file().then_some(bin)
}

/// Replays `jsonl` through the Lean checker: `None` when it is not built, `Ok` with its report
/// when it accepts the history, `Err` with its report when it rejects it.
pub fn check_history(jsonl: &str) -> Option<Result<String, String>> {
    let bin = lean_checker()?;
    let dir = std::env::temp_dir().join(format!(
        "vertep-wallet-history-{}-{}",
        std::process::id(),
        crate::credential::digest(jsonl.as_bytes())[7..19].to_owned()
    ));
    std::fs::create_dir_all(&dir).ok()?;
    let file = dir.join("history.jsonl");
    std::fs::write(&file, jsonl).ok()?;
    let out = std::process::Command::new(bin)
        .arg("--history")
        .arg(&file)
        .output()
        .ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Some(if out.status.success() {
        Ok(report)
    } else {
        Err(report)
    })
}

/// A ready-made party for conformance runs: a `did:key` wallet with a sealed in-memory store,
/// the built-in plugins, and `consent` as its policy.
///
/// # Errors
///
/// [`WalletError`] when the wallet cannot be created.
pub fn party(
    kit: &Kit,
    clock: &vertep_runtime::VirtualClock,
    consent: crate::consent::RulePolicy,
) -> Result<Exchanges, WalletError> {
    use vertep_federation::{InMemoryKeyManager, KeyManager, KeyType, RuntimeClock};
    let km = Arc::new(InMemoryKeyManager::new());
    let key = km.generate(KeyType::Ed25519)?;
    let registry = Arc::new(crate::plugin::WalletRegistry::with_builtins());
    let store = crate::store::SealedStore::open(
        km.as_ref(),
        &key,
        Arc::new(crate::store::MemoryRecords::new()),
    )?;
    registry.register_store(crate::vocab::STORE_SEALED, Arc::new(store));
    registry.register_consent(crate::vocab::CONSENT_RULES, Arc::new(consent));
    let mut config = crate::wallet::WalletConfig::did_key(km.as_ref(), &key)?;
    config.status = None;
    let km: Arc<dyn KeyManager> = km;
    let wallet = crate::wallet::Wallet::new(
        config,
        km,
        registry,
        Arc::new(crate::credential::DidKeyOnly),
        Arc::new(RuntimeClock(Arc::new(clock.clone()))),
    );
    Exchanges::builder(Arc::new(clock.clone()))
        .history(kit.history())
        .create(wallet, kit.wire())
}

/// The standard conformance scenario: an issuer issues an identity credential (SD-JWT VC) to a
/// holder over `issuing`, and two verifiers request presentations over `presenting`, all under
/// `schedule`.
///
/// # Errors
///
/// [`WalletError`] when a party cannot be created or an exchange cannot start.
pub fn scenario(issuing: &str, presenting: &str, schedule: Schedule) -> Result<Kit, WalletError> {
    use crate::consent::RulePolicy;
    let clock = vertep_runtime::VirtualClock::new(1_794_355_200_000);
    let mut kit = Kit::new();
    let issuer = party(&kit, &clock, RulePolicy::refuse_all())?;
    let holder = party(
        &kit,
        &clock,
        RulePolicy::refuse_all().allow("*", &["nationality", "birthDate"]),
    )?;
    let mut v1 = party(&kit, &clock, RulePolicy::refuse_all())?;
    let mut v2 = party(&kit, &clock, RulePolicy::refuse_all())?;
    let (i, h, a, b) = (
        issuer.wallet().did().to_owned(),
        holder.wallet().did().to_owned(),
        v1.wallet().did().to_owned(),
        v2.wallet().did().to_owned(),
    );
    v1.wallet_mut().trust(&i);
    v2.wallet_mut().trust(&i);
    for p in [issuer, holder, v1, v2] {
        kit.add(p);
    }
    let offer = crate::exchange::Offer {
        format: crate::vocab::FORMAT_SD_JWT.to_owned(),
        types: vec!["https://example.org/IdentityCredential".to_owned()],
        claims:
            serde_json::json!({"name": "Olena", "birthDate": "1990-05-01", "nationality": "UA"})
                .as_object()
                .cloned()
                .unwrap_or_default(),
        valid_until: None,
        status: None,
    };
    kit.party(&i).offer(issuing, &h, offer)?;
    kit.run(schedule);
    kit.party(&a)
        .request(presenting, &h, &["nationality"], &[], None)?;
    kit.party(&b)
        .request(presenting, &h, &["birthDate", "name"], &[], None)?;
    kit.run(schedule);
    Ok(kit)
}

/// The outcome of a conformance run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Histories recorded.
    pub histories: usize,
    /// Histories the Lean checker accepted (0 when it is not built).
    pub accepted: usize,
    /// Whether the Lean checker ran.
    pub checked: bool,
    /// Presentations verifiers accepted, over all histories.
    pub presentations: usize,
    /// The reports of rejected histories.
    pub rejected: Vec<String>,
}

/// Runs [`scenario`] over `seeds` adversarial and lossy schedules (and one honest one) and
/// checks every history with the Lean checker when it is built.
///
/// # Errors
///
/// [`WalletError`] when a scenario cannot run.
pub fn conformance(issuing: &str, presenting: &str, seeds: u64) -> Result<Report, WalletError> {
    let mut r = Report::default();
    let mut schedules = vec![Schedule::honest()];
    for s in 0..seeds {
        schedules.push(Schedule::adversarial(s));
        schedules.push(Schedule::lossy(s));
    }
    for s in schedules {
        let kit = scenario(issuing, presenting, s)?;
        r.histories += 1;
        r.presentations += kit
            .history()
            .lines()
            .iter()
            .filter(|l| matches!(l, Line::Verify { result, .. } if result == "accept"))
            .count();
        match kit.check() {
            Some(Ok(_)) => {
                r.checked = true;
                r.accepted += 1;
            }
            Some(Err(report)) => {
                r.checked = true;
                r.rejected.push(report);
            }
            None => {}
        }
    }
    Ok(r)
}
