//! Test helpers (feature `testkit`): hosts with in-memory keys, a static host table, and a
//! custody fixture of custodians in one process with switchable links.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxrdf::{Literal, NamedNode, Triple};
use vertep_federation::{InMemoryKeyManager, KeyManager, KeyType, PublicKey};
use vertep_store::{Entry, JournalStore, MemoryStore};

use crate::chain::{HostKeys, KeyManagerSigner};
use crate::custody::{
    Backoff, Custodian, CustodyConfig, CustodyHost, LocalTransport, Replica, StaticKeyDirectory,
};
use crate::identity::DurableObjectId;

/// A party with a DID, an Ed25519 signing key and an X25519 key-agreement key in an
/// in-memory key manager.
#[derive(Clone, Debug)]
pub struct TestHost {
    /// The DID.
    pub did: String,
    /// The signer.
    pub signer: KeyManagerSigner,
    /// The verification key.
    pub public: PublicKey,
    /// The key manager.
    pub keys: Arc<InMemoryKeyManager>,
    /// The name of the key-agreement key.
    pub agreement: String,
    /// The key-agreement public key.
    pub agreement_public: PublicKey,
}

impl TestHost {
    /// A party named `did` with fresh keys.
    ///
    /// # Panics
    ///
    /// When the in-memory key manager cannot generate a key.
    pub fn new(did: &str) -> Self {
        let km = Arc::new(InMemoryKeyManager::new());
        let key = km.generate(KeyType::Ed25519).expect("generate key");
        let public = km.public_key(&key).expect("public key");
        let agreement = km.generate(KeyType::X25519).expect("generate key");
        let agreement_public = km.public_key(&agreement).expect("public key");
        TestHost {
            did: did.to_owned(),
            signer: KeyManagerSigner::new(did, km.clone() as Arc<dyn KeyManager>, key),
            public,
            keys: km,
            agreement,
            agreement_public,
        }
    }
}

/// Host keys from a table of hosts and the epoch each writes from.
#[derive(Clone, Debug, Default)]
pub struct StaticHostKeys {
    hosts: BTreeMap<String, (u64, Vec<PublicKey>)>,
}

impl StaticHostKeys {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// `host` writes from epoch `from` on.
    pub fn with(mut self, host: &TestHost, from: u64) -> Self {
        self.hosts
            .insert(host.did.clone(), (from, vec![host.public]));
        self
    }
}

impl HostKeys for StaticHostKeys {
    fn host_keys(&self, _object: &NamedNode, epoch: u64, writer: &str) -> Option<Vec<PublicKey>> {
        let (from, keys) = self.hosts.get(writer)?;
        (epoch >= *from).then(|| keys.clone())
    }
}

/// A runtime-like turn entry of `object` named `<object>/entry/<n>` with a note.
pub fn turn(object: &NamedNode, n: u64, note: &str) -> Entry {
    let name = NamedNode::new_unchecked(format!("{}/entry/{n}", object.as_str()));
    Entry::new(
        name.clone(),
        [Triple::new(
            name,
            NamedNode::new_unchecked("https://w3id.org/vertep/vt#note"),
            Literal::new_simple_literal(note),
        )],
    )
}

/// One custodian of the fixture.
#[derive(Clone, Debug)]
pub struct Party {
    /// Its identity and keys.
    pub id: TestHost,
    /// Its own journal store.
    pub store: Arc<MemoryStore>,
    /// Its replica of the object.
    pub replica: Arc<Replica>,
    /// The in-process link to it.
    pub transport: Arc<LocalTransport>,
}

/// A durable object with custodians in one process.
#[derive(Debug)]
pub struct CustodyFixture {
    /// The object.
    pub object: DurableObjectId,
    /// Every party's keys, and the controller's.
    pub directory: Arc<StaticKeyDirectory>,
    /// The custodians, in configuration order.
    pub parties: Vec<Party>,
    /// The configuration of the DID document.
    pub config: CustodyConfig,
    /// The object's controller.
    pub controller: TestHost,
}

impl CustodyFixture {
    /// An object with `hosts` and `keepers` (DIDs) and the given quorum (the smallest majority
    /// when `None`).
    ///
    /// # Panics
    ///
    /// For an invalid configuration.
    pub fn new(hosts: &[&str], keepers: &[&str], quorum: Option<usize>) -> Self {
        let object = DurableObjectId::new("did:webvh:QmScid:example.org:objects:ticket-8812")
            .expect("valid object DID");
        Self::for_object(
            object,
            TestHost::new("did:web:controller.example.org"),
            hosts,
            keepers,
            quorum,
        )
    }

    /// As [`CustodyFixture::new`], for the object `object` controlled by `controller`.
    ///
    /// # Panics
    ///
    /// For an invalid configuration.
    pub fn for_object(
        object: DurableObjectId,
        controller: TestHost,
        hosts: &[&str],
        keepers: &[&str],
        quorum: Option<usize>,
    ) -> Self {
        let directory = Arc::new(StaticKeyDirectory::new());
        directory.add_controller(object.as_str(), controller.public);
        let custodians: Vec<Custodian> = hosts
            .iter()
            .map(|h| Custodian::host(*h))
            .chain(keepers.iter().map(|k| Custodian::keeper(*k)))
            .collect();
        let config = match quorum {
            Some(q) => CustodyConfig::new(custodians, q),
            None => CustodyConfig::majority(custodians),
        }
        .expect("valid configuration");
        let mut f = CustodyFixture {
            object,
            directory,
            parties: Vec::new(),
            config,
            controller,
        };
        let dids: Vec<String> = f.config.custodians.iter().map(|c| c.id.clone()).collect();
        for d in dids {
            let p = f.make_party(&d);
            f.parties.push(p);
        }
        f
    }

    /// A new party `did` (not yet in the configuration), known to the directory.
    ///
    /// # Panics
    ///
    /// When its replica cannot be opened.
    pub fn make_party(&self, did: &str) -> Party {
        let id = TestHost::new(did);
        self.directory.add_verification(did, id.public);
        self.directory.set_agreement(did, id.agreement_public);
        self.party_with(id)
    }

    /// Moves the fixture to the object `object` (a DID created after the custodians' keys, as
    /// a `did:webvh` whose witnesses are the custodians): every party keeps its keys and gets a
    /// fresh store and replica of the new object; the controller's key is registered for it.
    ///
    /// # Panics
    ///
    /// When a replica cannot be opened.
    pub fn rebind(&mut self, object: DurableObjectId) {
        self.object = object;
        self.directory
            .add_controller(self.object.as_str(), self.controller.public);
        let ids: Vec<TestHost> = self.parties.iter().map(|p| p.id.clone()).collect();
        self.parties = ids.into_iter().map(|id| self.party_with(id)).collect();
    }

    fn party_with(&self, id: TestHost) -> Party {
        let store = Arc::new(MemoryStore::new());
        let replica = Arc::new(
            Replica::open(
                self.object.iri(),
                Arc::new(id.signer.clone()),
                store.clone() as Arc<dyn JournalStore>,
                self.directory.clone(),
                self.config.clone(),
            )
            .expect("open replica"),
        );
        let transport = Arc::new(LocalTransport::new(replica.clone()));
        Party {
            id,
            store,
            replica,
            transport,
        }
    }

    /// The party `did`.
    ///
    /// # Panics
    ///
    /// When there is none.
    pub fn party(&self, did: &str) -> &Party {
        self.parties
            .iter()
            .find(|p| p.id.did == did)
            .unwrap_or_else(|| panic!("no party {did}"))
    }

    /// Connects or disconnects `did` from everyone.
    pub fn set_up(&self, did: &str, up: bool) {
        self.party(did).transport.set_up(up);
    }

    /// Restarts `did`: its replica is reopened from its store (its links are kept).
    ///
    /// # Panics
    ///
    /// When the replica cannot be reopened.
    pub fn restart(&mut self, did: &str) {
        let i = self
            .parties
            .iter()
            .position(|p| p.id.did == did)
            .expect("party");
        let p = &self.parties[i];
        let replica = Arc::new(
            Replica::open(
                self.object.iri(),
                Arc::new(p.id.signer.clone()),
                p.store.clone() as Arc<dyn JournalStore>,
                self.directory.clone(),
                self.config.clone(),
            )
            .expect("reopen replica"),
        );
        let transport = Arc::new(LocalTransport::new(replica.clone()));
        self.parties[i].replica = replica;
        self.parties[i].transport = transport;
    }

    /// The custody host of the host party `did`, linked to every other party, electing without
    /// waiting.
    pub fn custody_host(&self, did: &str) -> Arc<CustodyHost> {
        let me = self.party(did);
        let mut h = CustodyHost::new(
            self.object.clone(),
            Arc::new(me.id.signer.clone()),
            me.id.keys.clone() as Arc<dyn KeyManager>,
            me.id.agreement.clone(),
            me.replica.clone(),
            self.directory.clone(),
        )
        .with_backoff(Backoff::immediate(4));
        for p in &self.parties {
            if p.id.did != did {
                h = h.with_transport(p.transport.clone());
            }
        }
        Arc::new(h)
    }
}
