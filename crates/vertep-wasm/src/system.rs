//! The actor system a worker hosts: one runtime node on one store, driven by the shell, and
//! the command bridge's operations on it. Everything here is target-independent; the
//! WebAssembly surface (`js`) and the native helper binary are thin layers over [`System`].
//!
//! Commands are `(op, args)` with JSON arguments and JSON results, so the same command script
//! runs natively and in WebAssembly and the outputs can be compared.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value as J, json};
use vertep_core::datamodel::{Environment, Event, EventType, SessionInfo, Value};
use vertep_core::session::{self, Options, StartInput};
use vertep_runtime::chart::{ChartInfo, find_state};
use vertep_runtime::{
    ActivationError, ActorState, Address, AskError, EntryBody, EntryData, Envelope, Node,
    NodeConfig, NodeHandle, Shell,
};
use vertep_store::Position;

use crate::charts::{ChartFormat, read_chart};
use crate::error::{ApiError, Code, Res};
use crate::functions::{self, Datamodels, Implementations, Slot, json_to_value, value_to_json};
use crate::store::{StorageKind, Store, StoreOptions};

/// How a system is created.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemOptions {
    /// Storage.
    pub store: StoreOptions,
    /// Development mode: every macrostep is replayed from the journal and compared with the live
    /// result, which reports impure guards and actions.
    pub dev: bool,
    /// Idle deactivation of virtual actors, in milliseconds.
    pub idle_timeout_ms: Option<i64>,
    /// Guard IRIs the implementations provide.
    pub guards: Vec<String>,
    /// Action IRIs the implementations provide.
    pub actions: Vec<String>,
}

impl SystemOptions {
    /// Parses `{storage, path?, journal?, dev?, idleTimeoutMs?, guards?, actions?}`.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for a missing storage or malformed field.
    pub fn from_json(j: &J) -> Res<Self> {
        let kind = StorageKind::parse(j.get("storage").and_then(J::as_str))?;
        let path = j
            .get("path")
            .and_then(J::as_str)
            .unwrap_or("vertep.db")
            .to_owned();
        if path.is_empty() {
            return Err(ApiError::arg("path must not be empty"));
        }
        let strings = |k: &str| -> Res<Vec<String>> {
            match j.get(k) {
                None | Some(J::Null) => Ok(Vec::new()),
                Some(J::Array(a)) => a
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| ApiError::arg(format!("{k} lists strings")))
                    })
                    .collect(),
                Some(_) => Err(ApiError::arg(format!("{k} is an array of IRIs"))),
            }
        };
        Ok(SystemOptions {
            store: StoreOptions {
                kind,
                path,
                journal: j.get("journal").and_then(J::as_bool).unwrap_or(false),
            },
            dev: j.get("dev").and_then(J::as_bool).unwrap_or(false),
            idle_timeout_ms: j.get("idleTimeoutMs").and_then(J::as_i64),
            guards: strings("guards")?,
            actions: strings("actions")?,
        })
    }
}

/// What a subscription listens for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubKind {
    Snapshot,
    Transitions,
}

#[derive(Clone, Debug)]
struct Sub {
    address: Address,
    kind: SubKind,
    last_digest: Option<String>,
}

/// An actor system on one store.
pub struct System {
    node: Node,
    store: Store,
    dev: bool,
    _slot: Option<Slot>,
    known: BTreeSet<Address>,
    seen: BTreeMap<Address, u64>,
    subs: BTreeMap<u64, Sub>,
    next_sub: u64,
    out: Vec<J>,
}

impl std::fmt::Debug for System {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("System")
            .field("store", &self.store)
            .field("dev", &self.dev)
            .finish_non_exhaustive()
    }
}

fn activation(e: ActivationError) -> ApiError {
    ApiError::new(Code::Activation, e.to_string())
}

fn str_arg<'a>(args: &'a J, k: &str) -> Res<&'a str> {
    args.get(k)
        .and_then(J::as_str)
        .ok_or_else(|| ApiError::arg(format!("{k} is a required string")))
}

/// Reads an event: `{type, ...payload fields}`, `{type, data}` or `{name, data}`.
///
/// When the object has a `data` key and no other key besides the name, `data` is the payload;
/// otherwise every key but the name is a field of a record payload.
///
/// # Errors
///
/// `InvalidArgument` without a name.
pub fn event_from_json(j: &J) -> Res<Event> {
    let o = j
        .as_object()
        .ok_or_else(|| ApiError::arg("an event is an object with a type"))?;
    let name_key = if o.contains_key("type") {
        "type"
    } else {
        "name"
    };
    let name = o
        .get(name_key)
        .and_then(J::as_str)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| ApiError::arg("an event has a non-empty type"))?;
    let rest: Map<String, J> = o
        .iter()
        .filter(|(k, _)| *k != "type" && *k != "name")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let data = if rest.len() == 1 && rest.contains_key("data") {
        json_to_value(&rest["data"])
    } else if rest.is_empty() {
        Value::Undefined
    } else {
        json_to_value(&J::Object(rest))
    };
    Ok(Event {
        data,
        ..Event::named(name, EventType::External)
    })
}

/// The context of a datamodel snapshot as plain JSON: the CEL datamodel's `ctx` (decoded from
/// its exact tagged encoding), the `rust` datamodel's `context`, or the whole snapshot.
pub fn context_json(dm: &Value) -> J {
    let Value::Object(entries) = dm else {
        return value_to_json(dm);
    };
    let get = |k: &str| entries.iter().find(|(n, _)| n == k).map(|(_, v)| v);
    if let Some(Value::String(f)) = get("format")
        && f.starts_with("vertep-dm-cel")
    {
        return get("ctx").map_or(J::Null, cel_plain);
    }
    if entries.len() == 2
        && let (Some(c), Some(_)) = (get("context"), get("vars"))
    {
        return value_to_json(c);
    }
    value_to_json(dm)
}

/// The CEL snapshot encoding (`{"$int": "1"}`, `{"$map": [[k, v]]}`, ...) as plain JSON.
fn cel_plain(v: &Value) -> J {
    match v {
        Value::Array(a) => J::Array(a.iter().map(cel_plain).collect()),
        Value::Object(o) if o.len() == 1 => {
            let (tag, x) = (&o[0].0, &o[0].1);
            let text = match x {
                Value::String(t) => Some(t.as_str()),
                _ => None,
            };
            let num = |n: Option<f64>| {
                n.and_then(serde_json::Number::from_f64)
                    .map_or(J::Null, J::Number)
            };
            match tag.as_str() {
                "$int" | "$uint" => num(text.and_then(|t| t.parse::<f64>().ok())),
                "$double" => num(text
                    .and_then(|t| u64::from_str_radix(t, 16).ok())
                    .map(f64::from_bits)),
                "$decimal" => text
                    .and_then(|t| t.parse::<f64>().ok())
                    .filter(|f| {
                        text.is_some_and(|t| f.to_string() == t || t.parse::<i64>().is_ok())
                    })
                    .map_or_else(
                        || J::String(text.unwrap_or_default().to_owned()),
                        |f| num(Some(f)),
                    ),
                "$timestamp" | "$duration" | "$type" => {
                    J::String(text.unwrap_or_default().to_owned())
                }
                "$bytes" => value_to_json(x),
                "$map" => {
                    let mut m = Map::new();
                    if let Value::Array(pairs) = x {
                        for p in pairs {
                            if let Value::Array(kv) = p
                                && kv.len() == 2
                            {
                                let k = match cel_plain(&kv[0]) {
                                    J::String(s) => s,
                                    other => other.to_string(),
                                };
                                m.insert(k, cel_plain(&kv[1]));
                            }
                        }
                    }
                    J::Object(m)
                }
                _ => value_to_json(v),
            }
        }
        other => value_to_json(other),
    }
}

/// A snapshot as the client sees it.
pub fn snapshot_json(address: &Address, s: &ActorState) -> J {
    json!({
        "address": address.as_str(),
        "position": s.position,
        "version": s.version,
        "configuration": s.configuration,
        "context": context_json(&s.datamodel),
        "terminated": s.terminated,
    })
}

/// A transition taken, decoded from its node IRI: source state, index and targets.
fn transition_json(info: Option<&ChartInfo>, iri: &str) -> J {
    let frag = iri.rsplit_once('#').map_or(iri, |(_, f)| f);
    let (source, index) = match frag.rsplit_once(":t") {
        Some((s, i)) => (s.strip_prefix("s.").unwrap_or(s), i.parse::<usize>().ok()),
        None => (frag, None),
    };
    let targets: Vec<String> = info
        .and_then(|info| find_state(&info.ir.states, source))
        .and_then(|st| index.and_then(|i| st.transitions.get(i)))
        .map(|t| t.targets.clone())
        .unwrap_or_default();
    json!({ "iri": iri, "source": source, "index": index, "targets": targets })
}

/// A journal entry as the client sees it.
pub fn entry_json(e: &EntryData) -> J {
    let mut o = json!({
        "position": e.position,
        "version": e.version,
        "messageId": e.message_id,
        "timestamp": e.timestamp_ms,
        "digest": e.digest,
    });
    match &e.body {
        EntryBody::Input { input, effects } => {
            o["kind"] = J::String("input".into());
            o["event"] = functions::event_to_json(&input.event);
            o["transitions"] = J::Array(
                effects
                    .transitions
                    .iter()
                    .map(|t| J::String(t.clone()))
                    .collect(),
            );
            o["timers"] = J::Array(
                effects
                    .timers
                    .iter()
                    .map(|t| json!({ "iri": t.iri, "due": t.due_ms, "event": t.event.name }))
                    .collect(),
            );
            o["sends"] = J::Array(
                effects
                    .sends
                    .iter()
                    .map(|s| json!({ "event": s.event.name, "target": s.target, "type": s.type_ }))
                    .collect(),
            );
            if let Some(t) = &input.timer {
                o["timer"] = J::String(t.clone());
            }
        }
        EntryBody::Migration { migration, .. } => {
            o["kind"] = J::String("migration".into());
            o["migration"] = J::String(migration.clone());
        }
        EntryBody::Fork {
            source, position, ..
        } => {
            o["kind"] = J::String("fork".into());
            o["source"] = json!({ "actor": source, "position": position });
        }
    }
    o
}

/// The default chart type of a chart registered without one: its version IRI.
fn default_type(version: &str) -> String {
    version.to_owned()
}

impl System {
    /// Creates a system on `store`, with `imp` (if any) providing the named guards and actions
    /// the options list.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` when a function IRI is listed twice.
    pub fn new(
        opts: &SystemOptions,
        store: Store,
        shell: Shell,
        imp: Option<Rc<dyn Implementations>>,
    ) -> Res<System> {
        let (registry, slot) = match imp {
            Some(i) => {
                let (r, s) =
                    functions::bind(i, &opts.guards, &opts.actions).map_err(ApiError::arg)?;
                (r, Some(s))
            }
            None => (Arc::new(Default::default()), None),
        };
        let mut config = NodeConfig::default();
        if let Some(t) = opts.idle_timeout_ms {
            config.idle_timeout_ms = t;
        }
        if opts.dev {
            // every macrostep must be re-run on replay, so no snapshot may short-cut it
            config.snapshot_interval = 0;
        }
        let node = Node::builder(store.storage(), shell)
            .datamodels(Arc::new(Datamodels::new(registry)))
            .config(config)
            .build();
        Ok(System {
            node,
            store,
            dev: opts.dev,
            _slot: slot,
            known: BTreeSet::new(),
            seen: BTreeMap::new(),
            subs: BTreeMap::new(),
            next_sub: 1,
            out: Vec::new(),
        })
    }

    /// The runtime node.
    pub fn node(&mut self) -> &mut Node {
        &mut self.node
    }

    /// A handle for callers outside the system (requests, transports).
    pub fn handle(&self) -> NodeHandle {
        self.node.handle()
    }

    /// The store.
    pub fn store(&self) -> &Store {
        &self.store
    }

    fn address(&self, args: &J, key: &str) -> Res<Address> {
        let text = str_arg(args, key)?;
        Address::parse(text).map_err(|e| ApiError::arg(format!("{key}: {}", e.reason)))
    }

    fn know(&mut self, a: &Address) {
        if self.known.insert(a.clone()) {
            let head = self.head(a);
            self.seen.insert(a.clone(), head);
        }
    }

    fn head(&self, a: &Address) -> u64 {
        self.node
            .storage()
            .journal()
            .head(a.node())
            .map(Position::get)
            .unwrap_or(0)
    }

    /// Registers the chart in `args` (`source`, `format?`, `type?`): `{type, version}`.
    fn register(&mut self, args: &J) -> Res<(String, String)> {
        let source = str_arg(args, "source")?;
        let format = match args.get("format").and_then(J::as_str) {
            Some(f) => ChartFormat::parse(f)?,
            None => ChartFormat::sniff(source),
        };
        let chart = read_chart(source, format)?;
        let version = crate::charts::version_of(&chart);
        let ty = match args.get("type").and_then(J::as_str) {
            Some(t) => t.to_owned(),
            None => default_type(&version),
        };
        if self
            .node
            .latest(&ty)
            .is_none_or(|i| i.version.as_str() != version)
        {
            self.node.register_type(&ty, chart).map_err(|e| match e {
                vertep_runtime::RegisterError::BadType(t) => {
                    ApiError::arg(format!("chart type {t:?} is not an absolute IRI"))
                }
                other => ApiError::chart(vec![crate::error::Violation {
                    message: other.to_string(),
                    line: 0,
                    column: 0,
                    path: None,
                }]),
            })?;
        }
        Ok((ty, version))
    }

    /// The snapshot of `a`: its live or replayed state, or, before its first entry, the
    /// configuration its chart starts in (computed, not journaled).
    fn snapshot(&mut self, a: &Address) -> Res<J> {
        if self.head(a) == 0 {
            return self.initial_snapshot(a);
        }
        let s = self.node.state(a).map_err(activation)?;
        Ok(snapshot_json(a, &s))
    }

    fn initial_snapshot(&self, a: &Address) -> Res<J> {
        let ty = a
            .as_str()
            .rsplit_once('/')
            .map(|(t, _)| t)
            .unwrap_or_default();
        let info = self
            .node
            .latest(ty)
            .ok_or_else(|| ApiError::new(Code::NotFound, format!("no chart type for {a}")))?;
        let mut dm = vertep_core::null::create_datamodel(
            self.node.datamodels().as_ref(),
            info.compiled.datamodel(),
        )
        .map_err(|e| ApiError::new(Code::Activation, format!("{e:?}")))?;
        let start = StartInput {
            info: SessionInfo {
                sessionid: a.as_str().to_owned(),
                name: info.ir.name.clone(),
                ioprocessors: Value::Undefined,
            },
            env: Environment { now_ms: 0, seed: 0 },
            options: Options::default(),
            data: Vec::new(),
            sessions: None,
        };
        let o = session::start(&info.compiled, start, dm.as_mut())
            .map_err(|e| ApiError::new(Code::Activation, format!("{e:?}")))?;
        let s = ActorState {
            position: 0,
            version: info.version.as_str().to_owned(),
            configuration: info.active(&o.session),
            datamodel: dm.snapshot(),
            terminated: o.session.is_terminated(),
        };
        Ok(snapshot_json(a, &s))
    }

    /// Runs one bridge command.
    ///
    /// # Errors
    ///
    /// The command's [`ApiError`].
    pub fn command(&mut self, op: &str, args: &J) -> Res<J> {
        let r = self.command_inner(op, args);
        self.pump();
        r
    }

    fn command_inner(&mut self, op: &str, args: &J) -> Res<J> {
        match op {
            "register" => {
                let (ty, version) = self.register(args)?;
                Ok(json!({ "type": ty, "version": version }))
            }
            // @tg: implements:: [[openspec:wasm-bindings#Actor Creation From Chart Sources]]
            "spawn" => {
                let (ty, version) = if args.get("source").is_some() {
                    self.register(args)?
                } else {
                    let ty = str_arg(args, "type")?.to_owned();
                    let v = self
                        .node
                        .latest(&ty)
                        .ok_or_else(|| ApiError::new(Code::NotFound, format!("chart type {ty}")))?
                        .version
                        .as_str()
                        .to_owned();
                    (ty, v)
                };
                let key = match args.get("key").and_then(J::as_str) {
                    Some(k) => k.to_owned(),
                    None => {
                        let id = self.node.fresh_id("");
                        id.rsplit([':', '/']).next().unwrap_or(&id).to_owned()
                    }
                };
                let a = Address::virtual_actor(&ty, &key)
                    .map_err(|e| ApiError::arg(format!("key: {}", e.reason)))?;
                self.know(&a);
                let snap = self.snapshot(&a)?;
                Ok(
                    json!({ "address": a.as_str(), "type": ty, "version": version, "snapshot": snap }),
                )
            }
            // @tg: implements:: [[openspec:wasm-bindings#Actor Creation From Chart Sources]]
            "virtual" => {
                let ty = str_arg(args, "type")?;
                let key = str_arg(args, "key")?;
                if self.node.latest(ty).is_none() {
                    return Err(ApiError::new(
                        Code::NotFound,
                        format!("no chart registered as type {ty}"),
                    ));
                }
                let a = Address::virtual_actor(ty, key)
                    .map_err(|e| ApiError::arg(format!("key: {}", e.reason)))?;
                self.know(&a);
                Ok(json!({ "address": a.as_str() }))
            }
            // @tg: implements:: [[openspec:wasm-bindings#Event Sending From JavaScript]]
            "send" => {
                let to = self.address(args, "to")?;
                let ev = event_from_json(args.get("event").unwrap_or(&J::Null))?;
                self.know(&to);
                let id = match args.get("messageId").and_then(J::as_str) {
                    Some(id) => {
                        self.node
                            .send_envelope(Envelope::new(id.to_owned(), &to, ev))
                            .map_err(|e| ApiError::new(Code::Delivery, e.to_string()))?;
                        id.to_owned()
                    }
                    None => self
                        .node
                        .send(&to, ev)
                        .map_err(|e| ApiError::new(Code::Delivery, e.to_string()))?,
                };
                Ok(json!({ "messageId": id }))
            }
            "snapshot" => {
                let a = self.address(args, "address")?;
                self.snapshot(&a)
            }
            // @tg: implements:: [[openspec:wasm-bindings#SCXML Semantics In JavaScript]]
            "can" => {
                let a = self.address(args, "address")?;
                let ev = event_from_json(args.get("event").unwrap_or(&J::Null))?;
                if self.head(&a) == 0 {
                    return Ok(json!({ "can": false }));
                }
                let can = self.node.can(&a, &ev).map_err(activation)?;
                Ok(json!({ "can": can }))
            }
            // @tg: implements:: [[openspec:wasm-bindings#Journal Inspection And Time Travel]]
            "journal" => {
                let a = self.address(args, "address")?;
                let from = args.get("from").and_then(J::as_u64).unwrap_or(0);
                if self.head(&a) == 0 {
                    return Ok(J::Array(Vec::new()));
                }
                let entries = self.node.journal(&a).map_err(activation)?;
                Ok(J::Array(
                    entries
                        .iter()
                        .filter(|e| e.position >= from)
                        .map(entry_json)
                        .collect(),
                ))
            }
            // @tg: implements:: [[openspec:wasm-bindings#Journal Inspection And Time Travel]]
            "asOf" => {
                let a = self.address(args, "address")?;
                let position = match (
                    args.get("position").and_then(J::as_u64),
                    args.get("time").and_then(J::as_i64),
                ) {
                    (Some(p), _) => p,
                    (None, Some(t)) => {
                        let entries = if self.head(&a) == 0 {
                            Vec::new()
                        } else {
                            self.node.journal(&a).map_err(activation)?
                        };
                        entries
                            .iter()
                            .filter(|e| e.timestamp_ms <= t)
                            .map(|e| e.position)
                            .max()
                            .unwrap_or(0)
                    }
                    (None, None) => return Err(ApiError::arg("asOf needs a position or a time")),
                };
                if position == 0 {
                    return self.initial_snapshot(&a);
                }
                let s = self.node.state_at(&a, position).map_err(activation)?;
                Ok(snapshot_json(&a, &s))
            }
            "query" => {
                let sparql = str_arg(args, "sparql")?;
                self.store
                    .query(sparql, args.get("time").and_then(J::as_i64))
            }
            // @tg: implements:: [[openspec:wasm-bindings#Snapshot And Transition Subscriptions]]
            "subscribe" => {
                let a = self.address(args, "address")?;
                let kind = match args.get("kind").and_then(J::as_str) {
                    Some("snapshot") | None => SubKind::Snapshot,
                    Some("transitions") => SubKind::Transitions,
                    Some(k) => return Err(ApiError::arg(format!("unknown subscription {k:?}"))),
                };
                self.know(&a);
                let current = self
                    .snapshot(&a)
                    .ok()
                    .map(|s| json!([s["configuration"], s["context"]]).to_string());
                let id = self.next_sub;
                self.next_sub += 1;
                self.subs.insert(
                    id,
                    Sub {
                        address: a,
                        kind,
                        last_digest: current,
                    },
                );
                Ok(json!({ "id": id }))
            }
            "unsubscribe" => {
                let id = args
                    .get("id")
                    .and_then(J::as_u64)
                    .ok_or_else(|| ApiError::arg("id is a subscription number"))?;
                let found = self.subs.remove(&id).is_some();
                Ok(json!({ "removed": found }))
            }
            "pendingTimers" => Ok(J::Array(
                self.node
                    .pending_timers()
                    .iter()
                    .map(|(due, t)| json!({ "due": due, "actor": t.actor, "event": t.event.name }))
                    .collect(),
            )),
            "actors" => {
                let actors = self
                    .node
                    .storage()
                    .journal()
                    .actors()
                    .map_err(|e| ApiError::new(Code::Store, e.to_string()))?;
                Ok(J::Array(
                    actors
                        .iter()
                        .map(|a| J::String(a.as_str().to_owned()))
                        .collect(),
                ))
            }
            "now" => Ok(json!(self.node.now_ms())),
            other => Err(ApiError::arg(format!("unknown command {other:?}"))),
        }
    }

    /// Runs the node until nothing is due now, then collects the notifications of the entries
    /// appended meanwhile. Returns the scheduling rounds that did something.
    pub fn pump(&mut self) -> usize {
        let n = self.node.run_until_idle();
        self.collect();
        n
    }

    /// The pending notifications, in journal order per actor.
    pub fn take_notifications(&mut self) -> Vec<J> {
        std::mem::take(&mut self.out)
    }

    /// The earliest time the node has something due.
    pub fn next_deadline(&self) -> Option<i64> {
        self.node.next_deadline()
    }

    // @tg: implements:: [[openspec:wasm-bindings#Snapshot And Transition Subscriptions]]
    fn collect(&mut self) {
        let known: Vec<Address> = self.known.iter().cloned().collect();
        for a in known {
            let head = self.head(&a);
            let seen = self.seen.get(&a).copied().unwrap_or(0);
            if head <= seen {
                continue;
            }
            self.seen.insert(a.clone(), head);
            let watched: Vec<u64> = self
                .subs
                .iter()
                .filter(|(_, s)| s.address == a)
                .map(|(id, _)| *id)
                .collect();
            if watched.is_empty() && !self.dev {
                continue;
            }
            let recs = match self
                .node
                .storage()
                .journal()
                .read(a.node(), Position::new(seen + 1))
            {
                Ok(r) => r,
                Err(_) => continue,
            };
            for rec in recs {
                let Ok(e) = EntryData::from_record(&a, &rec) else {
                    continue;
                };
                if self.dev && e.position == head {
                    self.check_purity(&a, head);
                }
                for id in &watched {
                    let Some(sub) = self.subs.get(id).cloned() else {
                        continue;
                    };
                    match sub.kind {
                        SubKind::Transitions => {
                            let EntryBody::Input { input, effects } = &e.body else {
                                continue;
                            };
                            if effects.transitions.is_empty() {
                                continue;
                            }
                            let info = self.node.chart(&e.version);
                            let ts: Vec<J> = effects
                                .transitions
                                .iter()
                                .map(|t| transition_json(info.as_deref(), t))
                                .collect();
                            self.out.push(json!({
                                "kind": "transition",
                                "sub": id,
                                "address": a.as_str(),
                                "position": e.position,
                                "event": functions::event_to_json(&input.event),
                                "transitions": ts,
                            }));
                        }
                        SubKind::Snapshot => {
                            let state = if e.position == head {
                                self.node.state(&a)
                            } else {
                                self.node.state_at(&a, e.position)
                            };
                            let Ok(state) = state else { continue };
                            // one snapshot per macrostep that changed the configuration or
                            // the context
                            let key = json!([state.configuration, context_json(&state.datamodel)])
                                .to_string();
                            if sub.last_digest.as_deref() == Some(key.as_str()) {
                                continue;
                            }
                            if let Some(s) = self.subs.get_mut(id) {
                                s.last_digest = Some(key);
                            }
                            self.out.push(json!({
                                "kind": "snapshot",
                                "sub": id,
                                "snapshot": snapshot_json(&a, &state),
                            }));
                        }
                    }
                }
            }
        }
    }

    /// Development mode: re-runs the journal up to `position` and compares the result with the
    /// live actor; a difference means a guard or action is impure.
    // @tg: implements:: [[openspec:wasm-bindings#Worker-Hosted Actor System]] {confidence: partial, note: "development-mode purity check (task 3.5)"}
    fn check_purity(&mut self, a: &Address, position: u64) {
        let Ok(live) = self.node.state(a) else {
            return;
        };
        let message = match self.node.state_at(a, position) {
            Ok(replayed)
                if replayed.configuration == live.configuration
                    && replayed.datamodel == live.datamodel =>
            {
                return;
            }
            Ok(replayed) => format!(
                "replaying position {position} gave {:?} with context {}, the live step {:?} with context {}",
                replayed.configuration,
                value_to_json(&replayed.datamodel),
                live.configuration,
                value_to_json(&live.datamodel)
            ),
            Err(e) => e.to_string(),
        };
        self.out.push(json!({
            "kind": "impurity",
            "address": a.as_str(),
            "position": position,
            "message": format!("a guard or action is impure: {message}"),
        }));
    }
}

/// Sends `event` to `to` and waits for the reply, at most `timeout_ms` on the node's clock.
///
/// # Errors
///
/// `RequestTimeout` when no reply arrives in time (a late reply is dropped), and `Delivery`.
// @tg: implements:: [[openspec:wasm-bindings#Event Sending From JavaScript]]
pub async fn request(handle: NodeHandle, to: Address, event: Event, timeout_ms: u64) -> Res<J> {
    match handle
        .ask(&to, event, Duration::from_millis(timeout_ms))
        .await
    {
        Ok(reply) => Ok(functions::event_to_json(&reply)),
        Err(AskError::Timeout) => Err(ApiError::new(
            Code::RequestTimeout,
            format!("no reply from {to} within {timeout_ms} ms"),
        )),
        Err(AskError::Delivery(e)) => Err(ApiError::new(Code::Delivery, e.to_string())),
    }
}
