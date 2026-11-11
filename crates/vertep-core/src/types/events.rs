//! Event payload types: declared events, descriptor unions and the predefined types of the
//! SCXML system events.

use alloc::string::String;
use alloc::vec::Vec;

use crate::types::decl::Declarations;
use crate::types::ty::{Type, Variant};

/// Whether the SCXML event descriptor `descriptor` matches the event name `event`
/// (W3C SCXML 3.12.1): `*` matches every event, and otherwise the descriptor, without a
/// trailing `.*` or `.`, must equal the event name or be a prefix of it ending at a `.`.
pub fn descriptor_matches(descriptor: &str, event: &str) -> bool {
    let d = descriptor.trim();
    if d == "*" {
        return true;
    }
    let d = d.strip_suffix(".*").unwrap_or(d);
    let d = d.strip_suffix('.').unwrap_or(d);
    if d.is_empty() {
        return false;
    }
    event == d || event.strip_prefix(d).is_some_and(|r| r.starts_with('.'))
}

/// The predefined payload type of every `error.*` event: `{ kind: string, message: string }`,
/// where `kind` is the part after `error.` (`execution`, `communication`, `platform`).
pub fn error_payload_type() -> Type {
    Type::record([("kind", Type::STRING), ("message", Type::STRING)])
}

/// The predefined payload type of a system event name, if it is one: `error.*` has
/// [`error_payload_type`]; `done.state.*` and `done.invoke.*` carry `<donedata>` or the
/// invoked session's data, of type `value`.
// @tg: implements:: [[openspec:type-system#Event Type Declarations]]
pub fn system_event_type(name: &str) -> Option<Type> {
    if name == "error" || name.starts_with("error.") {
        Some(error_payload_type())
    } else if name.starts_with("done.state.") || name.starts_with("done.invoke.") {
        Some(Type::Value)
    } else {
        None
    }
}

/// The event names and payload types a chart declares, resolved for lookups.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventTable {
    /// `(name, payload)` in declaration order, first declaration wins.
    entries: Vec<(String, Option<Type>)>,
}

impl EventTable {
    /// The table of a declaration block. Conflicts are reported by
    /// [`Declarations::validate`]; here the first declaration of a name wins.
    pub fn new(decls: &Declarations) -> Self {
        let mut entries: Vec<(String, Option<Type>)> = Vec::new();
        for e in &decls.events {
            if !entries.iter().any(|(n, _)| *n == e.name) {
                entries.push((e.name.clone(), e.payload.clone()));
            }
        }
        EventTable { entries }
    }

    /// Whether any event is declared.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The declared events in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Option<&Type>)> {
        self.entries.iter().map(|(n, t)| (n.as_str(), t.as_ref()))
    }

    /// The payload type of the event named `name`:
    /// `Some(Some(t))` for a declared payload type or a system event, `Some(None)` for a
    /// declared event without payload, and `None` for an undeclared event, whose payload is
    /// the dynamic `value` ([[openspec:type-system#Opt-In Chart Typing]]).
    pub fn payload_of(&self, name: &str) -> Option<Option<Type>> {
        if let Some((_, t)) = self.entries.iter().find(|(n, _)| n == name) {
            return Some(t.clone());
        }
        system_event_type(name).map(Some)
    }

    /// The payload type a transition with this event descriptor sees: the system event type
    /// for `error` and `done` descriptors; the payload of the one declared event the
    /// descriptor names exactly; the tagged union, by event name, of every declared event the
    /// descriptor matches when it matches several (so a guard must discriminate on the event
    /// name before reading variant-specific fields); and `value` when it matches no declared
    /// event or is `*`. `None` means the matched event has no payload.
    // @tg: implements:: [[openspec:type-system#Event Type Declarations]]
    pub fn payload_for_descriptor(&self, descriptor: &str) -> Option<Type> {
        let d = descriptor.trim();
        if d == "*" {
            return Some(Type::Value);
        }
        let base = d.strip_suffix(".*").unwrap_or(d);
        let base = base.strip_suffix('.').unwrap_or(base);
        // A chart that declares a system event by its exact name (an agent chart declaring the
        // richer payload of `error.decide.invalid`, say) sees its declaration, as `payload_of`.
        if let Some((_, t)) = self.entries.iter().find(|(n, _)| n == base)
            && system_event_type(base).is_some()
        {
            return t.clone();
        }
        if let Some(t) = system_event_type(base).or_else(|| {
            (base == "done" || base == "done.state" || base == "done.invoke").then_some(Type::Value)
        }) {
            return Some(t);
        }
        let matched: Vec<&(String, Option<Type>)> = self
            .entries
            .iter()
            .filter(|(n, _)| descriptor_matches(d, n))
            .collect();
        match matched.as_slice() {
            [] => Some(Type::Value),
            [(n, t)] if n == base => t.clone(),
            many => Some(Type::Union(
                many.iter()
                    .map(|(n, t)| Variant::new(n.clone(), t.clone()))
                    .collect(),
            )),
        }
    }

    /// The payload type seen by a transition with several descriptors: the union of what
    /// each sees, or the single descriptor's type.
    pub fn payload_for_descriptors(&self, descriptors: &[String]) -> Option<Type> {
        match descriptors {
            [] => Some(Type::Value),
            [d] => self.payload_for_descriptor(d),
            _ => {
                let mut variants: Vec<Variant> = Vec::new();
                for d in descriptors {
                    let base = d.trim().trim_end_matches(".*");
                    if self.entries.iter().any(|(n, _)| n == base)
                        || self.entries.iter().all(|(n, _)| !descriptor_matches(d, n))
                    {
                        // Exact or unmatched descriptors contribute one variant.
                        if !variants.iter().any(|v| v.tag == base) {
                            variants.push(Variant::new(base, self.payload_for_descriptor(d)));
                        }
                        continue;
                    }
                    if let Some(Type::Union(vs)) = self.payload_for_descriptor(d) {
                        for v in vs {
                            if !variants.iter().any(|w| w.tag == v.tag) {
                                variants.push(v);
                            }
                        }
                    }
                }
                Some(Type::Union(variants))
            }
        }
    }
}
