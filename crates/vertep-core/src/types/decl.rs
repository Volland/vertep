//! Named type and event declarations and their validation.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use crate::ir::Span;
use crate::types::syntax::{is_ident, is_reserved, write_declarations};
use crate::types::ty::Type;

/// A named type declaration: `type Order = { id: string };`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TypeDecl {
    /// The type name, an identifier that is not reserved.
    pub name: String,
    /// The definition.
    pub ty: Type,
    /// Source location.
    pub span: Span,
}

/// An event declaration: `event pay: Payment;` or `event start;` (no payload).
// @tg: implements:: [[openspec:type-system#Event Type Declarations]]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EventDecl {
    /// The event name.
    pub name: String,
    /// The payload type; `None` when the event carries no data.
    pub payload: Option<Type>,
    /// Source location.
    pub span: Span,
}

/// A block of named type and event declarations, in declaration order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Declarations {
    /// Named types.
    pub types: Vec<TypeDecl>,
    /// Typed events.
    pub events: Vec<EventDecl>,
}

impl fmt::Display for Declarations {
    /// The canonical textual block; [`crate::types::parse_declarations`] reads it back.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_declarations(self, f)
    }
}

/// What is wrong with a declaration block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclErrorKind {
    /// Two types share a name.
    DuplicateType(String),
    /// A type name is reserved or not an identifier.
    BadTypeName(String),
    /// A named type is referenced but not declared.
    UnknownType {
        /// The unknown name.
        name: String,
        /// The closest declared name, if one is close.
        suggestion: Option<String>,
    },
    /// A record declares a field twice.
    DuplicateField(String),
    /// A union declares a tag twice.
    DuplicateTag(String),
    /// A union has no variants.
    EmptyUnion,
    /// Named types refer to each other without a list or optional in between: the types
    /// would be infinitely large. The cycle is listed from its first declared member back to it.
    UnguardedCycle(Vec<String>),
    /// One event is declared with two different payload types.
    ConflictingEvent {
        /// The event name.
        name: String,
        /// The first payload type, printed (`none` when absent).
        first: String,
        /// The second payload type, printed.
        second: String,
        /// Location of the first declaration.
        first_span: Span,
    },
}

/// A declaration error with the location of the offending declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclError {
    /// What is wrong.
    pub kind: DeclErrorKind,
    /// The declaration it was found in (type or event name).
    pub decl: String,
    /// Its source location.
    pub span: Span,
}

impl fmt::Display for DeclError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            DeclErrorKind::DuplicateType(n) => write!(f, "type `{n}` is declared more than once"),
            DeclErrorKind::BadTypeName(n) => write!(
                f,
                "`{n}` cannot name a type: names are identifiers other than the reserved words"
            ),
            DeclErrorKind::UnknownType { name, suggestion } => {
                write!(f, "in `{}`: unknown type `{name}`", self.decl)?;
                if let Some(s) = suggestion {
                    write!(f, "; did you mean `{s}`?")?;
                }
                Ok(())
            }
            DeclErrorKind::DuplicateField(n) => {
                write!(
                    f,
                    "in `{}`: field `{n}` is declared more than once",
                    self.decl
                )
            }
            DeclErrorKind::DuplicateTag(n) => {
                write!(
                    f,
                    "in `{}`: variant `{n}` is declared more than once",
                    self.decl
                )
            }
            DeclErrorKind::EmptyUnion => write!(f, "in `{}`: a union needs a variant", self.decl),
            DeclErrorKind::UnguardedCycle(c) => write!(
                f,
                "type `{}` is infinitely large: cycle {} does not pass through a list or optional",
                self.decl,
                c.join(" -> ")
            ),
            DeclErrorKind::ConflictingEvent {
                name,
                first,
                second,
                ..
            } => write!(
                f,
                "event `{name}` is declared with two payload types: {first} and {second}"
            ),
        }
    }
}

/// Edit distance between two names, for suggestions.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = Vec::with_capacity(b.len() + 1);
        cur.push(i + 1);
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != cb);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The candidate closest to `name` when it is close enough to be a likely typo (distance at
/// most a third of the name's length, and at least 1).
pub fn suggest<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let limit = (name.chars().count() / 3).max(1);
    candidates
        .into_iter()
        .map(|c| (edit_distance(name, c), c))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c.to_string())
}

impl Declarations {
    /// The definition of the named type `name`.
    pub fn lookup(&self, name: &str) -> Option<&Type> {
        self.types.iter().find(|t| t.name == name).map(|t| &t.ty)
    }

    /// The payload declarations of event `name`.
    pub fn event(&self, name: &str) -> Option<&EventDecl> {
        self.events.iter().find(|e| e.name == name)
    }

    /// Whether the block declares nothing.
    pub fn is_empty(&self) -> bool {
        self.types.is_empty() && self.events.is_empty()
    }

    /// An unknown-type error for `name`, with a suggestion from the declared names.
    pub fn unknown_type(&self, name: &str) -> DeclErrorKind {
        DeclErrorKind::UnknownType {
            name: name.to_string(),
            suggestion: suggest(name, self.types.iter().map(|t| t.name.as_str())),
        }
    }

    /// Validate a type expression used outside the block (a `vt:type` annotation): every
    /// referenced name must be declared and records and unions well formed.
    pub fn validate_type(&self, ty: &Type, decl: &str, span: Span) -> Vec<DeclError> {
        let mut out = Vec::new();
        self.structure(ty, decl, span, &mut out);
        for r in ty.references() {
            if self.lookup(r).is_none() {
                out.push(DeclError {
                    kind: self.unknown_type(r),
                    decl: decl.to_string(),
                    span,
                });
            }
        }
        out
    }

    fn structure(&self, ty: &Type, decl: &str, span: Span, out: &mut Vec<DeclError>) {
        let mut push = |kind| {
            out.push(DeclError {
                kind,
                decl: decl.to_string(),
                span,
            })
        };
        match ty {
            Type::Record(fields) => {
                for (i, f) in fields.iter().enumerate() {
                    if fields[..i].iter().any(|g| g.name == f.name) {
                        push(DeclErrorKind::DuplicateField(f.name.clone()));
                    }
                }
                for f in fields {
                    self.structure(&f.ty, decl, span, out);
                }
            }
            Type::Union(vs) => {
                if vs.is_empty() {
                    push(DeclErrorKind::EmptyUnion);
                }
                for (i, v) in vs.iter().enumerate() {
                    if vs[..i].iter().any(|w| w.tag == v.tag) {
                        push(DeclErrorKind::DuplicateTag(v.tag.clone()));
                    }
                }
                for p in vs.iter().filter_map(|v| v.payload.as_ref()) {
                    self.structure(p, decl, span, out);
                }
            }
            Type::List(t) | Type::Optional(t) => self.structure(t, decl, span, out),
            Type::Primitive(_) | Type::Value | Type::Named(_) => {}
        }
    }

    /// Validate the block and report every error: duplicate or reserved type names, unknown
    /// references (with the closest declared name), duplicate fields and tags, empty unions,
    /// cycles not guarded by a list or optional, and events declared with two payload types.
    // @tg: implements:: [[openspec:type-system#Recursive and Cyclic Type Definitions]]
    // @tg: implements:: [[openspec:type-system#Event Type Declarations]]
    pub fn validate(&self) -> Vec<DeclError> {
        let mut out = Vec::new();
        for (i, t) in self.types.iter().enumerate() {
            if !is_ident(&t.name) || is_reserved(&t.name) {
                out.push(DeclError {
                    kind: DeclErrorKind::BadTypeName(t.name.clone()),
                    decl: t.name.clone(),
                    span: t.span,
                });
            }
            if self.types[..i].iter().any(|u| u.name == t.name) {
                out.push(DeclError {
                    kind: DeclErrorKind::DuplicateType(t.name.clone()),
                    decl: t.name.clone(),
                    span: t.span,
                });
            }
            out.extend(self.validate_type(&t.ty, &t.name, t.span));
        }
        out.extend(self.cycles());
        for (i, e) in self.events.iter().enumerate() {
            if let Some(p) = &e.payload {
                out.extend(self.validate_type(p, &e.name, e.span));
            }
            if let Some(first) = self.events[..i].iter().find(|f| f.name == e.name)
                && first.payload != e.payload
                && !self.events[..i]
                    .iter()
                    .filter(|f| f.name == e.name)
                    .any(|f| f.payload != first.payload)
            {
                let show = |p: &Option<Type>| match p {
                    Some(t) => t.to_string(),
                    None => "none".to_string(),
                };
                out.push(DeclError {
                    kind: DeclErrorKind::ConflictingEvent {
                        name: e.name.clone(),
                        first: show(&first.payload),
                        second: show(&e.payload),
                        first_span: first.span,
                    },
                    decl: e.name.clone(),
                    span: e.span,
                });
            }
        }
        out
    }

    /// Names reachable from `ty` without passing through a list or optional.
    fn unguarded<'t>(ty: &'t Type, out: &mut Vec<&'t str>) {
        match ty {
            Type::Named(n) => {
                if !out.contains(&n.as_str()) {
                    out.push(n);
                }
            }
            Type::Record(fields) => fields.iter().for_each(|f| Self::unguarded(&f.ty, out)),
            Type::Union(vs) => vs
                .iter()
                .filter_map(|v| v.payload.as_ref())
                .for_each(|t| Self::unguarded(t, out)),
            Type::List(_) | Type::Optional(_) | Type::Primitive(_) | Type::Value => {}
        }
    }

    /// One error per unguarded cycle, reported on the cycle's first declared member.
    fn cycles(&self) -> Vec<DeclError> {
        let index = |n: &str| self.types.iter().position(|t| t.name == n);
        let edges: Vec<Vec<usize>> = self
            .types
            .iter()
            .map(|t| {
                let mut names = Vec::new();
                Self::unguarded(&t.ty, &mut names);
                names.into_iter().filter_map(index).collect()
            })
            .collect();
        let mut out = Vec::new();
        for start in 0..self.types.len() {
            // Breadth-first search for the shortest path back to `start`.
            let mut prev: Vec<Option<usize>> = alloc::vec![None; self.types.len()];
            let mut queue = alloc::collections::VecDeque::from([start]);
            let mut seen = alloc::vec![false; self.types.len()];
            let mut found = None;
            'bfs: while let Some(n) = queue.pop_front() {
                for &m in &edges[n] {
                    if m == start {
                        found = Some(n);
                        break 'bfs;
                    }
                    if !seen[m] {
                        seen[m] = true;
                        prev[m] = Some(n);
                        queue.push_back(m);
                    }
                }
            }
            let Some(last) = found else { continue };
            let mut path = Vec::from([last]);
            let mut cur = last;
            while cur != start {
                let Some(p) = prev[cur] else { break };
                path.push(p);
                cur = p;
            }
            path.reverse();
            // Report each cycle once, on its member declared first.
            if path.iter().any(|&m| m < start) {
                continue;
            }
            let mut names: Vec<String> = path.iter().map(|&m| self.types[m].name.clone()).collect();
            names.push(self.types[start].name.clone());
            out.push(DeclError {
                kind: DeclErrorKind::UnguardedCycle(names),
                decl: self.types[start].name.clone(),
                span: self.types[start].span,
            });
        }
        out
    }

    /// The textual block, as [`core::fmt::Display`] prints it.
    pub fn to_text(&self) -> String {
        format!("{self}")
    }
}
