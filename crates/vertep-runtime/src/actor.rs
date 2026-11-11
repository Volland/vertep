//! The actor contract and its adapters for charts, async functions and streams.
//!
//! An actor receives SCXML events one at a time and produces zero or more outgoing events. A
//! running chart is one implementation ([`ChartActor`]); an async function ([`FnActor`]) and a
//! stream source ([`StreamActor`]) are others, so all of them are interchangeable as targets of
//! `<send>` and `<invoke>`. A turn is either finished at once ([`Turn::Ready`], a chart's
//! macrostep) or a `Send` future the runtime runs in the background ([`Turn::Pending`]); in both
//! cases the actor takes no other input until the turn is over.

use std::sync::Arc;

use futures::StreamExt;
use futures::lock::Mutex as AsyncMutex;
use futures::stream::BoxStream;
use vertep_core::datamodel::{
    Datamodel, DatamodelRegistry, Environment, Event, EventType, SessionInfo, Value,
};
use vertep_core::session::{self, Effect, Input, StartInput};

use crate::address::Address;
use crate::chart::ChartInfo;
use crate::shell::BoxFuture;

/// The event name of the input that starts an actor.
pub const START_EVENT: &str = "vt.start";
/// The event a stream actor sends itself to pull its next item.
pub const STREAM_NEXT_EVENT: &str = "vt.stream.next";

/// One input to an actor's turn.
#[derive(Clone, Debug, PartialEq)]
pub struct ActorInput {
    /// The event.
    pub event: Event,
    /// The message id it arrived with.
    pub message_id: String,
    /// The time stamped when it was dequeued, in milliseconds since the epoch.
    pub received_at_ms: i64,
}

/// Where an outgoing event goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutTarget {
    /// The invoking parent (`#_parent`).
    Parent,
    /// The origin of the input being handled (a reply).
    Origin,
    /// The actor itself.
    Itself,
    /// An address.
    Address(Address),
    /// A target text that is not an address; delivery fails with `error.communication`.
    Invalid(String),
}

/// One outgoing event.
#[derive(Clone, Debug, PartialEq)]
pub struct Outgoing {
    /// Where it goes.
    pub target: OutTarget,
    /// The event.
    pub event: Event,
    /// Delay in milliseconds.
    pub delay_ms: u64,
}

impl Outgoing {
    /// `name` with `data` to `target`, now.
    pub fn new(target: OutTarget, name: impl Into<String>, data: Value) -> Self {
        Outgoing {
            target,
            event: Event {
                name: name.into(),
                data,
                ..Event::default()
            },
            delay_ms: 0,
        }
    }
}

/// What a turn produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TurnOutput {
    /// Outgoing events.
    pub sends: Vec<Outgoing>,
    /// Send ids of delayed sends to cancel.
    pub cancels: Vec<String>,
    /// Set when the actor completed, with its done data.
    pub done: Option<Value>,
}

/// A failed turn: an error returned by the actor or a caught panic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActorFailure(pub String);

impl core::fmt::Display for ActorFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "actor failed: {}", self.0)
    }
}

impl std::error::Error for ActorFailure {}

/// The result of starting a turn.
pub enum Turn {
    /// The turn is complete.
    Ready(Result<TurnOutput, ActorFailure>),
    /// The turn completes when this future does.
    Pending(BoxFuture<Result<TurnOutput, ActorFailure>>),
}

impl std::fmt::Debug for Turn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Turn::Ready(r) => f.debug_tuple("Ready").field(r).finish(),
            Turn::Pending(_) => f.write_str("Pending"),
        }
    }
}

/// The actor contract: one event in, zero or more events out, one turn at a time.
// @tg: implements:: [[openspec:actor-model#Actor Trait]]
pub trait Actor {
    /// The actor's address.
    fn address(&self) -> &Address;
    /// Handles one input.
    fn turn(&mut self, input: ActorInput) -> Turn;
}

/// How an actor kind is started by `<invoke>`.
#[derive(Clone, Debug, PartialEq)]
pub struct ActorStart {
    /// The child's address.
    pub address: Address,
    /// The parent's address.
    pub parent: Address,
    /// The invoke id.
    pub invokeid: String,
    /// The evaluated `src`.
    pub src: Option<String>,
    /// The evaluated inline content.
    pub content: Option<Value>,
    /// The evaluated `namelist` and `<param>` values.
    pub params: Value,
}

/// Creates an actor of one kind for an `<invoke>`.
pub type ActorFactory =
    Arc<dyn Fn(&ActorStart) -> Result<Box<dyn Actor>, ActorFailure> + Send + Sync>;

/// The function behind a [`FnActor`]: one input in, a turn's output out.
pub type ActorFn =
    Arc<dyn Fn(ActorInput) -> BoxFuture<Result<TurnOutput, ActorFailure>> + Send + Sync>;

/// An actor that runs an async function per input.
pub struct FnActor {
    address: Address,
    f: ActorFn,
}

impl std::fmt::Debug for FnActor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FnActor")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

impl FnActor {
    /// An actor at `address` running `f` for every input.
    pub fn new(address: Address, f: ActorFn) -> Self {
        FnActor { address, f }
    }

    /// The factory of an invokable async function: on start it runs `f` on the invoke's
    /// parameters, and its result is the child's done data (`done.invoke.<id>`); an error is a
    /// failure, subject to `vt:restart`.
    // @tg: implements:: [[openspec:event-journal#External Results As Inputs]]
    pub fn invokable<F, Fut>(f: F) -> ActorFactory
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Value, String>> + Send + 'static,
    {
        let f = Arc::new(f);
        Arc::new(move |start: &ActorStart| {
            let f = f.clone();
            let params = start.params.clone();
            let run: ActorFn = Arc::new(move |input: ActorInput| {
                let f = f.clone();
                let params = params.clone();
                Box::pin(async move {
                    if input.event.name != START_EVENT {
                        return Ok(TurnOutput::default());
                    }
                    match f(params).await {
                        Ok(v) => Ok(TurnOutput {
                            done: Some(v),
                            ..TurnOutput::default()
                        }),
                        Err(e) => Err(ActorFailure(e)),
                    }
                })
            });
            Ok(Box::new(FnActor::new(start.address.clone(), run)) as Box<dyn Actor>)
        })
    }

    /// A request handler for a stateless worker: `f` gets each event and its result, when
    /// there is one, is sent back to the event's origin as `vt:reply`.
    pub fn handler<F, Fut>(f: F) -> ActorFn
    where
        F: Fn(Event) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Option<Value>, String>> + Send + 'static,
    {
        let f = Arc::new(f);
        Arc::new(move |input: ActorInput| {
            let f = f.clone();
            Box::pin(async move {
                match f(input.event).await {
                    Ok(Some(v)) => Ok(TurnOutput {
                        sends: vec![Outgoing::new(
                            OutTarget::Origin,
                            crate::vocab::REPLY_EVENT,
                            v,
                        )],
                        ..TurnOutput::default()
                    }),
                    Ok(None) => Ok(TurnOutput::default()),
                    Err(e) => Err(ActorFailure(e)),
                }
            })
        })
    }
}

impl Actor for FnActor {
    fn address(&self) -> &Address {
        &self.address
    }

    fn turn(&mut self, input: ActorInput) -> Turn {
        Turn::Pending((self.f)(input))
    }
}

/// The stream behind a [`StreamActor`].
pub type ActorStream = BoxStream<'static, Result<Value, String>>;

/// An actor that forwards the items of a stream to its parent as events named `item_event`,
/// one per turn, and completes when the stream ends.
pub struct StreamActor {
    address: Address,
    item_event: String,
    stream: Arc<AsyncMutex<ActorStream>>,
}

impl std::fmt::Debug for StreamActor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamActor")
            .field("address", &self.address)
            .field("item_event", &self.item_event)
            .finish_non_exhaustive()
    }
}

impl StreamActor {
    /// An actor at `address` forwarding `stream`.
    pub fn new(address: Address, item_event: impl Into<String>, stream: ActorStream) -> Self {
        StreamActor {
            address,
            item_event: item_event.into(),
            stream: Arc::new(AsyncMutex::new(stream)),
        }
    }

    /// The factory of an invokable stream: `f` opens the stream from the invoke's parameters.
    pub fn invokable<F>(item_event: &str, f: F) -> ActorFactory
    where
        F: Fn(Value) -> ActorStream + Send + Sync + 'static,
    {
        let item_event = item_event.to_owned();
        Arc::new(move |start: &ActorStart| {
            Ok(Box::new(StreamActor::new(
                start.address.clone(),
                item_event.clone(),
                f(start.params.clone()),
            )) as Box<dyn Actor>)
        })
    }
}

impl Actor for StreamActor {
    fn address(&self) -> &Address {
        &self.address
    }

    fn turn(&mut self, input: ActorInput) -> Turn {
        if input.event.name != START_EVENT && input.event.name != STREAM_NEXT_EVENT {
            return Turn::Ready(Ok(TurnOutput::default()));
        }
        let stream = self.stream.clone();
        let item_event = self.item_event.clone();
        Turn::Pending(Box::pin(async move {
            let next = stream.lock().await.next().await;
            match next {
                Some(Ok(v)) => Ok(TurnOutput {
                    sends: vec![
                        Outgoing::new(OutTarget::Parent, item_event, v),
                        Outgoing::new(OutTarget::Itself, STREAM_NEXT_EVENT, Value::Undefined),
                    ],
                    ..TurnOutput::default()
                }),
                Some(Err(e)) => Err(ActorFailure(e)),
                None => Ok(TurnOutput {
                    done: Some(Value::Undefined),
                    ..TurnOutput::default()
                }),
            }
        }))
    }
}

/// A chart run as an actor without a journal: an invoked chart child, whose inputs are
/// journaled by its parent.
pub struct ChartActor {
    address: Address,
    info: Arc<ChartInfo>,
    registry: Arc<dyn DatamodelRegistry + Send + Sync>,
    session: Option<session::Session>,
    dm: Option<Box<dyn Datamodel>>,
    start_data: Vec<(String, Value)>,
    ioprocessors: Value,
}

impl std::fmt::Debug for ChartActor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChartActor")
            .field("address", &self.address)
            .field("version", &self.info.version.as_str())
            .finish_non_exhaustive()
    }
}

impl ChartActor {
    /// A chart actor at `address` running `info`; it starts on its first input
    /// ([`START_EVENT`]) with `start_data` for its top-level data items.
    pub fn new(
        address: Address,
        info: Arc<ChartInfo>,
        registry: Arc<dyn DatamodelRegistry + Send + Sync>,
        start_data: Vec<(String, Value)>,
        ioprocessors: Value,
    ) -> Self {
        ChartActor {
            address,
            info,
            registry,
            session: None,
            dm: None,
            start_data,
            ioprocessors,
        }
    }

    /// The active state ids, once started.
    pub fn active(&self) -> Vec<String> {
        self.session
            .as_ref()
            .map(|s| self.info.active(s))
            .unwrap_or_default()
    }

    fn output(&self, effects: Vec<Effect>) -> TurnOutput {
        let mut out = TurnOutput::default();
        for e in effects {
            match e {
                Effect::Send(r) => {
                    let target = match r.target.as_deref() {
                        None | Some("#_internal") => OutTarget::Itself,
                        Some("#_parent") => OutTarget::Parent,
                        Some(t) => match t.strip_prefix("#_scxml_") {
                            Some(sid) => Address::parse(sid)
                                .map(OutTarget::Address)
                                .unwrap_or_else(|_| OutTarget::Invalid(t.to_owned())),
                            None => Address::parse(t)
                                .map(OutTarget::Address)
                                .unwrap_or_else(|_| OutTarget::Invalid(t.to_owned())),
                        },
                    };
                    let mut event = r.event;
                    if event.sendid.is_none() {
                        event.sendid = Some(r.sendid);
                    }
                    out.sends.push(Outgoing {
                        target,
                        event,
                        delay_ms: r.delay_ms,
                    });
                }
                Effect::Cancel { sendid } => out.cancels.push(sendid),
                Effect::SessionDone { data, cancelled } if !cancelled => out.done = Some(data),
                _ => {}
            }
        }
        out
    }
}

impl Actor for ChartActor {
    fn address(&self) -> &Address {
        &self.address
    }

    fn turn(&mut self, input: ActorInput) -> Turn {
        let env = Environment {
            now_ms: input.received_at_ms,
            seed: crate::address::seed_of(self.address.node()),
        };
        let result = if let Some(s) = self.session.take() {
            let Some(dm) = self.dm.as_mut() else {
                return Turn::Ready(Err(ActorFailure("chart actor has no datamodel".into())));
            };
            if input.event.name == START_EVENT {
                self.session = Some(s);
                return Turn::Ready(Ok(TurnOutput::default()));
            }
            session::step(
                &self.info.compiled,
                s,
                Input::event(input.event, env),
                dm.as_mut(),
            )
            .map_err(|e| ActorFailure(format!("{e:?}")))
        } else {
            let mut dm = match vertep_core::null::create_datamodel(
                self.registry.as_ref(),
                self.info.compiled.datamodel(),
            ) {
                Ok(d) => d,
                Err(e) => return Turn::Ready(Err(ActorFailure(format!("{e:?}")))),
            };
            let mut options = session::Options::default();
            options
                .send_types
                .extend(self.info.vt_send_types.iter().cloned());
            let start = StartInput {
                info: SessionInfo {
                    sessionid: self.address.as_str().to_owned(),
                    name: self.info.ir.name.clone(),
                    ioprocessors: self.ioprocessors.clone(),
                },
                env,
                options,
                data: self.start_data.clone(),
                sessions: None,
            };
            let r = session::start(&self.info.compiled, start, dm.as_mut())
                .map_err(|e| ActorFailure(format!("{e:?}")));
            self.dm = Some(dm);
            r
        };
        Turn::Ready(result.map(|o| {
            self.session = Some(o.session);
            self.output(o.effects)
        }))
    }
}

/// A platform event named `name` with `data`.
pub fn platform_event(name: &str, data: Value) -> Event {
    Event {
        name: name.to_owned(),
        type_: EventType::Platform,
        data,
        ..Event::default()
    }
}
