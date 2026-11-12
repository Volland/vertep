//! Durable objects inside the actor runtime: [`ObjectStorage`] is a runtime `Storage` whose
//! journal routes durable objects to their custody hosts ([`CustodyStore`]) and every other
//! actor to the wrapped storage, whose graph stores serve everyone.
//!
//! The runtime's virtual-actor machinery is unchanged: a message to a durable object's DID
//! (bound to a chart type with `NodeHandle::bind_address`) activates it by reading its committed
//! journal from the custodians, each turn is one custody append, and the runtime performs a
//! turn's sends, timers and invokes only after the append returned, that is after a quorum of
//! custodians accepted it.

use std::sync::Arc;

use oxrdf::NamedNode;
use vertep_runtime::storage::Storage;
use vertep_store::{
    AppendError, AppendMeta, Entry, GraphStore, JournalCapabilities, JournalRecord, JournalStore,
    Position, StoreError,
};

use crate::custody::CustodyStore;

/// The journal of a wrapped storage, as a `JournalStore` the custody store can own.
struct InnerJournal<S: ?Sized>(Arc<S>);

impl<S: Storage + ?Sized> JournalStore for InnerJournal<S> {
    fn capabilities(&self) -> JournalCapabilities {
        self.0.journal().capabilities()
    }
    fn append(
        &self,
        actor: &NamedNode,
        expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError> {
        self.0.journal().append(actor, expected, entries, meta)
    }
    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        self.0.journal().read(actor, from)
    }
    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        self.0.journal().head(actor)
    }
    fn actors(&self) -> Result<Vec<NamedNode>, StoreError> {
        self.0.journal().actors()
    }
}

/// A node's storage with durable objects: journals of durable objects are custody journals,
/// everything else is `inner`.
pub struct ObjectStorage<S: ?Sized> {
    inner: Arc<S>,
    custody: Arc<CustodyStore>,
}

impl<S: ?Sized> std::fmt::Debug for ObjectStorage<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectStorage").finish_non_exhaustive()
    }
}

impl<S: Storage + ?Sized + 'static> ObjectStorage<S> {
    /// Durable objects on top of `inner`.
    pub fn new(inner: Arc<S>) -> Self {
        let custody = Arc::new(CustodyStore::new(Arc::new(InnerJournal(inner.clone()))));
        ObjectStorage { inner, custody }
    }

    /// The custody store: add the custody hosts of the durable objects served here.
    pub fn custody(&self) -> &Arc<CustodyStore> {
        &self.custody
    }
}

// @tg: implements:: [[openspec:durable-objects#Effects After Durable Commit]] {note: "the runtime performs a turn's effects only after the custody append returns"}
impl<S: Storage + ?Sized + 'static> Storage for ObjectStorage<S> {
    fn journal(&self) -> &dyn JournalStore {
        self.custody.as_ref()
    }
    fn graph(&self, actor: &NamedNode) -> &dyn GraphStore {
        self.inner.graph(actor)
    }
    fn graphs(&self) -> Vec<&dyn GraphStore> {
        self.inner.graphs()
    }
}

/// Starts serving the durable object at `object` on `node`: activates it, which wins custody
/// (unless this host is a standby), replays its committed journal and re-arms its durable
/// timers. A timer that came due while no host served the object fires now; one already fired
/// and journaled elsewhere is not armed again. A host that takes an object over by handoff or
/// failover calls this, so the object's alarms move with it.
///
/// # Errors
///
/// The runtime's activation error, including the custody store's when no quorum is reachable.
// @tg: implements:: [[openspec:durable-objects#Durable Alarms]]
pub fn serve_object(
    node: &mut vertep_runtime::Node,
    object: &vertep_runtime::Address,
) -> Result<(), vertep_runtime::ActivationError> {
    node.activate_actor(object)
}
