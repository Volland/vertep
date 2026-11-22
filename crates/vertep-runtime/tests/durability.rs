//! Durable commits (M18 phase 1, design D2): the tiramemsu backend syncs each append by
//! default, a relaxed open is declared, and a dispatched send never outlives its entry.

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use oxrdf::NamedNode;
use vertep_runtime::mailbox::{Mailbox, MailboxError, MemoryMailbox};
use vertep_runtime::{
    Address, CommitDurability, CrashPoint, Envelope, Node, NodeConfig, Resolver, Shell, Storage,
    VirtualClock,
};
use vertep_store::{
    AppendError, AppendMeta, Entry, GraphStore, JournalCapabilities, JournalRecord, JournalStore,
    Position, StoreError,
};
use vertep_tiramemsu::{ShardDurability, TiramemsuBackend, TiramemsuShard};

// @tg: verifies:: [[openspec:storage#Durable Commit#Default is durable]] {level: integration}
#[test]
fn default_open_is_durable() {
    let dir = tempfile::tempdir().unwrap();
    let shard = TiramemsuShard::open(dir.path().join("s.db")).unwrap();
    assert!(
        JournalStore::capabilities(&shard).durable_commit,
        "a shard syncs each append"
    );
    let backend = TiramemsuBackend::open(dir.path().join("b"), 3).unwrap();
    assert!(backend.capabilities().durable_commit, "every shard syncs");

    let clock = VirtualClock::new(T0);
    let node = node_on(Arc::new(shard), &clock, NodeConfig::default());
    assert!(node.durable_commits());
    assert_eq!(node.durability(), CommitDurability::Durable);
    assert!(
        !node_on(
            Arc::new(vertep_store::MemoryStore::new()),
            &clock,
            NodeConfig::default()
        )
        .durable_commits(),
        "the memory store is volatile and says so"
    );
}

// @tg: verifies:: [[openspec:storage#Durable Commit#Relaxed mode is declared]] {level: integration}
#[test]
fn relaxed_open_is_declared() {
    let dir = tempfile::tempdir().unwrap();
    let shard =
        TiramemsuShard::open_durability(dir.path().join("s.db"), ShardDurability::Relaxed).unwrap();
    assert!(!JournalStore::capabilities(&shard).durable_commit);
    let backend =
        TiramemsuBackend::open_durability(dir.path().join("b"), 2, ShardDurability::Relaxed)
            .unwrap();
    assert!(!backend.capabilities().durable_commit);
    // caller-given options are taken as given: tiramemsu's own default is not durable
    let given = TiramemsuShard::open_with(
        dir.path().join("given.db"),
        vertep_tiramemsu::tiramemsu::OpenOptions::default(),
    )
    .unwrap();
    assert!(!JournalStore::capabilities(&given).durable_commit);

    let clock = VirtualClock::new(T0);
    let node = node_on(
        Arc::new(shard),
        &clock,
        NodeConfig {
            durability: CommitDurability::Relaxed,
            ..NodeConfig::default()
        },
    );
    assert!(!node.durable_commits());
    assert_eq!(node.durability(), CommitDurability::Relaxed);
    assert!(vertep_runtime::node::RELAXED_DURABILITY_WARNING.contains("operating-system crash"));
}

/// What happened, in order: appends that returned, and messages that reached the outside.
type Log = Arc<Mutex<Vec<String>>>;

/// A shard that logs each append once it has returned.
struct Logged {
    shard: TiramemsuShard,
    log: Log,
}

impl JournalStore for Logged {
    fn capabilities(&self) -> JournalCapabilities {
        JournalStore::capabilities(&self.shard)
    }
    fn append(
        &self,
        actor: &NamedNode,
        expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError> {
        let head = self.shard.append(actor, expected, entries, meta)?;
        self.log
            .lock()
            .unwrap()
            .push(format!("appended {actor} {}", head.get()));
        Ok(head)
    }
    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        self.shard.read(actor, from)
    }
    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        self.shard.head(actor)
    }
    fn actors(&self) -> Result<Vec<NamedNode>, StoreError> {
        self.shard.actors()
    }
}

impl Storage for Logged {
    fn journal(&self) -> &dyn JournalStore {
        self
    }
    fn graph(&self, _actor: &NamedNode) -> &dyn GraphStore {
        &self.shard
    }
    fn graphs(&self) -> Vec<&dyn GraphStore> {
        vec![&self.shard]
    }
}

/// `test:out`: a mailbox outside the node that logs each message it receives.
struct Outside {
    inbox: Arc<MemoryMailbox>,
    log: Log,
}

struct LoggingMailbox {
    inner: Arc<MemoryMailbox>,
    log: Log,
}

impl Mailbox for LoggingMailbox {
    fn enqueue(&self, env: Envelope) -> Result<(), MailboxError> {
        self.log
            .lock()
            .unwrap()
            .push(format!("dispatched {}", env.event.name));
        self.inner.enqueue(env)
    }
    fn receive(&self, deferred: &[String]) -> Option<Envelope> {
        self.inner.receive(deferred)
    }
    fn ack(&self, message_id: &str) -> Result<(), MailboxError> {
        self.inner.ack(message_id)
    }
    fn release(&self, message_id: &str) {
        self.inner.release(message_id)
    }
    fn release_all(&self) {
        self.inner.release_all()
    }
    fn pending(&self) -> Vec<Envelope> {
        self.inner.pending()
    }
    fn pending_deliverable(&self, deferred: &[String]) -> usize {
        self.inner.pending_deliverable(deferred)
    }
}

impl Resolver for Outside {
    fn resolve(&self, address: &Address) -> Option<Arc<dyn Mailbox>> {
        (address.as_str() == "test:out").then(|| {
            Arc::new(LoggingMailbox {
                inner: self.inbox.clone(),
                log: self.log.clone(),
            }) as Arc<dyn Mailbox>
        })
    }
}

fn node_with(storage: Arc<dyn Storage>, crash: Option<CrashPoint>) -> Node {
    let clock = VirtualClock::new(T0);
    let shell = Shell {
        spawn: Arc::new(vertep_runtime::shell::TokioSpawn),
        clock: Arc::new(clock.clone()),
        sleep: Arc::new(clock),
    };
    Node::builder(storage, shell)
        .datamodels(ecma())
        .config(NodeConfig {
            crash_at: crash,
            ..NodeConfig::default()
        })
        .build()
}

const SENDER: &str = r#"<state id="s"><transition event="go"><send event="one" target="test:out"/><send event="two" target="test:out"/></transition></state>"#;

// @tg: verifies:: [[openspec:event-journal#Transactional Outbox#No send outlives its entry]] {level: integration}
#[test]
fn no_send_outlives_its_entry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outbox.db");
    let log: Log = Arc::default();
    let a = va("sender", "a");
    {
        let storage = Arc::new(Logged {
            shard: TiramemsuShard::open(&path).unwrap(),
            log: log.clone(),
        });
        let mut node = node_with(storage.clone(), Some(CrashPoint::AfterDispatch));
        assert!(
            node.durable_commits(),
            "the default shard syncs each append"
        );
        node.handle().register_resolver(
            "test",
            Arc::new(Outside {
                inbox: Arc::new(MemoryMailbox::default()),
                log: log.clone(),
            }),
        );
        node.register_type(&ty("sender"), scxml("s", SENDER))
            .unwrap();
        node.send(&a, ev("go")).unwrap();
        node.run_until_idle();
        // the process dies here, right after the sends went out
    }
    let log = log.lock().unwrap().clone();
    let appended = log
        .iter()
        .position(|l| l == &format!("appended {} 1", a.node()))
        .expect("the entry was appended");
    let dispatched: Vec<usize> = log
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with("dispatched "))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(dispatched.len(), 2, "both sends went out: {log:?}");
    assert!(
        dispatched.iter().all(|&i| i > appended),
        "every send left after its durable append returned: {log:?}"
    );

    // restart on the same file: the entry whose sends were delivered is there
    let shard = TiramemsuShard::open(&path).unwrap();
    assert_eq!(shard.head(a.node()).unwrap(), Position::new(1));
    let node = node_with(Arc::new(shard), None);
    let journal = node.journal(&a).unwrap();
    assert_eq!(journal.len(), 1, "the entry survived the crash");
}
