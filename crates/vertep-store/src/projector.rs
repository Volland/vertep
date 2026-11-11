//! Eventual projection of journals into an external graph store.

use std::collections::HashSet;

use oxrdf::vocab::xsd;
use oxrdf::{GraphName, Literal, NamedNode, Quad, Term, Triple};

use crate::error::StoreError;
use crate::graph::GraphStore;
use crate::journal::{JournalStore, Position};
use crate::vocab;

/// What one [`Projector::project_actor`] run did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionReport {
    /// The actor.
    pub actor: NamedNode,
    /// The projected position before the run.
    pub from: Position,
    /// The projected position after the run.
    pub to: Position,
    /// The number of entries written.
    pub entries: usize,
}

/// Copies journal entries into a separate graph store in journal order, idempotently, and
/// records per actor the last projected position in the target store.
///
/// Entry graphs are written with one insert per entry, in position order. When a state source
/// is configured (the graph side of a journal store that keeps current state, such as a
/// tiramemsu shard), the actor's current-state graph is then brought to the source's content by
/// a difference (removals and insertions, never a clear), and finally the position is recorded
/// as `<actor> vt:projectedPosition n` in the graph [`vocab::PROJECTION_GRAPH`].
///
/// Every write is idempotent for entries without blank nodes, so a run interrupted at any point
/// (the target unreachable, the process stopped) is completed by running again: entries after
/// the recorded position are written again and change nothing they had already written.
// @tg: implements:: [[openspec:storage#Eventual Projection To External Graph Stores]]
pub struct Projector<'a> {
    journal: &'a dyn JournalStore,
    target: &'a dyn GraphStore,
    state_source: Option<&'a dyn GraphStore>,
}

impl std::fmt::Debug for Projector<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Projector")
            .field("state_source", &self.state_source.is_some())
            .finish_non_exhaustive()
    }
}

impl<'a> Projector<'a> {
    /// A projector from `journal` into `target`.
    pub fn new(journal: &'a dyn JournalStore, target: &'a dyn GraphStore) -> Self {
        Projector {
            journal,
            target,
            state_source: None,
        }
    }

    /// This projector also copying each actor's current-state graph from `source`.
    pub fn with_state_source(mut self, source: &'a dyn GraphStore) -> Self {
        self.state_source = Some(source);
        self
    }

    /// The last position of `actor` recorded in the target, 0 when none.
    ///
    /// # Errors
    ///
    /// The errors of [`GraphStore::query`].
    pub fn projected_position(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        let q = format!(
            "SELECT ?n WHERE {{ GRAPH <{g}> {{ {actor} <{p}> ?n }} }}",
            g = vocab::PROJECTION_GRAPH,
            p = vocab::PROJECTED_POSITION,
        );
        let solutions = self.target.query(&q)?.into_solutions()?;
        Ok(Position::new(
            solutions
                .values("n")
                .iter()
                .filter_map(|t| match t {
                    Term::Literal(l) => l.value().parse::<u64>().ok(),
                    _ => None,
                })
                .max()
                .unwrap_or(0),
        ))
    }

    /// Projects the entries of `actor` after its recorded position, then its state, then
    /// records the new position.
    ///
    /// # Errors
    ///
    /// The errors of the journal read and of the graph store writes. Nothing already written
    /// needs undoing: running again completes the projection.
    pub fn project_actor(&self, actor: &NamedNode) -> Result<ProjectionReport, StoreError> {
        let from = self.projected_position(actor)?;
        let records = self.journal.read(actor, from.next())?;
        let mut to = from;
        for record in &records {
            debug_assert_eq!(record.position, to.next(), "journal reads are gap-free");
            self.target.insert_quads(&record.entry.quads())?;
            to = record.position;
        }
        if records.is_empty() {
            return Ok(ProjectionReport {
                actor: actor.clone(),
                from,
                to,
                entries: 0,
            });
        }
        if let Some(source) = self.state_source {
            self.copy_state(source, actor)?;
        }
        self.record_position(actor, from, to)?;
        Ok(ProjectionReport {
            actor: actor.clone(),
            from,
            to,
            entries: records.len(),
        })
    }

    /// Projects every actor of the journal store.
    ///
    /// # Errors
    ///
    /// The first error of [`Projector::project_actor`]; actors before it stay projected.
    pub fn project_all(&self) -> Result<Vec<ProjectionReport>, StoreError> {
        self.journal
            .actors()?
            .iter()
            .map(|a| self.project_actor(a))
            .collect()
    }

    fn graph_triples(store: &dyn GraphStore, graph: &NamedNode) -> Result<Vec<Triple>, StoreError> {
        store
            .query(&format!(
                "CONSTRUCT {{ ?s ?p ?o }} WHERE {{ GRAPH {graph} {{ ?s ?p ?o }} }}"
            ))?
            .into_graph()
    }

    fn copy_state(&self, source: &dyn GraphStore, actor: &NamedNode) -> Result<(), StoreError> {
        let want: HashSet<Triple> = Self::graph_triples(source, actor)?.into_iter().collect();
        let have: HashSet<Triple> = Self::graph_triples(self.target, actor)?
            .into_iter()
            .collect();
        let g = GraphName::NamedNode(actor.clone());
        let add: Vec<Quad> = want
            .difference(&have)
            .map(|t| t.clone().in_graph(g.clone()))
            .collect();
        let remove: Vec<Quad> = have
            .difference(&want)
            .map(|t| t.clone().in_graph(g.clone()))
            .collect();
        if !add.is_empty() {
            self.target.insert_quads(&add)?;
        }
        if !remove.is_empty() {
            self.target.remove_quads(&remove)?;
        }
        Ok(())
    }

    fn record_position(
        &self,
        actor: &NamedNode,
        old: Position,
        new: Position,
    ) -> Result<(), StoreError> {
        let quad = |p: Position| {
            Quad::new(
                actor.clone(),
                vocab::node(vocab::PROJECTED_POSITION),
                Literal::new_typed_literal(p.get().to_string(), xsd::INTEGER),
                vocab::node(vocab::PROJECTION_GRAPH),
            )
        };
        // insert the new position before removing the old one: a crash in between leaves two
        // values, and readers take the larger
        self.target.insert_quads(&[quad(new)])?;
        if old != Position::ZERO && old != new {
            self.target.remove_quads(&[quad(old)])?;
        }
        Ok(())
    }
}
