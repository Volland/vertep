//! One shard: one tiramemsu file holding the journals and current-state graphs of its actors.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use oxrdf::vocab::rdf;
use oxrdf::{GraphName, NamedNode, NamedOrBlankNode, Quad, Term, Triple};
use tiramemsu::{
    AssertOpts, Db, Eid, Error, ObjectId, OpenOptions, SparqlOptions, SparqlResult, TimeRef, Tx,
    TxOptions, Valid, Value, View,
};
use vertep_store::sparql::{clear_graph, delete_data, insert_data};
use vertep_store::{
    AppendError, AppendMeta, Entry, GraphCapabilities, GraphStore, JournalCapabilities,
    JournalRecord, JournalStore, Position, QueryResults, RdfCapability, Solutions, StoreError,
    prepare_append, vocab,
};

use crate::convert::{BlankMap, iri, literal, rdf_triple, store_error, term_obj, value_term};

const SYS_CARDINALITY: &str = "urn:tiramemsu:sys:cardinality";
const SYS_ONE: &str = "urn:tiramemsu:sys:one";
const SYS_IN_GRAPH: &str = "urn:tiramemsu:sys:inGraph";
const SYS_DERIVED_FROM: &str = "urn:tiramemsu:sys:derivedFrom";

/// Files opened by this process: tiramemsu has a single writer per file, so a file is opened
/// by at most one [`TiramemsuShard`] at a time.
fn open_files() -> &'static Mutex<HashSet<PathBuf>> {
    static OPEN: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    OPEN.get_or_init(|| Mutex::new(HashSet::new()))
}

/// The claim on a file, released when the shard is dropped.
#[derive(Debug)]
struct FileClaim(PathBuf);

impl FileClaim {
    fn take(path: &Path) -> Result<FileClaim, StoreError> {
        let dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let dir = dir
            .canonicalize()
            .map_err(|e| StoreError::Invalid(format!("shard directory {}: {e}", dir.display())))?;
        let name = path
            .file_name()
            .ok_or_else(|| StoreError::invalid("a shard path names a file"))?;
        let key = dir.join(name);
        let mut open = open_files()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !open.insert(key.clone()) {
            return Err(StoreError::Unavailable(format!(
                "{} is already open in this process (tiramemsu has one writer per file)",
                key.display()
            )));
        }
        Ok(FileClaim(key))
    }
}

impl Drop for FileClaim {
    fn drop(&mut self) {
        open_files()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.0);
    }
}

/// One tiramemsu file: the journals, current-state graphs and any other quads of the actors of
/// one shard, behind the file's single writer.
///
/// It implements both [`JournalStore`] and [`GraphStore`] on the same `Db`, so the journal and
/// the current-state graph are one consistent store. An append is one `Db::transact`: it checks
/// the fencing epoch and the actor's `vt:head`, writes each entry's statements as members of the
/// graph named by the entry IRI, retracts the statements of exited states, derives the new state
/// statements with the entries as support (`Tx::derive`), and advances the head. Any failure
/// rolls the whole transaction back.
///
/// Layout in the file:
///
/// | Statement | Graph | Notes |
/// |---|---|---|
/// | entry content and stamps | entry IRI | written with `create`, never retracted |
/// | `<actor> vt:head n` | none | cardinality one: a new head retracts the old |
/// | `<scope> vt:fencingEpoch e` | none | cardinality one, written only when the epoch rises |
/// | current state | actor IRI | derived, support = the entries' `rdf:type vt:JournalEntry` statements |
///
/// Tiramemsu never deletes, so [`TiramemsuShard::view_as_of`] shows the current-state graph as
/// of any earlier transaction without replay.
// @tg: implements:: [[openspec:storage#Tiramemsu Backend]]
// @tg: implements:: [[openspec:storage#Input And State In One Transaction]]
// @tg: implements:: [[openspec:storage#State Facts Carry Their Cause]]
pub struct TiramemsuShard {
    db: Db,
    path: PathBuf,
    _claim: FileClaim,
}

impl std::fmt::Debug for TiramemsuShard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TiramemsuShard")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// The journal store capabilities of a tiramemsu shard: fencing and state in the append.
pub const CAPABILITIES: JournalCapabilities = JournalCapabilities {
    fencing: true,
    state_in_append: true,
};

/// The graph store capabilities of tiramemsu: RDF 1.2 (annotations resolve to statement ids;
/// a triple term in inserted data is asserted), a union default graph, and its `sys:` and `tm:`
/// bookkeeping namespaces.
pub fn graph_capabilities() -> GraphCapabilities {
    GraphCapabilities {
        rdf: RdfCapability::Rdf12,
        union_default_graph: true,
        reserved_namespaces: vec![
            tiramemsu::vocab::SYS.to_owned(),
            tiramemsu::vocab::TM.to_owned(),
        ],
    }
}

/// Why an append transaction stopped, carried out of the transaction body.
struct Abort(AppendError);

impl From<Error> for Abort {
    fn from(e: Error) -> Self {
        Abort(AppendError::Storage(store_error(e)))
    }
}

impl From<StoreError> for Abort {
    fn from(e: StoreError) -> Self {
        Abort(AppendError::from(e))
    }
}

/// The integer object of the live `(s p ?)` statement, read inside a transaction.
fn tx_int(tx: &mut Tx<'_>, s: &Value, p: &Value) -> Result<Option<u64>, Error> {
    let (Some(s), Some(p)) = (tx.lookup(s)?, tx.lookup(p)?) else {
        return Ok(None);
    };
    for row in tx.live_rows(Some(s), Some(p), None)? {
        if let Value::Int(n) = tx.decode(ObjectId::from_raw(row.o))? {
            return Ok(u64::try_from(n).ok());
        }
    }
    Ok(None)
}

/// The integer object of the visible `(s p ?)` statement in a view.
fn view_int(view: &View<'_>, s: &Value, p: &Value) -> Result<Option<u64>, Error> {
    let (Some(s), Some(p)) = (view.encode(s)?, view.encode(p)?) else {
        return Ok(None);
    };
    for t in view.triples(Some(s), Some(p), None)? {
        if let Value::Int(n) = view.decode(t.o)? {
            return Ok(u64::try_from(n).ok());
        }
    }
    Ok(None)
}

/// The values of a state statement, which has no blank nodes.
fn state_values(t: &Triple) -> Result<(Value, Value, Value), StoreError> {
    let s = match &t.subject {
        NamedOrBlankNode::NamedNode(n) => iri(n.as_str()),
        NamedOrBlankNode::BlankNode(_) => {
            return Err(StoreError::invalid("state statements have no blank nodes"));
        }
    };
    let o = match &t.object {
        Term::NamedNode(n) => iri(n.as_str()),
        Term::Literal(l) => literal(l)?,
        _ => {
            return Err(StoreError::invalid(
                "state statement objects are IRIs or literals",
            ));
        }
    };
    Ok((s, iri(t.predicate.as_str()), o))
}

/// The live statements with content `(s p o)` that are members of `graph`.
fn tx_members(
    tx: &mut Tx<'_>,
    (s, p, o): &(Value, Value, Value),
    graph: &Value,
) -> Result<Vec<Eid>, Error> {
    let (Some(s), Some(p), Some(o), Some(g), Some(in_graph)) = (
        tx.lookup(s)?,
        tx.lookup(p)?,
        tx.lookup(o)?,
        tx.lookup(graph)?,
        tx.lookup(&iri(SYS_IN_GRAPH))?,
    ) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for eid in tx.live_eids(s, p, o)? {
        if !tx
            .live_rows(Some(eid.oid()), Some(in_graph), Some(g))?
            .is_empty()
        {
            out.push(eid);
        }
    }
    Ok(out)
}

impl TiramemsuShard {
    /// Opens (or creates) the shard file at `path` with tiramemsu's default options.
    ///
    /// # Errors
    ///
    /// [`StoreError::Unavailable`] when the file is already open in this process, and the
    /// errors of `Db::open`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_with(path, OpenOptions::default())
    }

    /// Opens (or creates) the shard file at `path` with `options`.
    ///
    /// # Errors
    ///
    /// As [`TiramemsuShard::open`].
    pub fn open_with(path: impl AsRef<Path>, options: OpenOptions) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        let claim = FileClaim::take(&path)?;
        let db = Db::open(&path, options).map_err(store_error)?;
        let shard = TiramemsuShard {
            db,
            path,
            _claim: claim,
        };
        shard.declare_schema()?;
        Ok(shard)
    }

    /// Declares `vt:head` and `vt:fencingEpoch` cardinality one, once per file.
    fn declare_schema(&self) -> Result<(), StoreError> {
        let card = iri(SYS_CARDINALITY);
        let one = iri(SYS_ONE);
        let view = self.db.now();
        let declared = |p: &str| -> Result<bool, Error> {
            let (Some(p), Some(c), Some(o)) = (
                view.encode(&iri(p))?,
                view.encode(&card)?,
                view.encode(&one)?,
            ) else {
                return Ok(false);
            };
            Ok(!view.triples(Some(p), Some(c), Some(o))?.is_empty())
        };
        let missing: Vec<&str> = [vocab::HEAD, vocab::FENCING_EPOCH]
            .into_iter()
            .filter(|p| !declared(p).unwrap_or(false))
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        self.db
            .transact(TxOptions::default(), |tx| {
                for p in &missing {
                    tx.assert(iri(p), card.clone(), one.clone(), Valid::ALWAYS)?;
                }
                Ok(())
            })
            .map(|_| ())
            .map_err(store_error)
    }

    /// The tiramemsu database, for its own views and queries.
    pub fn db(&self) -> &Db {
        &self.db
    }

    /// The shard file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The number of the last committed tiramemsu transaction.
    ///
    /// # Errors
    ///
    /// The store's read errors.
    pub fn last_tx(&self) -> Result<u64, StoreError> {
        self.db.last_t().map_err(store_error)
    }

    /// The graphs of this shard as of transaction `t`: what was believed right after it
    /// committed, read from the store's own history, not by replay.
    // @tg: implements:: [[openspec:storage#Time Travel Views]]
    pub fn view_as_of(&self, t: u64) -> TiramemsuView<'_> {
        TiramemsuView {
            db: &self.db,
            at: Some(TimeRef::Tx(t)),
        }
    }

    /// The graphs of this shard now, read-only.
    pub fn view_now(&self) -> TiramemsuView<'_> {
        TiramemsuView {
            db: &self.db,
            at: None,
        }
    }

    /// The journal entries whose appends caused the live statement `triple` in `actor`'s
    /// current-state graph, found by following the statement's support links.
    ///
    /// # Errors
    ///
    /// The store's read errors.
    pub fn state_causes(
        &self,
        actor: &NamedNode,
        triple: &Triple,
    ) -> Result<Vec<NamedNode>, StoreError> {
        let q = format!(
            "SELECT DISTINCT ?entry WHERE {{ GRAPH {actor} {{ {s} {p} {o} ~ ?state }} \
             ?state <{df}> ?support . ?entry {ty} <{je}> ~ ?support . }} ORDER BY ?entry",
            s = triple.subject,
            p = triple.predicate,
            o = triple.object,
            df = SYS_DERIVED_FROM,
            ty = rdf::TYPE,
            je = vocab::JOURNAL_ENTRY,
        );
        let solutions = run_query(&self.db.now(), &q)?.into_solutions()?;
        Ok(solutions
            .values("entry")
            .into_iter()
            .filter_map(|t| match t {
                Term::NamedNode(n) => Some(n),
                _ => None,
            })
            .collect())
    }

    /// The lifetimes of every statement `triple` ever had in `actor`'s current-state graph, as
    /// `(added at t, retracted at t)`, oldest first.
    ///
    /// # Errors
    ///
    /// The store's read errors.
    pub fn state_history(
        &self,
        actor: &NamedNode,
        triple: &Triple,
    ) -> Result<Vec<(u64, Option<u64>)>, StoreError> {
        let (s, p, o) = state_values(triple)?;
        let h = self.db.history();
        let ids = (
            h.encode(&s).map_err(store_error)?,
            h.encode(&p).map_err(store_error)?,
            h.encode(&o).map_err(store_error)?,
            h.encode(&iri(actor.as_str())).map_err(store_error)?,
            h.encode(&iri(SYS_IN_GRAPH)).map_err(store_error)?,
        );
        let (Some(s), Some(p), Some(o), Some(g), Some(in_graph)) = ids else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for t in h.triples(Some(s), Some(p), Some(o)).map_err(store_error)? {
            let member = !h
                .triples(Some(t.eid.oid()), Some(in_graph), Some(g))
                .map_err(store_error)?
                .is_empty();
            if member {
                out.push((t.t_add.0, t.t_ret.map(|r| r.0)));
            }
        }
        out.sort_unstable();
        Ok(out)
    }

    fn append_tx(
        tx: &mut Tx<'_>,
        actor: &NamedNode,
        expected: Position,
        stamped: &[Entry],
        meta: &AppendMeta,
    ) -> Result<(), Abort> {
        // 1. fencing: a stale epoch is rejected before the position is even looked at
        let mut raise_epoch = None;
        if let Some(token) = &meta.fencing {
            let scope = iri(token.scope.as_str());
            let highest = tx_int(tx, &scope, &iri(vocab::FENCING_EPOCH))?;
            if token.epoch < highest.unwrap_or(0) {
                return Err(Abort(AppendError::Fenced {
                    scope: token.scope.clone(),
                    epoch: token.epoch,
                    highest: highest.unwrap_or(0),
                }));
            }
            if highest != Some(token.epoch) {
                raise_epoch = Some((scope, token.epoch));
            }
        }
        // 2. compare-and-set on the head
        let actor_v = iri(actor.as_str());
        let head = iri(vocab::HEAD);
        let actual = Position::new(tx_int(tx, &actor_v, &head)?.unwrap_or(0));
        if actual != expected {
            return Err(Abort(AppendError::Conflict { expected, actual }));
        }
        // 3. the entries, each in its own graph
        let position_p = iri(vocab::POSITION);
        let anchor = (iri(vocab::RDF_TYPE), iri(vocab::JOURNAL_ENTRY));
        let mut support = Vec::with_capacity(stamped.len());
        for entry in stamped {
            let name = iri(entry.name().as_str());
            if tx_int(tx, &name, &position_p)?.is_some() {
                return Err(Abort(AppendError::Invalid(format!(
                    "entry {} is already in the store",
                    entry.name()
                ))));
            }
            let mut blanks = BlankMap::new();
            for t in entry.triples() {
                let s = term_obj(tx, t.subject.into(), &mut blanks)?;
                let p = iri(t.predicate.as_str());
                let is_anchor = t.subject == entry.name().as_ref().into()
                    && p == anchor.0
                    && t.object == oxrdf::TermRef::from(vocab::node(vocab::JOURNAL_ENTRY).as_ref());
                let o = term_obj(tx, t.object, &mut blanks)?;
                let eid = tx.create(s, p, o, Valid::ALWAYS)?;
                tx.add_to_graph(eid, name.clone(), AssertOpts::default())?;
                if is_anchor {
                    support.push(eid);
                }
            }
        }
        // 4. the current-state graph: retract exited states, derive the new ones from the entries
        if let Some(delta) = &meta.state {
            for t in &delta.retract {
                let content = state_values(t)?;
                for eid in tx_members(tx, &content, &actor_v)? {
                    tx.remove_from_graph(eid, actor_v.clone())?;
                    let in_graph = tx.lookup(&iri(SYS_IN_GRAPH))?;
                    if tx.live_rows(Some(eid.oid()), in_graph, None)?.is_empty() {
                        tx.retract(eid)?;
                    }
                }
            }
            for t in &delta.assert {
                let content = state_values(t)?;
                if !tx_members(tx, &content, &actor_v)?.is_empty() {
                    continue; // already active: its cause stays the entry that made it so
                }
                let (s, p, o) = content;
                let eid = tx.derive(s, p, o, Valid::ALWAYS, &support)?.eid();
                tx.add_to_graph(eid, actor_v.clone(), AssertOpts::default())?;
            }
        }
        // 5. the fencing epoch and the head, both cardinality one
        if let Some((scope, epoch)) = raise_epoch {
            tx.assert(
                scope,
                iri(vocab::FENCING_EPOCH),
                Value::Int(epoch as i64),
                Valid::ALWAYS,
            )?;
        }
        let new_head = expected.advance(stamped.len() as u64);
        tx.assert(
            actor_v,
            head,
            Value::Int(new_head.get() as i64),
            Valid::ALWAYS,
        )?;
        Ok(())
    }
}

impl JournalStore for TiramemsuShard {
    fn capabilities(&self) -> JournalCapabilities {
        CAPABILITIES
    }

    fn append(
        &self,
        actor: &NamedNode,
        expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError> {
        let stamped = prepare_append(actor, expected, entries, meta, CAPABILITIES)?;
        if let Some(fencing) = &meta.fencing
            && fencing.epoch > i64::MAX as u64
        {
            return Err(AppendError::Invalid("fencing epochs fit in 63 bits".into()));
        }
        let mut abort: Option<AppendError> = None;
        let result = self.db.transact(TxOptions::default(), |tx| {
            Self::append_tx(tx, actor, expected, &stamped, meta).map_err(|Abort(e)| {
                abort = Some(e);
                Error::custom("vertep append aborted")
            })
        });
        match result {
            Ok(_) => Ok(expected.advance(stamped.len() as u64)),
            Err(e) => Err(abort.unwrap_or_else(|| AppendError::Storage(store_error(e)))),
        }
    }

    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        let q = format!(
            "SELECT ?e ?n ?s ?p ?o WHERE {{ ?e <{a}> {actor} ; <{pos}> ?n . FILTER(?n >= {from}) \
             GRAPH ?e {{ ?s ?p ?o }} }}",
            a = vocab::ACTOR,
            pos = vocab::POSITION,
            from = from.get(),
        );
        let s = run_query(&self.db.now(), &q)?.into_solutions()?;
        let mut entries: BTreeMap<u64, (NamedNode, Vec<Triple>)> = BTreeMap::new();
        for i in 0..s.len() {
            let (
                Some(Term::NamedNode(e)),
                Some(Term::Literal(n)),
                Some(subj),
                Some(Term::NamedNode(p)),
                Some(o),
            ) = (
                s.get(i, "e"),
                s.get(i, "n"),
                s.get(i, "s"),
                s.get(i, "p"),
                s.get(i, "o"),
            )
            else {
                return Err(StoreError::Backend(format!("malformed journal row {i}")));
            };
            let n: u64 = n
                .value()
                .parse()
                .map_err(|_| StoreError::Backend(format!("position {n}")))?;
            let subj = match subj {
                Term::NamedNode(x) => NamedOrBlankNode::from(x.clone()),
                Term::BlankNode(x) => NamedOrBlankNode::from(x.clone()),
                other => return Err(StoreError::Backend(format!("subject {other}"))),
            };
            entries
                .entry(n)
                .or_insert_with(|| (e.clone(), Vec::new()))
                .1
                .push(Triple::new(subj, p.clone(), o.clone()));
        }
        Ok(entries
            .into_iter()
            .map(|(n, (name, triples))| JournalRecord {
                position: Position::new(n),
                entry: Entry::new(name, triples),
            })
            .collect())
    }

    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        view_int(&self.db.now(), &iri(actor.as_str()), &iri(vocab::HEAD))
            .map(|h| Position::new(h.unwrap_or(0)))
            .map_err(store_error)
    }

    fn actors(&self) -> Result<Vec<NamedNode>, StoreError> {
        let view = self.db.now();
        let Some(head) = view.encode(&iri(vocab::HEAD)).map_err(store_error)? else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for t in view.triples(None, Some(head), None).map_err(store_error)? {
            if let Value::Iri(s) = view.decode(t.s).map_err(store_error)? {
                out.push(NamedNode::new(s).map_err(StoreError::backend)?);
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }
}

/// Runs SPARQL query text (never an update) on a view.
fn run_query(view: &View<'_>, sparql: &str) -> Result<QueryResults, StoreError> {
    let opts = SparqlOptions {
        query_only: true,
        ..Default::default()
    };
    match view.sparql_with(sparql, &opts).map_err(store_error)? {
        SparqlResult::Solutions(s) => {
            let mut rows = Vec::with_capacity(s.rows.len());
            for row in &s.rows {
                let mut out = Vec::with_capacity(row.len());
                for cell in row {
                    out.push(match cell {
                        Some(v) => Some(value_term(v)?),
                        None => None,
                    });
                }
                rows.push(out);
            }
            Ok(QueryResults::Solutions(Solutions {
                variables: s.vars.clone(),
                rows,
            }))
        }
        SparqlResult::Boolean(b) => Ok(QueryResults::Boolean(b)),
        SparqlResult::Graph(g) => Ok(QueryResults::Graph(
            g.iter().map(rdf_triple).collect::<Result<_, _>>()?,
        )),
        SparqlResult::Update(_) => Err(StoreError::invalid("an update in a query call")),
    }
}

fn run_update(db: &Db, sparql: &str) -> Result<(), StoreError> {
    db.now().sparql(sparql).map(|_| ()).map_err(store_error)
}

/// `INSERT DATA` text for tiramemsu. Tiramemsu resolves every reifier to the id of the
/// statement it reifies and refuses a reifier IRI, so a reifier's `rdf:reifies` quad and its
/// properties are written in annotation syntax (`s p o {| k v |}`) instead; everything else is
/// plain data.
fn insert_text(quads: &[Quad]) -> String {
    let reifies = vertep_store::reification::RDF_REIFIES;
    let mut reified: BTreeMap<(String, String), (GraphName, Triple)> = BTreeMap::new();
    for q in quads {
        if let (true, Term::Triple(t)) = (q.predicate.as_str() == reifies, &q.object) {
            reified.insert(
                (q.subject.to_string(), q.graph_name.to_string()),
                (q.graph_name.clone(), (**t).clone()),
            );
        }
    }
    if reified.is_empty() {
        return insert_data(quads);
    }
    let mut plain = Vec::new();
    let mut annotations: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for q in quads {
        let key = (q.subject.to_string(), q.graph_name.to_string());
        if !reified.contains_key(&key) {
            plain.push(q.clone());
        } else if q.predicate.as_str() != reifies {
            annotations
                .entry(key)
                .or_default()
                .push(format!("{} {}", q.predicate, q.object));
        }
    }
    let mut text = insert_data(&plain);
    text.pop(); // the closing brace
    for (key, (graph, t)) in &reified {
        let props = annotations
            .get(key)
            .map(|p| p.join(" ; "))
            .unwrap_or_default();
        let body = if props.is_empty() {
            format!("{} {} {} .", t.subject, t.predicate, t.object)
        } else {
            format!(
                "{} {} {} {{| {props} |}} .",
                t.subject, t.predicate, t.object
            )
        };
        match graph {
            GraphName::NamedNode(g) => text.push_str(&format!("  GRAPH {g} {{ {body} }}\n")),
            _ => text.push_str(&format!("  {body}\n")),
        }
    }
    text.push('}');
    text
}

fn check_insertable(quads: &[Quad]) -> Result<(), StoreError> {
    for q in quads {
        if let Term::Literal(l) = &q.object {
            literal(l)?;
        }
        if matches!(q.graph_name, GraphName::BlankNode(_)) {
            return Err(StoreError::invalid("graph names are IRIs"));
        }
    }
    Ok(())
}

impl GraphStore for TiramemsuShard {
    fn capabilities(&self) -> GraphCapabilities {
        graph_capabilities()
    }

    fn insert_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        if quads.is_empty() {
            return Ok(());
        }
        check_insertable(quads)?;
        run_update(&self.db, &insert_text(quads))
    }

    fn remove_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        if quads.is_empty() {
            return Ok(());
        }
        run_update(&self.db, &delete_data(quads)?)
    }

    fn clear_graph(&self, graph: &NamedNode) -> Result<(), StoreError> {
        run_update(&self.db, &clear_graph(graph))
    }

    fn query(&self, sparql: &str) -> Result<QueryResults, StoreError> {
        run_query(&self.db.now(), sparql)
    }
}

/// A read-only view of a shard's graphs, now or as of a past transaction.
#[derive(Clone, Copy)]
pub struct TiramemsuView<'a> {
    db: &'a Db,
    at: Option<TimeRef>,
}

impl std::fmt::Debug for TiramemsuView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TiramemsuView")
            .field("at", &self.at)
            .finish()
    }
}

impl TiramemsuView<'_> {
    fn view(&self) -> View<'_> {
        match self.at {
            Some(t) => self.db.as_of(t),
            None => self.db.now(),
        }
    }
}

impl GraphStore for TiramemsuView<'_> {
    fn capabilities(&self) -> GraphCapabilities {
        graph_capabilities()
    }

    fn insert_quads(&self, _quads: &[Quad]) -> Result<(), StoreError> {
        Err(StoreError::Unsupported(
            "a tiramemsu view is read-only".into(),
        ))
    }

    fn remove_quads(&self, _quads: &[Quad]) -> Result<(), StoreError> {
        Err(StoreError::Unsupported(
            "a tiramemsu view is read-only".into(),
        ))
    }

    fn clear_graph(&self, _graph: &NamedNode) -> Result<(), StoreError> {
        Err(StoreError::Unsupported(
            "a tiramemsu view is read-only".into(),
        ))
    }

    fn query(&self, sparql: &str) -> Result<QueryResults, StoreError> {
        run_query(&self.view(), sparql)
    }
}
