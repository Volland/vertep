//! Capability grants: what an invoked LLM, tool or plan actor may reach, and their enforcement
//! in the send path.
//!
//! An invoking state or `<invoke>` declares grants as `vt:grant` extension elements,
//! `<vt:grant targets="#_parent vt:actor/Crm/1" tools="vt:tool/search" deciders="human/inbox"
//! functions="vt:action/notify"/>` (several elements add up), or as the attributes
//! `vt:grantTargets`, `vt:grantTools`, `vt:grantDeciders` and `vt:grantFunctions`. The
//! effective grant of a child is the intersection of its parent's effective grant and its own
//! declared grant, so authority never widens down the invoke tree. A send to an ungranted
//! target is refused before delivery: `error.communication` with the reason `not-granted` is
//! raised in the sender, and the attempt is recorded.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use vertep_core::datamodel::{EventType, Value};
use vertep_core::ir::{Extension, ExtensionValue, VT_NS};
use vertep_runtime::actor::{
    Actor, ActorFactory, ActorInput, OutTarget, Outgoing, Turn, TurnOutput,
};
use vertep_runtime::chart::element_attributes;
use vertep_runtime::{ActorFailure, ActorStart, Address};

use crate::json::{obj, s};
use crate::vocab;

/// The target name of the invoking parent.
pub const PARENT: &str = "#_parent";

/// A set of capabilities.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// Send targets: actor IRIs and `#_parent`.
    pub targets: BTreeSet<String>,
    /// Tool IRIs.
    pub tools: BTreeSet<String>,
    /// Decider bindings.
    pub deciders: BTreeSet<String>,
    /// Functions a plan may call (Rust guard or action IRIs, `vt:call` names).
    pub functions: BTreeSet<String>,
}

fn split(v: Option<&String>) -> BTreeSet<String> {
    v.map(|t| t.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

impl Grant {
    /// A grant with these targets and tools.
    pub fn new<I: IntoIterator<Item = S>, J: IntoIterator<Item = S>, S: Into<String>>(
        targets: I,
        tools: J,
    ) -> Self {
        Grant {
            targets: targets.into_iter().map(Into::into).collect(),
            tools: tools.into_iter().map(Into::into).collect(),
            ..Grant::default()
        }
    }

    /// The grant that allows only replying to the parent.
    pub fn parent_only() -> Self {
        Grant::new([PARENT], Vec::<&str>::new())
    }

    /// The intersection of two grants.
    // @tg: implements:: [[openspec:llm-actors#Capability Grants]]
    pub fn intersect(&self, other: &Grant) -> Grant {
        Grant {
            targets: self.targets.intersection(&other.targets).cloned().collect(),
            tools: self.tools.intersection(&other.tools).cloned().collect(),
            deciders: self
                .deciders
                .intersection(&other.deciders)
                .cloned()
                .collect(),
            functions: self
                .functions
                .intersection(&other.functions)
                .cloned()
                .collect(),
        }
    }

    /// The union of two grants (several declarations on one owner).
    pub fn union(&self, other: &Grant) -> Grant {
        Grant {
            targets: self.targets.union(&other.targets).cloned().collect(),
            tools: self.tools.union(&other.tools).cloned().collect(),
            deciders: self.deciders.union(&other.deciders).cloned().collect(),
            functions: self.functions.union(&other.functions).cloned().collect(),
        }
    }

    /// Whether a send to `target` (an actor IRI, a tool IRI or `#_parent`) is granted.
    pub fn allows(&self, target: &str) -> bool {
        self.targets.contains(target) || self.tools.contains(target)
    }

    /// Whether calling `tool` is granted.
    pub fn allows_tool(&self, tool: &str) -> bool {
        self.tools.contains(tool)
    }

    /// Whether `function` is granted.
    pub fn allows_function(&self, function: &str) -> bool {
        self.functions.contains(function)
    }
}

/// The grant declared by `vt:grant` elements or `vt:grant*` attributes; `None` when there is
/// none.
pub fn declared(exts: &[Extension]) -> Option<Grant> {
    let mut out: Option<Grant> = None;
    let mut add = |g: Grant| {
        out = Some(match out.take() {
            Some(o) => o.union(&g),
            None => g,
        });
    };
    let mut attrs = Grant::default();
    let mut any_attr = false;
    for e in exts.iter().filter(|e| e.ns == VT_NS) {
        match (&e.value, e.local.as_str()) {
            (ExtensionValue::Element(xml), "grant") => {
                let a = element_attributes(xml);
                add(Grant {
                    targets: split(a.get("targets")),
                    tools: split(a.get("tools")),
                    deciders: split(a.get("deciders")),
                    functions: split(a.get("functions")),
                });
            }
            (ExtensionValue::Attribute(v), local) => {
                let set: BTreeSet<String> = v.split_whitespace().map(str::to_owned).collect();
                match local {
                    "grantTargets" => attrs.targets.extend(set),
                    "grantTools" => attrs.tools.extend(set),
                    "grantDeciders" => attrs.deciders.extend(set),
                    "grantFunctions" => attrs.functions.extend(set),
                    _ => continue,
                }
                any_attr = true;
            }
            _ => {}
        }
    }
    if any_attr {
        add(attrs);
    }
    out
}

/// The effective grant of a child: `parent ∩ declared`. `None` stands for an unrestricted
/// actor (a root actor), so an unrestricted parent passes the declared grant through, and a
/// child that declares nothing inherits its parent's grant.
// @tg: implements:: [[openspec:llm-actors#Capability Grants]]
pub fn effective(parent: Option<&Grant>, declared: Option<&Grant>) -> Option<Grant> {
    match (parent, declared) {
        (Some(p), Some(d)) => Some(p.intersect(d)),
        (Some(p), None) => Some(p.clone()),
        (None, Some(d)) => Some(d.clone()),
        (None, None) => None,
    }
}

/// A send refused by a grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Denied {
    /// The sending actor.
    pub sender: String,
    /// The refused target.
    pub target: String,
    /// The event name.
    pub event: String,
}

/// `data` of the `error.*` event `name` with the `kind` and `message` fields of the
/// predefined error payload type added when missing: `kind` is the name after `error.`,
/// `message` the `reason` or `class` field, or the kind.
pub fn with_error_kind(name: &str, data: Value) -> Value {
    let Some(kind) = name.strip_prefix("error.") else {
        return data;
    };
    let Value::Object(mut fields) = data else {
        return data;
    };
    let has = |fields: &[(String, Value)], k: &str| fields.iter().any(|(n, _)| n == k);
    if !has(&fields, "message") {
        let message = ["reason", "class"]
            .iter()
            .find_map(|k| match fields.iter().find(|(n, _)| n == k) {
                Some((_, Value::String(m))) => Some(m.clone()),
                _ => None,
            })
            .unwrap_or_else(|| kind.to_owned());
        fields.insert(0, ("message".into(), Value::String(message)));
    }
    if !has(&fields, "kind") {
        fields.insert(0, ("kind".into(), Value::String(kind.to_owned())));
    }
    Value::Object(fields)
}

/// The `error.communication` event of a refused send.
pub fn not_granted_event(target: &str, event: &str) -> vertep_core::datamodel::Event {
    vertep_core::datamodel::Event {
        name: vocab::COMMUNICATION_ERROR_EVENT.into(),
        type_: EventType::Platform,
        data: with_error_kind(
            vocab::COMMUNICATION_ERROR_EVENT,
            obj([
                ("reason", s("not-granted")),
                ("target", s(target)),
                ("event", s(event)),
            ]),
        ),
        ..Default::default()
    }
}

/// Wraps an invoked actor on a [`vertep_runtime::Node`] so that every send it makes passes its
/// grant: an ungranted send is not delivered; instead `error.communication` (reason
/// `not-granted`) is sent to the actor itself and `vt.grant.denied` to its parent, which
/// journals the attempt as an input.
pub struct GrantedActor {
    inner: Box<dyn Actor>,
    grant: Grant,
    denied: Arc<Mutex<Vec<Denied>>>,
}

impl std::fmt::Debug for GrantedActor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrantedActor")
            .field("grant", &self.grant)
            .finish_non_exhaustive()
    }
}

fn target_name(t: &OutTarget) -> Option<String> {
    match t {
        OutTarget::Parent => Some(PARENT.to_owned()),
        OutTarget::Address(a) => Some(a.as_str().to_owned()),
        OutTarget::Origin | OutTarget::Itself | OutTarget::Invalid(_) => None,
    }
}

fn filter(
    sender: &Address,
    grant: &Grant,
    denied: &Mutex<Vec<Denied>>,
    out: TurnOutput,
) -> TurnOutput {
    let mut kept = Vec::with_capacity(out.sends.len());
    for o in out.sends {
        match target_name(&o.target) {
            Some(t) if !grant.allows(&t) => {
                if let Ok(mut d) = denied.lock() {
                    d.push(Denied {
                        sender: sender.as_str().to_owned(),
                        target: t.clone(),
                        event: o.event.name.clone(),
                    });
                }
                kept.push(Outgoing {
                    target: OutTarget::Itself,
                    event: not_granted_event(&t, &o.event.name),
                    delay_ms: 0,
                });
                kept.push(Outgoing::new(
                    OutTarget::Parent,
                    vocab::GRANT_DENIED_EVENT,
                    obj([("target", s(&t)), ("event", s(&o.event.name))]),
                ));
            }
            _ => kept.push(o),
        }
    }
    TurnOutput { sends: kept, ..out }
}

impl GrantedActor {
    /// `inner` restricted to `grant`.
    pub fn new(inner: Box<dyn Actor>, grant: Grant) -> Self {
        GrantedActor {
            inner,
            grant,
            denied: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// The refused sends so far.
    pub fn denied(&self) -> Vec<Denied> {
        self.denied.lock().map(|d| d.clone()).unwrap_or_default()
    }

    /// A factory whose actors are `factory`'s, restricted to `grant`; refused sends are also
    /// appended to `log`.
    pub fn factory(
        factory: ActorFactory,
        grant: Grant,
        log: Arc<Mutex<Vec<Denied>>>,
    ) -> ActorFactory {
        Arc::new(
            move |start: &ActorStart| -> Result<Box<dyn Actor>, ActorFailure> {
                let inner = factory(start)?;
                Ok(Box::new(GrantedActor {
                    inner,
                    grant: grant.clone(),
                    denied: log.clone(),
                }))
            },
        )
    }
}

impl Actor for GrantedActor {
    fn address(&self) -> &Address {
        self.inner.address()
    }

    // @tg: implements:: [[openspec:llm-actors#Capability Grants]]
    fn turn(&mut self, input: ActorInput) -> Turn {
        let sender = self.inner.address().clone();
        let grant = self.grant.clone();
        let denied = self.denied.clone();
        match self.inner.turn(input) {
            Turn::Ready(r) => Turn::Ready(r.map(|o| filter(&sender, &grant, &denied, o))),
            Turn::Pending(f) => Turn::Pending(Box::pin(async move {
                f.await.map(|o| filter(&sender, &grant, &denied, o))
            })),
        }
    }
}

/// The data of a refused send, as recorded.
pub fn denied_value(d: &Denied) -> Value {
    obj([
        ("sender", s(&d.sender)),
        ("target", s(&d.target)),
        ("event", s(&d.event)),
    ])
}
