//! The journal store contract: per-actor, compare-and-set appends of RDF entries.

use std::collections::HashSet;

use oxrdf::dataset::CanonicalizationAlgorithm;
use oxrdf::vocab::xsd;
use oxrdf::{
    Dataset, Graph, GraphName, GraphNameRef, Literal, NamedNode, NamedOrBlankNode, Quad, Term,
    Triple,
};

use crate::error::{AppendError, StoreError};
use crate::vocab;

/// A journal position: the number of an entry in one actor's journal.
///
/// Entries are numbered from 1 without gaps; position 0 is the empty journal. An append
/// "at expected position n" succeeds only while the journal's last entry is n, and writes its
/// entries at n + 1, n + 2, ...
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position(u64);

impl Position {
    /// The empty journal.
    pub const ZERO: Position = Position(0);

    /// The position `n`.
    pub const fn new(n: u64) -> Self {
        Position(n)
    }

    /// The number.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The next position.
    pub const fn next(self) -> Self {
        Position(self.0 + 1)
    }

    /// This position advanced by `n` entries.
    pub const fn advance(self, n: u64) -> Self {
        Position(self.0 + n)
    }
}

impl From<u64> for Position {
    fn from(n: u64) -> Self {
        Position(n)
    }
}

/// One journal entry: an RDF dataset with exactly one named graph, named by the entry's IRI.
///
/// The entry IRI is chosen by the caller (the runtime derives it from the actor address and the
/// position, see `runtime-ontology#Stable Runtime IRIs`) and is unique in the store. On append a
/// store adds three stamps to the graph ([`Entry::stamp`]): `rdf:type vt:JournalEntry`,
/// `vt:actor` and `vt:position`, so every entry read back is self-describing in `vt:` terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    name: NamedNode,
    triples: Graph,
}

impl Entry {
    /// An entry named `name` holding `triples`.
    pub fn new(name: NamedNode, triples: impl IntoIterator<Item = Triple>) -> Self {
        Entry {
            name,
            triples: triples.into_iter().collect(),
        }
    }

    /// The entry encoded by a dataset, which must hold exactly one named graph, named by an IRI,
    /// and nothing in the default graph.
    ///
    /// # Errors
    ///
    /// [`StoreError::Invalid`] for any other dataset shape.
    pub fn from_dataset(dataset: &Dataset) -> Result<Self, StoreError> {
        let mut name: Option<NamedNode> = None;
        let mut triples = Graph::new();
        for q in dataset {
            let g = match q.graph_name {
                GraphNameRef::NamedNode(n) => n.into_owned(),
                GraphNameRef::BlankNode(_) => {
                    return Err(StoreError::invalid(
                        "an entry graph must be named by an IRI",
                    ));
                }
                GraphNameRef::DefaultGraph => {
                    return Err(StoreError::invalid(
                        "an entry has no statements in the default graph",
                    ));
                }
            };
            match &name {
                None => name = Some(g),
                Some(n) if *n != g => {
                    return Err(StoreError::invalid(
                        "an entry is one named graph, found several",
                    ));
                }
                Some(_) => {}
            }
            triples.insert(&Triple::new(
                q.subject.into_owned(),
                q.predicate.into_owned(),
                q.object.into_owned(),
            ));
        }
        let name = name.ok_or_else(|| StoreError::invalid("an entry has at least one quad"))?;
        Ok(Entry { name, triples })
    }

    /// The entry IRI, which is also the name of its graph.
    pub fn name(&self) -> &NamedNode {
        &self.name
    }

    /// The statements of the entry graph.
    pub fn triples(&self) -> &Graph {
        &self.triples
    }

    /// The quads of the entry: every statement in the graph named by the entry IRI.
    pub fn quads(&self) -> Vec<Quad> {
        let g = GraphName::NamedNode(self.name.clone());
        self.triples
            .iter()
            .map(|t| t.into_owned().in_graph(g.clone()))
            .collect()
    }

    /// The entry as an RDF dataset with one named graph.
    pub fn to_dataset(&self) -> Dataset {
        self.quads().into_iter().collect()
    }

    /// The position stamped on the entry (`<entry> vt:position n`), if any.
    pub fn position(&self) -> Option<Position> {
        let p = vocab::node(vocab::POSITION);
        self.triples
            .objects_for_subject_predicate(&self.name, &p)
            .find_map(|o| match o {
                oxrdf::TermRef::Literal(l) if l.datatype() == xsd::INTEGER => {
                    l.value().parse::<u64>().ok().map(Position)
                }
                _ => None,
            })
    }

    /// The entry with the store's stamps added: `rdf:type vt:JournalEntry`, `vt:actor <actor>`
    /// and `vt:position n`.
    ///
    /// # Errors
    ///
    /// [`StoreError::Invalid`] when the entry already states another actor or position.
    pub fn stamp(&self, actor: &NamedNode, position: Position) -> Result<Entry, StoreError> {
        let subject = NamedOrBlankNode::from(self.name.clone());
        let actor_p = vocab::node(vocab::ACTOR);
        let pos_p = vocab::node(vocab::POSITION);
        let pos = Literal::new_typed_literal(position.get().to_string(), xsd::INTEGER);
        for o in self
            .triples
            .objects_for_subject_predicate(&self.name, &actor_p)
        {
            if o != actor.as_ref().into() {
                return Err(StoreError::invalid(format!(
                    "entry {} states actor {o}, appended to {actor}",
                    self.name
                )));
            }
        }
        for o in self
            .triples
            .objects_for_subject_predicate(&self.name, &pos_p)
        {
            if o != pos.as_ref().into() {
                return Err(StoreError::invalid(format!(
                    "entry {} states position {o}, appended at {}",
                    self.name,
                    position.get()
                )));
            }
        }
        let mut triples = self.triples.clone();
        triples.insert(&Triple::new(
            subject.clone(),
            vocab::node(vocab::RDF_TYPE),
            vocab::node(vocab::JOURNAL_ENTRY),
        ));
        triples.insert(&Triple::new(subject.clone(), actor_p, actor.clone()));
        triples.insert(&Triple::new(subject, pos_p, pos));
        Ok(Entry {
            name: self.name.clone(),
            triples,
        })
    }

    /// True when both entries have the same IRI and isomorphic graphs (equal up to blank node
    /// labels, which a store may rename).
    pub fn is_isomorphic(&self, other: &Entry) -> bool {
        if self.name != other.name || self.triples.len() != other.triples.len() {
            return false;
        }
        let mut a = self.triples.clone();
        let mut b = other.triples.clone();
        a.canonicalize(CanonicalizationAlgorithm::default());
        b.canonicalize(CanonicalizationAlgorithm::default());
        a == b
    }

    /// True when the graph contains a blank node anywhere.
    pub fn has_blank_nodes(&self) -> bool {
        self.triples.iter().any(|t| {
            matches!(t.subject, oxrdf::NamedOrBlankNodeRef::BlankNode(_))
                || matches!(t.object, oxrdf::TermRef::BlankNode(_))
        })
    }
}

/// An entry read back from a journal, with its position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalRecord {
    /// The entry's position in the actor's journal.
    pub position: Position,
    /// The entry, with the store's stamps.
    pub entry: Entry,
}

/// A fencing token: a scope (for example a shard) and a monotonically increasing epoch.
///
/// A store that declares fencing rejects an append whose epoch is lower than the highest epoch
/// it has accepted for the scope.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FencingToken {
    /// The scope the epoch counts in, such as a shard IRI.
    pub scope: NamedNode,
    /// The epoch.
    pub epoch: u64,
}

impl FencingToken {
    /// A token for `scope` at `epoch`.
    pub fn new(scope: NamedNode, epoch: u64) -> Self {
        FencingToken { scope, epoch }
    }
}

/// The changes an append makes to the actor's current-state graph (the named graph named by
/// the actor address): statements of exited states to retract, and statements to assert.
///
/// Only stores that declare [`JournalCapabilities::state_in_append`] accept a delta; they
/// commit it in the same transaction as the entries. Retractions apply before assertions.
/// Statements may not contain blank nodes, so that a later delta can name them again.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StateDelta {
    /// Statements to retract from the current-state graph.
    pub retract: Vec<Triple>,
    /// Statements to assert in the current-state graph.
    pub assert: Vec<Triple>,
}

impl StateDelta {
    /// True when the delta changes nothing.
    pub fn is_empty(&self) -> bool {
        self.retract.is_empty() && self.assert.is_empty()
    }
}

/// Optional metadata of an append.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppendMeta {
    /// The fencing token; `None` is an unfenced append, checked only by compare-and-set.
    pub fencing: Option<FencingToken>,
    /// The state changes to commit with the entries.
    pub state: Option<StateDelta>,
}

impl AppendMeta {
    /// No metadata: an unfenced append without state changes.
    pub fn none() -> Self {
        AppendMeta::default()
    }

    /// This metadata with a fencing token.
    pub fn with_fencing(mut self, token: FencingToken) -> Self {
        self.fencing = Some(token);
        self
    }

    /// This metadata with a state delta.
    pub fn with_state(mut self, delta: StateDelta) -> Self {
        self.state = Some(delta);
        self
    }
}

/// What a journal store supports beyond compare-and-set, read before relying on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct JournalCapabilities {
    /// The store enforces fencing tokens. A store without fencing rejects an append that
    /// carries a token with [`AppendError::Unsupported`] instead of ignoring it.
    pub fencing: bool,
    /// The store commits a [`StateDelta`] in the same transaction as the entries.
    pub state_in_append: bool,
}

/// A journal store: per actor address, a linearizable, all-or-nothing append at an expected
/// position, and reads from a position.
// @tg: implements:: [[openspec:storage#Journal Store Contract]]
// @tg: implements:: [[openspec:storage#Append Fencing Token]]
pub trait JournalStore: Send + Sync {
    /// What the store supports beyond compare-and-set.
    fn capabilities(&self) -> JournalCapabilities;

    /// True when the store enforces fencing tokens.
    fn supports_fencing(&self) -> bool {
        self.capabilities().fencing
    }

    /// Appends `entries` to `actor`'s journal if, and only if, its last entry is at `expected`.
    ///
    /// On success the entries are at `expected + 1 ..= expected + entries.len()` and the new
    /// head is returned. On any error nothing is written: no entry, no state change and no
    /// fencing epoch. Appends for one actor are linearizable.
    ///
    /// # Errors
    ///
    /// [`AppendError::Conflict`] when the journal is elsewhere, [`AppendError::Fenced`] for a
    /// stale fencing epoch, [`AppendError::Unsupported`] for metadata the store does not
    /// support, [`AppendError::Invalid`] for malformed entries, [`AppendError::Storage`] when
    /// the store fails.
    fn append(
        &self,
        actor: &NamedNode,
        expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError>;

    /// The entries of `actor`'s journal from position `from` (inclusive; 0 reads from the
    /// start) to the head, in order.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the store fails.
    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError>;

    /// The position of `actor`'s last entry, 0 for an empty journal.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the store fails.
    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError>;

    /// Every actor with at least one entry.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the store fails.
    fn actors(&self) -> Result<Vec<NamedNode>, StoreError>;
}

impl<J: JournalStore + ?Sized> JournalStore for std::sync::Arc<J> {
    fn capabilities(&self) -> JournalCapabilities {
        (**self).capabilities()
    }
    fn append(
        &self,
        actor: &NamedNode,
        expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError> {
        (**self).append(actor, expected, entries, meta)
    }
    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        (**self).read(actor, from)
    }
    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        (**self).head(actor)
    }
    fn actors(&self) -> Result<Vec<NamedNode>, StoreError> {
        (**self).actors()
    }
}

/// The checks every backend runs before an append touches storage, and the stamped entries to
/// write at `expected + 1 ..`.
///
/// It rejects an empty append, two entries with one IRI, an entry named like the actor,
/// metadata the store does not support and state deltas with blank nodes.
///
/// # Errors
///
/// [`AppendError::Invalid`] or [`AppendError::Unsupported`].
pub fn prepare_append(
    actor: &NamedNode,
    expected: Position,
    entries: &[Entry],
    meta: &AppendMeta,
    caps: JournalCapabilities,
) -> Result<Vec<Entry>, AppendError> {
    if entries.is_empty() {
        return Err(AppendError::Invalid(
            "an append has at least one entry".into(),
        ));
    }
    if meta.fencing.is_some() && !caps.fencing {
        return Err(AppendError::Unsupported(
            "this journal store does not enforce fencing tokens".into(),
        ));
    }
    if let Some(delta) = &meta.state {
        if !caps.state_in_append {
            return Err(AppendError::Unsupported(
                "this journal store does not commit state changes with entries".into(),
            ));
        }
        let blank = delta.retract.iter().chain(&delta.assert).any(|t| {
            matches!(t.subject, NamedOrBlankNode::BlankNode(_))
                || matches!(t.object, Term::BlankNode(_))
        });
        if blank {
            return Err(AppendError::Invalid(
                "current-state statements cannot contain blank nodes".into(),
            ));
        }
    }
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(entries.len());
    for (i, e) in entries.iter().enumerate() {
        if e.name() == actor {
            return Err(AppendError::Invalid(format!(
                "entry {} is named like its actor",
                e.name()
            )));
        }
        if !seen.insert(e.name().clone()) {
            return Err(AppendError::Invalid(format!(
                "entry {} appears twice in one append",
                e.name()
            )));
        }
        out.push(e.stamp(actor, expected.advance(i as u64 + 1))?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> NamedNode {
        NamedNode::new(format!("urn:test:{s}")).unwrap()
    }

    #[test]
    fn dataset_round_trip_and_shape_checks() {
        let e = Entry::new(iri("e1"), [Triple::new(iri("a"), iri("p"), iri("b"))]);
        let ds = e.to_dataset();
        assert_eq!(Entry::from_dataset(&ds).unwrap(), e);

        let mut two = ds.clone();
        two.insert(&Quad::new(iri("a"), iri("p"), iri("c"), iri("e2")));
        assert!(Entry::from_dataset(&two).is_err());
        let mut default = ds;
        default.insert(&Quad::new(
            iri("a"),
            iri("p"),
            iri("c"),
            GraphName::DefaultGraph,
        ));
        assert!(Entry::from_dataset(&default).is_err());
        assert!(Entry::from_dataset(&Dataset::new()).is_err());
    }

    #[test]
    fn stamp_adds_type_actor_and_position_and_rejects_contradictions() {
        let e = Entry::new(iri("e1"), [Triple::new(iri("a"), iri("p"), iri("b"))]);
        let s = e.stamp(&iri("actor"), Position::new(7)).unwrap();
        assert_eq!(s.triples().len(), 4);
        assert_eq!(s.position(), Some(Position::new(7)));
        // stamping twice is idempotent
        assert_eq!(s.stamp(&iri("actor"), Position::new(7)).unwrap(), s);
        assert!(s.stamp(&iri("actor"), Position::new(8)).is_err());
        assert!(s.stamp(&iri("other"), Position::new(7)).is_err());
    }

    #[test]
    fn prepare_append_rejects_bad_requests() {
        let caps = JournalCapabilities::default();
        let a = iri("actor");
        let e = Entry::new(iri("e1"), [Triple::new(iri("a"), iri("p"), iri("b"))]);
        assert!(matches!(
            prepare_append(&a, Position::ZERO, &[], &AppendMeta::none(), caps),
            Err(AppendError::Invalid(_))
        ));
        assert!(matches!(
            prepare_append(
                &a,
                Position::ZERO,
                &[e.clone(), e.clone()],
                &AppendMeta::none(),
                caps
            ),
            Err(AppendError::Invalid(_))
        ));
        let fenced = AppendMeta::none().with_fencing(FencingToken::new(iri("shard"), 1));
        assert!(matches!(
            prepare_append(&a, Position::ZERO, std::slice::from_ref(&e), &fenced, caps),
            Err(AppendError::Unsupported(_))
        ));
        let stamped = prepare_append(&a, Position::new(3), &[e], &AppendMeta::none(), caps)
            .unwrap()
            .remove(0);
        assert_eq!(stamped.position(), Some(Position::new(4)));
    }
}
