//! Guards and actions as Rust functions, registered by IRI.
//!
//! A chart on the `rust` datamodel names each guard and action by an IRI: a guard is a
//! transition `cond` such as `vt:guard/order/positive`, an action is the `array` of a lowered
//! `<foreach>` such as `vt:action/order/record_total` (see [`crate::lowering`]). A [`Registry`]
//! resolves these IRIs to compiled functions, so a chart compiled by `statechart!`, read from a
//! file or loaded from RDF at runtime binds to the same code.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use vertep_core::datamodel::{EvalError, Event};
use vertep_core::ir::{Block, Chart, ExecKind, State, VT_NS};

use crate::emit::{ChartEvent, Emit, Emitted, Sys};
use crate::lowering;
use crate::value::FromEvent;

/// A guard as stored in the registry: the typed function behind a conversion of the event.
pub type GuardImpl<C> = Arc<dyn Fn(&C, &Event) -> Result<bool, EvalError> + Send + Sync>;

/// An action as stored in the registry.
pub type ActionImpl<C> =
    Arc<dyn Fn(&mut C, &Event, &Sys) -> Result<Vec<Emitted>, EvalError> + Send + Sync>;

/// A guard: a function of a shared context and the typed event argument, returning `bool`.
///
/// Guards are pure by construction for context and effects: they receive `&C`, never `&mut C`,
/// and no [`Emit`]. Not performing I/O and returning the same result for the same context and
/// event is their documented contract.
// @tg: implements:: [[openspec:rust-dsl#Pure Guards]]
// @tg: implements:: [[openspec:rust-dsl#Guard and Action Signature Checking]]
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a guard for events with payload `{P}`",
    label = "expected `fn(&{C}, &{P}) -> bool`",
    note = "a guard reads a shared context and the typed event and returns bool; it cannot change the context or emit events"
)]
pub trait GuardFn<C, P>: Send + Sync + 'static {
    /// Evaluate the guard.
    fn eval(&self, ctx: &C, event: &P) -> bool;
}

impl<C, P, F> GuardFn<C, P> for F
where
    F: Fn(&C, &P) -> bool + Send + Sync + 'static,
{
    fn eval(&self, ctx: &C, event: &P) -> bool {
        self(ctx, event)
    }
}

/// Marker: an action `fn(&mut C, &P, &mut Emit<E>)`.
pub enum WithEmit {}
/// Marker: an action `fn(&mut C, &P)`.
pub enum Plain {}
/// Marker: an action `fn(&mut C, &P) -> Vec<E>` whose events are raised.
pub enum Returning {}

/// An action: a function of the mutable context and the typed event argument that may emit
/// events, in one of three shapes: `fn(&mut C, &P, &mut Emit<E>)`, `fn(&mut C, &P)` or
/// `fn(&mut C, &P) -> Vec<E>` (the returned events are raised).
// @tg: implements:: [[openspec:rust-dsl#Actions Emit Effects as Events]]
// @tg: implements:: [[openspec:rust-dsl#Guard and Action Signature Checking]]
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not an action for events with payload `{P}`",
    label = "expected `fn(&mut {C}, &{P}, &mut Emit)`, `fn(&mut {C}, &{P})` or `fn(&mut {C}, &{P}) -> Vec<{E}>`",
    note = "an action changes the context and has effects only by emitting events"
)]
pub trait IntoAction<C, P, E, M>: Send + Sync + 'static {
    /// Run the action, collecting its emitted events.
    fn run(&self, ctx: &mut C, event: &P, emit: &mut Emit<E>);
}

impl<C, P, E, F> IntoAction<C, P, E, WithEmit> for F
where
    F: Fn(&mut C, &P, &mut Emit<E>) + Send + Sync + 'static,
{
    fn run(&self, ctx: &mut C, event: &P, emit: &mut Emit<E>) {
        self(ctx, event, emit)
    }
}

impl<C, P, E, F> IntoAction<C, P, E, Plain> for F
where
    F: Fn(&mut C, &P) + Send + Sync + 'static,
{
    fn run(&self, ctx: &mut C, event: &P, _emit: &mut Emit<E>) {
        self(ctx, event)
    }
}

impl<C, P, E: ChartEvent, F> IntoAction<C, P, E, Returning> for F
where
    F: Fn(&mut C, &P) -> Vec<E> + Send + Sync + 'static,
{
    fn run(&self, ctx: &mut C, event: &P, emit: &mut Emit<E>) {
        for e in self(ctx, event) {
            emit.raise(e);
        }
    }
}

/// Why a function could not be registered or resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryError {
    /// Two functions were registered under one IRI.
    Duplicate {
        /// The IRI.
        iri: String,
    },
    /// The chart names a guard or action IRI no function is registered under.
    Unregistered {
        /// The IRI as the chart writes it.
        iri: String,
        /// `guard` or `action`.
        role: &'static str,
    },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::Duplicate { iri } => {
                write!(f, "a function is already registered under `{iri}`")
            }
            RegistryError::Unregistered { iri, role } => {
                write!(f, "no {role} function is registered under `{iri}`")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

/// The IRI in its expanded form: a leading `vt:` becomes the vt namespace.
pub fn expand_iri(iri: &str) -> String {
    match iri.trim().strip_prefix("vt:") {
        Some(rest) => format!("{VT_NS}{rest}"),
        None => iri.trim().to_string(),
    }
}

/// Resolves guard and action IRIs to Rust functions over the context `C`
/// ([[openspec:rust-dsl#Functions Registered by IRI]]).
// @tg: implements:: [[openspec:rust-dsl#Functions Registered by IRI]]
pub struct Registry<C> {
    guards: BTreeMap<String, GuardImpl<C>>,
    actions: BTreeMap<String, ActionImpl<C>>,
}

impl<C> Default for Registry<C> {
    fn default() -> Self {
        Registry {
            guards: BTreeMap::new(),
            actions: BTreeMap::new(),
        }
    }
}

impl<C> fmt::Debug for Registry<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry")
            .field("guards", &self.guards.keys().collect::<Vec<_>>())
            .field("actions", &self.actions.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl<C> Clone for Registry<C> {
    fn clone(&self) -> Self {
        Registry {
            guards: self.guards.clone(),
            actions: self.actions.clone(),
        }
    }
}

impl<C: 'static> Registry<C> {
    /// An empty registry.
    pub fn new() -> Self {
        Registry::default()
    }

    fn taken(&self, key: &str) -> bool {
        self.guards.contains_key(key) || self.actions.contains_key(key)
    }

    /// Register a guard under `iri`. The event argument `P` is read from the current event
    /// with [`FromEvent`]; a payload that does not fit makes the guard fail, which the
    /// interpreter treats as `false` with `error.execution`.
    pub fn guard<P, F>(&mut self, iri: &str, f: F) -> Result<&mut Self, RegistryError>
    where
        P: FromEvent + 'static,
        F: GuardFn<C, P>,
    {
        let key = expand_iri(iri);
        if self.taken(&key) {
            return Err(RegistryError::Duplicate {
                iri: iri.to_string(),
            });
        }
        let g: GuardImpl<C> = Arc::new(move |ctx: &C, ev: &Event| {
            let p = P::from_event(ev)?;
            Ok(f.eval(ctx, &p))
        });
        self.guards.insert(key, g);
        Ok(self)
    }

    /// Register an action under `iri`.
    pub fn action<P, E, M, F>(&mut self, iri: &str, f: F) -> Result<&mut Self, RegistryError>
    where
        P: FromEvent + 'static,
        E: 'static,
        M: 'static,
        F: IntoAction<C, P, E, M>,
    {
        let key = expand_iri(iri);
        if self.taken(&key) {
            return Err(RegistryError::Duplicate {
                iri: iri.to_string(),
            });
        }
        let a: ActionImpl<C> = Arc::new(move |ctx: &mut C, ev: &Event, sys: &Sys| {
            let p = P::from_event(ev)?;
            let mut emit = Emit::<E>::new(sys.clone());
            f.run(ctx, &p, &mut emit);
            Ok(emit.into_emitted())
        });
        self.actions.insert(key, a);
        Ok(self)
    }

    /// The guard registered under `iri`.
    pub fn resolve_guard(&self, iri: &str) -> Result<&GuardImpl<C>, RegistryError> {
        self.guards
            .get(&expand_iri(iri))
            .ok_or_else(|| RegistryError::Unregistered {
                iri: iri.to_string(),
                role: "guard",
            })
    }

    /// The action registered under `iri`.
    pub fn resolve_action(&self, iri: &str) -> Result<&ActionImpl<C>, RegistryError> {
        self.actions
            .get(&expand_iri(iri))
            .ok_or_else(|| RegistryError::Unregistered {
                iri: iri.to_string(),
                role: "action",
            })
    }

    /// Check that every guard and action IRI the chart names resolves. This is what makes
    /// instantiating a chart with an unregistered IRI fail before it runs.
    pub fn check_chart(&self, chart: &Chart) -> Result<(), Vec<RegistryError>> {
        let refs = chart_functions(chart);
        let mut errors = Vec::new();
        for iri in &refs.guards {
            if let Err(e) = self.resolve_guard(iri) {
                errors.push(e);
            }
        }
        for iri in &refs.actions {
            if let Err(e) = self.resolve_action(iri) {
                errors.push(e);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// The guard and action IRIs a chart names, in document order, each once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FunctionRefs {
    /// Guard IRIs.
    pub guards: Vec<String>,
    /// Action IRIs.
    pub actions: Vec<String>,
}

/// Collect the guard and action IRIs a chart on the `rust` datamodel names.
pub fn chart_functions(chart: &Chart) -> FunctionRefs {
    fn push(list: &mut Vec<String>, iri: &str) {
        if !list.iter().any(|x| x == iri) {
            list.push(iri.to_string());
        }
    }
    fn block(b: &Block, out: &mut FunctionRefs) {
        for e in b {
            match &e.kind {
                ExecKind::Foreach { array, body, .. } => {
                    if lowering::is_iri(array) {
                        push(&mut out.actions, array.trim());
                    }
                    block(body, out);
                }
                ExecKind::Script(s) => {
                    if let Some(b) = s.body.as_deref().filter(|b| lowering::is_iri(b)) {
                        push(&mut out.actions, b.trim());
                    }
                }
                ExecKind::If { branches } => {
                    for br in branches {
                        if let Some(c) = &br.cond {
                            cond(c, out);
                        }
                        block(&br.body, out);
                    }
                }
                _ => {}
            }
        }
    }
    fn cond(c: &str, out: &mut FunctionRefs) {
        if let Some(iri) = lowering::guard_iri(c) {
            push(&mut out.guards, iri);
        }
    }
    fn state(s: &State, out: &mut FunctionRefs) {
        s.onentry.iter().for_each(|b| block(b, out));
        s.onexit.iter().for_each(|b| block(b, out));
        if let Some(i) = &s.initial_element {
            block(&i.transition.body, out);
        }
        for t in &s.transitions {
            if let Some(c) = &t.cond {
                cond(c, out);
            }
            block(&t.body, out);
        }
        for inv in &s.invokes {
            block(&inv.finalize, out);
        }
        s.children.iter().for_each(|c| state(c, out));
    }
    let mut out = FunctionRefs::default();
    chart.states.iter().for_each(|s| state(s, &mut out));
    out
}
