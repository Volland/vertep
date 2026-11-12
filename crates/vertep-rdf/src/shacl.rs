//! A small SHACL Core validator, sufficient for the chart shapes.
//!
//! Supported: targets `sh:targetClass`, `sh:targetNode`, `sh:targetSubjectsOf` and
//! `sh:targetObjectsOf`; every SHACL property path (predicate, inverse, sequence, alternative,
//! zero-or-more, one-or-more, zero-or-one); and the constraint components `sh:class`,
//! `sh:datatype`, `sh:nodeKind`, `sh:minCount`, `sh:maxCount`, `sh:minInclusive`,
//! `sh:maxInclusive`, `sh:minExclusive`, `sh:maxExclusive` (on integers), `sh:minLength`,
//! `sh:maxLength`, `sh:in`, `sh:hasValue`, `sh:equals`, `sh:disjoint`, `sh:lessThan`,
//! `sh:lessThanOrEquals` (on integers), `sh:not`, `sh:and`, `sh:or`, `sh:xone`, `sh:node`,
//! `sh:property`, `sh:qualifiedValueShape` with `sh:qualifiedMinCount` and
//! `sh:qualifiedMaxCount`, `sh:pattern` with `sh:flags` (`i`, `m`, `s`, `x`), `sh:closed` with
//! `sh:ignoredProperties`, and `sh:deactivated`. SHACL-SPARQL is not supported: the chart shapes
//! are SHACL Core only (design D9), which the Lean validator also models.
//!
//! An unsupported constraint parameter in a shapes graph is an error at load time, so a shape
//! is never silently ignored.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt;
use std::rc::Rc;

use oxrdf::{Graph, NamedNode, NamedNodeRef, NamedOrBlankNodeRef, Term, TermRef};

const SH: &str = "http://www.w3.org/ns/shacl#";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

fn sh(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{SH}{local}"))
}

fn rdf(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{RDF}{local}"))
}

/// A SHACL property path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Path {
    /// A predicate IRI.
    Predicate(NamedNode),
    /// `sh:inversePath`.
    Inverse(Box<Path>),
    /// A sequence path (an RDF list).
    Sequence(Vec<Path>),
    /// `sh:alternativePath`.
    Alternative(Vec<Path>),
    /// `sh:zeroOrMorePath`.
    ZeroOrMore(Box<Path>),
    /// `sh:oneOrMorePath`.
    OneOrMore(Box<Path>),
    /// `sh:zeroOrOnePath`.
    ZeroOrOne(Box<Path>),
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn short(n: &NamedNode) -> String {
            match n.as_str().strip_prefix(crate::vocab::VT) {
                Some(l) => format!("vt:{l}"),
                None => n.to_string(),
            }
        }
        match self {
            Path::Predicate(p) => f.write_str(&short(p)),
            Path::Inverse(p) => write!(f, "^{p}"),
            Path::Sequence(ps) => {
                let parts: Vec<String> = ps.iter().map(|p| format!("{p}")).collect();
                write!(f, "({})", parts.join("/"))
            }
            Path::Alternative(ps) => {
                let parts: Vec<String> = ps.iter().map(|p| format!("{p}")).collect();
                write!(f, "({})", parts.join("|"))
            }
            Path::ZeroOrMore(p) => write!(f, "({p})*"),
            Path::OneOrMore(p) => write!(f, "({p})+"),
            Path::ZeroOrOne(p) => write!(f, "({p})?"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeKind {
    Iri,
    BlankNode,
    Literal,
    BlankNodeOrIri,
    BlankNodeOrLiteral,
    IriOrLiteral,
}

#[derive(Clone, Debug)]
enum Constraint {
    Class(NamedNode),
    Datatype(NamedNode),
    NodeKind(NodeKind),
    MinCount(usize),
    MaxCount(usize),
    MinInclusive(i128),
    MaxInclusive(i128),
    MinExclusive(i128),
    MaxExclusive(i128),
    MinLength(usize),
    MaxLength(usize),
    In(Vec<Term>),
    HasValue(Term),
    Equals(NamedNode),
    Disjoint(NamedNode),
    LessThan(NamedNode),
    LessThanOrEquals(NamedNode),
    Not(usize),
    And(Vec<usize>),
    Or(Vec<usize>),
    Xone(Vec<usize>),
    Node(usize),
    Property(usize),
    /// `sh:pattern`: the regular expression (flags applied) and its source text.
    Pattern(regex::Regex, String),
    /// `sh:closed true`: the predicates a value node may have.
    Closed(BTreeSet<NamedNode>),
    Qualified {
        shape: usize,
        min: Option<usize>,
        max: Option<usize>,
    },
}

impl Constraint {
    fn component(&self) -> &'static str {
        match self {
            Constraint::Class(_) => "ClassConstraintComponent",
            Constraint::Datatype(_) => "DatatypeConstraintComponent",
            Constraint::NodeKind(_) => "NodeKindConstraintComponent",
            Constraint::MinCount(_) => "MinCountConstraintComponent",
            Constraint::MaxCount(_) => "MaxCountConstraintComponent",
            Constraint::MinInclusive(_) => "MinInclusiveConstraintComponent",
            Constraint::MaxInclusive(_) => "MaxInclusiveConstraintComponent",
            Constraint::MinExclusive(_) => "MinExclusiveConstraintComponent",
            Constraint::MaxExclusive(_) => "MaxExclusiveConstraintComponent",
            Constraint::MinLength(_) => "MinLengthConstraintComponent",
            Constraint::MaxLength(_) => "MaxLengthConstraintComponent",
            Constraint::In(_) => "InConstraintComponent",
            Constraint::HasValue(_) => "HasValueConstraintComponent",
            Constraint::Equals(_) => "EqualsConstraintComponent",
            Constraint::Disjoint(_) => "DisjointConstraintComponent",
            Constraint::LessThan(_) => "LessThanConstraintComponent",
            Constraint::LessThanOrEquals(_) => "LessThanOrEqualsConstraintComponent",
            Constraint::Not(_) => "NotConstraintComponent",
            Constraint::And(_) => "AndConstraintComponent",
            Constraint::Or(_) => "OrConstraintComponent",
            Constraint::Xone(_) => "XoneConstraintComponent",
            Constraint::Node(_) => "NodeConstraintComponent",
            Constraint::Property(_) => "PropertyConstraintComponent",
            Constraint::Pattern(..) => "PatternConstraintComponent",
            Constraint::Closed(_) => "ClosedConstraintComponent",
            Constraint::Qualified { min: Some(_), .. } => "QualifiedMinCountConstraintComponent",
            Constraint::Qualified { .. } => "QualifiedMaxCountConstraintComponent",
        }
    }
}

#[derive(Clone, Debug)]
enum Target {
    Class(NamedNode),
    Node(Term),
    SubjectsOf(NamedNode),
    ObjectsOf(NamedNode),
}

#[derive(Clone, Debug)]
struct Shape {
    id: Term,
    targets: Vec<Target>,
    path: Option<Path>,
    constraints: Vec<Constraint>,
    deactivated: bool,
    message: Option<String>,
}

/// A shapes graph could not be loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapesError(pub String);

impl fmt::Display for ShapesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid shapes graph: {}", self.0)
    }
}

impl std::error::Error for ShapesError {}

/// A compiled shapes graph.
#[derive(Clone, Debug)]
pub struct Shapes {
    shapes: Vec<Shape>,
}

/// One result of a validation report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationResult {
    /// `sh:focusNode`.
    pub focus_node: Term,
    /// `sh:resultPath`, for results of property shapes.
    pub path: Option<Path>,
    /// `sh:value`, when the constraint is about one value node.
    pub value: Option<Term>,
    /// `sh:sourceShape`.
    pub source_shape: Term,
    /// The local name of `sh:sourceConstraintComponent`, such as `MinCountConstraintComponent`.
    pub component: &'static str,
    /// `sh:resultMessage`: the shape's `sh:message` if any, else a generated description.
    pub message: String,
}

impl fmt::Display for ValidationResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.focus_node)?;
        if let Some(p) = &self.path {
            write!(f, " {p}")?;
        }
        if let Some(v) = &self.value {
            write!(f, " = {v}")?;
        }
        write!(f, ": {} ({})", self.message, self.component)
    }
}

/// A SHACL validation report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValidationReport {
    /// The results; the report conforms exactly when there are none.
    pub results: Vec<ValidationResult>,
}

impl ValidationReport {
    /// `sh:conforms`.
    pub fn conforms(&self) -> bool {
        self.results.is_empty()
    }
}

impl fmt::Display for ValidationReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.conforms() {
            return f.write_str("conforms");
        }
        writeln!(f, "{} violation(s):", self.results.len())?;
        for r in &self.results {
            writeln!(f, "  {r}")?;
        }
        Ok(())
    }
}

struct Loader<'g> {
    g: &'g Graph,
    shapes: Vec<Shape>,
    ids: HashMap<Term, usize>,
}

impl<'g> Loader<'g> {
    fn subject(t: &Term) -> Result<NamedOrBlankNodeRef<'_>, ShapesError> {
        match t {
            Term::NamedNode(n) => Ok(n.as_ref().into()),
            Term::BlankNode(b) => Ok(b.as_ref().into()),
            _ => Err(ShapesError(format!("{t} cannot be a shape"))),
        }
    }

    fn objects(&self, s: &Term, p: &NamedNode) -> Result<Vec<Term>, ShapesError> {
        Ok(self
            .g
            .objects_for_subject_predicate(Self::subject(s)?, p)
            .map(TermRef::into_owned)
            .collect())
    }

    fn object(&self, s: &Term, p: &NamedNode) -> Result<Option<Term>, ShapesError> {
        let mut os = self.objects(s, p)?;
        if os.len() > 1 {
            return Err(ShapesError(format!("{s} has several values for {p}")));
        }
        Ok(os.pop())
    }

    fn list(&self, head: &Term) -> Result<Vec<Term>, ShapesError> {
        let (first, rest, nil) = (rdf("first"), rdf("rest"), rdf("nil"));
        let mut out = Vec::new();
        let mut cur = head.clone();
        while cur != Term::NamedNode(nil.clone()) {
            let item = self
                .object(&cur, &first)?
                .ok_or_else(|| ShapesError(format!("{cur} is not a list")))?;
            out.push(item);
            cur = self
                .object(&cur, &rest)?
                .ok_or_else(|| ShapesError(format!("{cur} is not a list")))?;
            if out.len() > 10_000 {
                return Err(ShapesError("list too long".into()));
            }
        }
        Ok(out)
    }

    fn iri(t: Term, what: &str) -> Result<NamedNode, ShapesError> {
        match t {
            Term::NamedNode(n) => Ok(n),
            other => Err(ShapesError(format!("{what} must be an IRI, got {other}"))),
        }
    }

    fn count(t: Term, what: &str) -> Result<usize, ShapesError> {
        match &t {
            Term::Literal(l) => l
                .value()
                .parse()
                .map_err(|_| ShapesError(format!("{what} must be a non-negative integer"))),
            _ => Err(ShapesError(format!("{what} must be a literal"))),
        }
    }

    fn int(t: Term, what: &str) -> Result<i128, ShapesError> {
        match &t {
            Term::Literal(l) => l
                .value()
                .parse()
                .map_err(|_| ShapesError(format!("{what} must be an integer"))),
            _ => Err(ShapesError(format!("{what} must be a literal"))),
        }
    }

    fn path(&self, t: &Term) -> Result<Path, ShapesError> {
        // An IRI is a predicate path, whatever the shapes graph says about it (SHACL §2.3.1).
        if let Term::NamedNode(n) = t {
            return Ok(Path::Predicate(n.clone()));
        }
        if self.object(t, &rdf("first"))?.is_some() {
            let items = self.list(t)?;
            return Ok(Path::Sequence(
                items
                    .iter()
                    .map(|i| self.path(i))
                    .collect::<Result<_, _>>()?,
            ));
        }
        if let Some(p) = self.object(t, &sh("inversePath"))? {
            return Ok(Path::Inverse(Box::new(self.path(&p)?)));
        }
        if let Some(l) = self.object(t, &sh("alternativePath"))? {
            let items = self.list(&l)?;
            return Ok(Path::Alternative(
                items
                    .iter()
                    .map(|i| self.path(i))
                    .collect::<Result<_, _>>()?,
            ));
        }
        if let Some(p) = self.object(t, &sh("zeroOrMorePath"))? {
            return Ok(Path::ZeroOrMore(Box::new(self.path(&p)?)));
        }
        if let Some(p) = self.object(t, &sh("oneOrMorePath"))? {
            return Ok(Path::OneOrMore(Box::new(self.path(&p)?)));
        }
        if let Some(p) = self.object(t, &sh("zeroOrOnePath"))? {
            return Ok(Path::ZeroOrOne(Box::new(self.path(&p)?)));
        }
        Err(ShapesError(format!("unsupported path {t}")))
    }

    fn shape(&mut self, id: &Term) -> Result<usize, ShapesError> {
        if let Some(&i) = self.ids.get(id) {
            return Ok(i);
        }
        let index = self.shapes.len();
        self.ids.insert(id.clone(), index);
        self.shapes.push(Shape {
            id: id.clone(),
            targets: Vec::new(),
            path: None,
            constraints: Vec::new(),
            deactivated: false,
            message: None,
        });

        let mut targets = Vec::new();
        for c in self.objects(id, &sh("targetClass"))? {
            targets.push(Target::Class(Self::iri(c, "sh:targetClass")?));
        }
        for n in self.objects(id, &sh("targetNode"))? {
            targets.push(Target::Node(n));
        }
        for p in self.objects(id, &sh("targetSubjectsOf"))? {
            targets.push(Target::SubjectsOf(Self::iri(p, "sh:targetSubjectsOf")?));
        }
        for p in self.objects(id, &sh("targetObjectsOf"))? {
            targets.push(Target::ObjectsOf(Self::iri(p, "sh:targetObjectsOf")?));
        }
        let path = match self.object(id, &sh("path"))? {
            Some(p) => Some(self.path(&p)?),
            None => None,
        };
        let deactivated = matches!(
            self.object(id, &sh("deactivated"))?,
            Some(Term::Literal(l)) if l.value() == "true"
        );
        let message = match self.object(id, &sh("message"))? {
            Some(Term::Literal(l)) => Some(l.value().to_owned()),
            _ => None,
        };

        let mut cs = Vec::new();
        let s = Self::subject(id)?.into_owned();
        let preds: BTreeSet<NamedNode> = self
            .g
            .triples_for_subject(&s)
            .map(|t| t.predicate.into_owned())
            .collect();
        for p in preds {
            let Some(local) = p.as_str().strip_prefix(SH) else {
                continue;
            };
            for v in self.objects(id, &p)? {
                let c = match local {
                    "class" => Constraint::Class(Self::iri(v, "sh:class")?),
                    "datatype" => Constraint::Datatype(Self::iri(v, "sh:datatype")?),
                    "nodeKind" => {
                        let n = Self::iri(v, "sh:nodeKind")?;
                        let k = match n.as_str().strip_prefix(SH) {
                            Some("IRI") => NodeKind::Iri,
                            Some("BlankNode") => NodeKind::BlankNode,
                            Some("Literal") => NodeKind::Literal,
                            Some("BlankNodeOrIRI") => NodeKind::BlankNodeOrIri,
                            Some("BlankNodeOrLiteral") => NodeKind::BlankNodeOrLiteral,
                            Some("IRIOrLiteral") => NodeKind::IriOrLiteral,
                            _ => return Err(ShapesError(format!("unknown node kind {n}"))),
                        };
                        Constraint::NodeKind(k)
                    }
                    "minCount" => Constraint::MinCount(Self::count(v, "sh:minCount")?),
                    "maxCount" => Constraint::MaxCount(Self::count(v, "sh:maxCount")?),
                    "minInclusive" => Constraint::MinInclusive(Self::int(v, "sh:minInclusive")?),
                    "maxInclusive" => Constraint::MaxInclusive(Self::int(v, "sh:maxInclusive")?),
                    "minExclusive" => Constraint::MinExclusive(Self::int(v, "sh:minExclusive")?),
                    "maxExclusive" => Constraint::MaxExclusive(Self::int(v, "sh:maxExclusive")?),
                    "minLength" => Constraint::MinLength(Self::count(v, "sh:minLength")?),
                    "maxLength" => Constraint::MaxLength(Self::count(v, "sh:maxLength")?),
                    "in" => Constraint::In(self.list(&v)?),
                    "hasValue" => Constraint::HasValue(v),
                    "equals" => Constraint::Equals(Self::iri(v, "sh:equals")?),
                    "disjoint" => Constraint::Disjoint(Self::iri(v, "sh:disjoint")?),
                    "lessThan" => Constraint::LessThan(Self::iri(v, "sh:lessThan")?),
                    "lessThanOrEquals" => {
                        Constraint::LessThanOrEquals(Self::iri(v, "sh:lessThanOrEquals")?)
                    }
                    "not" => Constraint::Not(self.shape(&v)?),
                    "node" => Constraint::Node(self.shape(&v)?),
                    "pattern" => {
                        let Term::Literal(l) = &v else {
                            return Err(ShapesError("sh:pattern must be a literal".into()));
                        };
                        let flags = match self.object(id, &sh("flags"))? {
                            Some(Term::Literal(f)) => f.value().to_owned(),
                            Some(_) => {
                                return Err(ShapesError("sh:flags must be a literal".into()));
                            }
                            None => String::new(),
                        };
                        let mut b = regex::RegexBuilder::new(l.value());
                        for c in flags.chars() {
                            match c {
                                'i' => b.case_insensitive(true),
                                'm' => b.multi_line(true),
                                's' => b.dot_matches_new_line(true),
                                'x' => b.ignore_whitespace(true),
                                other => {
                                    return Err(ShapesError(format!(
                                        "unsupported sh:flags {other}"
                                    )));
                                }
                            };
                        }
                        let re = b
                            .build()
                            .map_err(|e| ShapesError(format!("bad sh:pattern: {e}")))?;
                        Constraint::Pattern(re, l.value().to_owned())
                    }
                    "closed" => {
                        if !matches!(&v, Term::Literal(l) if l.value() == "true") {
                            continue;
                        }
                        let mut allowed = BTreeSet::new();
                        for p in self.objects(id, &sh("property"))? {
                            if let Some(Term::NamedNode(n)) = self.object(&p, &sh("path"))? {
                                allowed.insert(n);
                            }
                        }
                        if let Some(l) = self.object(id, &sh("ignoredProperties"))? {
                            for i in self.list(&l)? {
                                allowed.insert(Self::iri(i, "sh:ignoredProperties")?);
                            }
                        }
                        Constraint::Closed(allowed)
                    }
                    "property" => Constraint::Property(self.shape(&v)?),
                    "and" | "or" | "xone" => {
                        let members = self
                            .list(&v)?
                            .iter()
                            .map(|m| self.shape(m))
                            .collect::<Result<Vec<_>, _>>()?;
                        match local {
                            "and" => Constraint::And(members),
                            "or" => Constraint::Or(members),
                            _ => Constraint::Xone(members),
                        }
                    }
                    "qualifiedValueShape" => {
                        let shape = self.shape(&v)?;
                        let min = match self.object(id, &sh("qualifiedMinCount"))? {
                            Some(n) => Some(Self::count(n, "sh:qualifiedMinCount")?),
                            None => None,
                        };
                        let max = match self.object(id, &sh("qualifiedMaxCount"))? {
                            Some(n) => Some(Self::count(n, "sh:qualifiedMaxCount")?),
                            None => None,
                        };
                        Constraint::Qualified { shape, min, max }
                    }
                    // Parameters handled above, or descriptive only.
                    "targetClass"
                    | "targetNode"
                    | "targetSubjectsOf"
                    | "targetObjectsOf"
                    | "path"
                    | "deactivated"
                    | "message"
                    | "severity"
                    | "name"
                    | "description"
                    | "order"
                    | "group"
                    | "qualifiedMinCount"
                    | "qualifiedMaxCount"
                    | "qualifiedValueShapesDisjoint"
                    | "flags"
                    | "ignoredProperties" => continue,
                    other => {
                        return Err(ShapesError(format!("unsupported parameter sh:{other}")));
                    }
                };
                cs.push(c);
            }
        }
        let shape = &mut self.shapes[index];
        shape.targets = targets;
        shape.path = path;
        shape.constraints = cs;
        shape.deactivated = deactivated;
        shape.message = message;
        Ok(index)
    }
}

impl Shapes {
    /// Compile every shape of a shapes graph: the instances of `sh:NodeShape` and
    /// `sh:PropertyShape`, every subject with a target, and every shape they reference.
    pub fn from_graph(g: &Graph) -> Result<Self, ShapesError> {
        let mut loader = Loader {
            g,
            shapes: Vec::new(),
            ids: HashMap::new(),
        };
        let rdf_type = rdf("type");
        let mut roots: Vec<Term> = Vec::new();
        for class in ["NodeShape", "PropertyShape"] {
            for s in g.subjects_for_predicate_object(&rdf_type, &sh(class)) {
                roots.push(s.into_owned().into());
            }
        }
        for t in [
            "targetClass",
            "targetNode",
            "targetSubjectsOf",
            "targetObjectsOf",
        ] {
            for tr in g.triples_for_predicate(&sh(t)) {
                roots.push(tr.subject.into_owned().into());
            }
        }
        roots.sort_by_cached_key(|t| t.to_string());
        roots.dedup();
        for r in roots {
            loader.shape(&r)?;
        }
        Ok(Shapes {
            shapes: loader.shapes,
        })
    }

    /// The number of compiled shapes, nested ones included.
    pub fn len(&self) -> usize {
        self.shapes.len()
    }

    /// Whether there are no shapes.
    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    /// The identifiers of the shapes that have targets.
    pub fn targeted_shapes(&self) -> Vec<&Term> {
        self.shapes
            .iter()
            .filter(|s| !s.targets.is_empty())
            .map(|s| &s.id)
            .collect()
    }

    /// Validate a data graph.
    pub fn validate(&self, data: &Graph) -> ValidationReport {
        let v = Validator {
            shapes: self,
            data,
            type_: rdf("type"),
            sub_class_of: NamedNode::new_unchecked(crate::vocab::RDFS_SUB_CLASS_OF),
            class_cache: RefCell::new(HashMap::new()),
            closure_cache: RefCell::new(HashMap::new()),
            in_progress: RefCell::new(HashSet::new()),
        };
        let mut results = Vec::new();
        for (i, shape) in self.shapes.iter().enumerate() {
            if shape.targets.is_empty() || shape.deactivated {
                continue;
            }
            for focus in v.focus_nodes(shape) {
                v.validate_shape(i, &focus, &mut results, 0);
            }
        }
        ValidationReport { results }
    }
}

/// Nesting limit for `sh:node`, `sh:not` and the logical constraints.
const MAX_DEPTH: usize = 64;

/// A closure being memoized: the inner path's address and the start node.
type ClosureKey = (usize, Term);

struct Validator<'a> {
    shapes: &'a Shapes,
    data: &'a Graph,
    type_: NamedNode,
    sub_class_of: NamedNode,
    /// Each class with its subclasses in the data graph.
    class_cache: RefCell<HashMap<NamedNode, Rc<Vec<Term>>>>,
    /// Memoized `sh:oneOrMorePath` results, keyed by the inner path's address and the node.
    closure_cache: RefCell<HashMap<ClosureKey, Rc<Vec<Term>>>>,
    /// Closures being computed, to detect cycles.
    in_progress: RefCell<HashSet<ClosureKey>>,
}

fn as_subject(t: &Term) -> Option<NamedOrBlankNodeRef<'_>> {
    match t {
        Term::NamedNode(n) => Some(n.as_ref().into()),
        Term::BlankNode(b) => Some(b.as_ref().into()),
        _ => None,
    }
}

fn as_integer(t: &Term) -> Option<i128> {
    match t {
        Term::Literal(l) if l.datatype().as_str() == crate::vocab::XSD_INTEGER => {
            l.value().parse().ok()
        }
        _ => None,
    }
}

impl Validator<'_> {
    fn focus_nodes(&self, shape: &Shape) -> Vec<Term> {
        let mut out: Vec<Term> = Vec::new();
        let mut seen = HashSet::new();
        let mut add = |t: Term, out: &mut Vec<Term>| {
            if seen.insert(t.clone()) {
                out.push(t);
            }
        };
        for t in &shape.targets {
            match t {
                Target::Class(c) => {
                    for class in self.subclasses(c).iter() {
                        for s in self.data.subjects_for_predicate_object(&self.type_, class) {
                            add(s.into_owned().into(), &mut out);
                        }
                    }
                }
                Target::Node(n) => add(n.clone(), &mut out),
                Target::SubjectsOf(p) => {
                    for tr in self.data.triples_for_predicate(p) {
                        add(tr.subject.into_owned().into(), &mut out);
                    }
                }
                Target::ObjectsOf(p) => {
                    for tr in self.data.triples_for_predicate(p) {
                        add(tr.object.into_owned(), &mut out);
                    }
                }
            }
        }
        out
    }

    /// The class and all its subclasses in the data graph.
    fn subclasses(&self, c: &NamedNode) -> Rc<Vec<Term>> {
        if let Some(cached) = self.class_cache.borrow().get(c) {
            return Rc::clone(cached);
        }
        let out = Rc::new(self.compute_subclasses(c));
        self.class_cache
            .borrow_mut()
            .insert(c.clone(), Rc::clone(&out));
        out
    }

    fn compute_subclasses(&self, c: &NamedNode) -> Vec<Term> {
        let mut out = vec![Term::NamedNode(c.clone())];
        let mut i = 0;
        while i < out.len() {
            let cur = out[i].clone();
            for s in self
                .data
                .subjects_for_predicate_object(&self.sub_class_of, &cur)
            {
                let s: Term = s.into_owned().into();
                if !out.contains(&s) {
                    out.push(s);
                }
            }
            i += 1;
        }
        out
    }

    fn is_instance(&self, v: &Term, c: &NamedNode) -> bool {
        let Some(s) = as_subject(v) else {
            return false;
        };
        let classes = self.subclasses(c);
        self.data
            .objects_for_subject_predicate(s, &self.type_)
            .any(|t| classes.iter().any(|c| c.as_ref() == t))
    }

    fn objects(&self, t: &Term, p: NamedNodeRef<'_>) -> Vec<Term> {
        match as_subject(t) {
            Some(s) => self
                .data
                .objects_for_subject_predicate(s, p)
                .map(TermRef::into_owned)
                .collect(),
            None => Vec::new(),
        }
    }

    fn subjects(&self, t: &Term, p: NamedNodeRef<'_>) -> Vec<Term> {
        self.data
            .subjects_for_predicate_object(p, t)
            .map(|s: NamedOrBlankNodeRef<'_>| s.into_owned().into())
            .collect()
    }

    fn eval_path(&self, path: &Path, focus: &Term) -> Vec<Term> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for t in self.eval_path_raw(path, focus) {
            if seen.insert(t.clone()) {
                out.push(t);
            }
        }
        out
    }

    fn eval_path_raw(&self, path: &Path, focus: &Term) -> Vec<Term> {
        match path {
            Path::Predicate(p) => self.objects(focus, p.as_ref()),
            Path::Inverse(inner) => match inner.as_ref() {
                Path::Predicate(p) => self.subjects(focus, p.as_ref()),
                other => {
                    // Inverse of a complex path: reverse it structurally.
                    self.eval_path_raw(&invert(other), focus)
                }
            },
            Path::Sequence(steps) => {
                let mut cur = vec![focus.clone()];
                for step in steps {
                    let mut next = Vec::new();
                    let mut seen = HashSet::new();
                    for c in &cur {
                        for n in self.eval_path_raw(step, c) {
                            if seen.insert(n.clone()) {
                                next.push(n);
                            }
                        }
                    }
                    cur = next;
                }
                cur
            }
            Path::Alternative(alts) => alts
                .iter()
                .flat_map(|a| self.eval_path_raw(a, focus))
                .collect(),
            Path::ZeroOrMore(inner) => self.closure(inner, focus, true),
            Path::OneOrMore(inner) => self.closure(inner, focus, false),
            Path::ZeroOrOne(inner) => {
                let mut v = vec![focus.clone()];
                v.extend(self.eval_path_raw(inner, focus));
                v
            }
        }
    }

    fn closure(&self, inner: &Path, focus: &Term, reflexive: bool) -> Vec<Term> {
        if let Some(memo) = self.memo_closure(inner, focus) {
            let mut out = Vec::with_capacity(memo.len() + 1);
            if reflexive {
                out.push(focus.clone());
            }
            out.extend(memo.iter().filter(|t| !reflexive || *t != focus).cloned());
            return out;
        }
        self.bfs_closure(inner, focus, reflexive)
    }

    /// The non-reflexive closure computed recursively and memoized, so that the closures of
    /// all nodes of a tree cost one step per node. `None` when a cycle is met, in which case
    /// the caller falls back to a breadth-first search.
    fn memo_closure(&self, inner: &Path, focus: &Term) -> Option<Rc<Vec<Term>>> {
        let key = (inner as *const Path as usize, focus.clone());
        if let Some(r) = self.closure_cache.borrow().get(&key) {
            return Some(Rc::clone(r));
        }
        if !self.in_progress.borrow_mut().insert(key.clone()) {
            return None;
        }
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut acyclic = true;
        for n in self.eval_path_raw(inner, focus) {
            if seen.insert(n.clone()) {
                out.push(n.clone());
            }
            match self.memo_closure(inner, &n) {
                Some(rest) => {
                    for r in rest.iter() {
                        if seen.insert(r.clone()) {
                            out.push(r.clone());
                        }
                    }
                }
                None => acyclic = false,
            }
        }
        self.in_progress.borrow_mut().remove(&key);
        if !acyclic {
            return None;
        }
        let out = Rc::new(out);
        self.closure_cache.borrow_mut().insert(key, Rc::clone(&out));
        Some(out)
    }

    fn bfs_closure(&self, inner: &Path, focus: &Term, reflexive: bool) -> Vec<Term> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        if reflexive {
            seen.insert(focus.clone());
            out.push(focus.clone());
        }
        let mut queue = VecDeque::from([focus.clone()]);
        while let Some(cur) = queue.pop_front() {
            for n in self.eval_path_raw(inner, &cur) {
                if seen.insert(n.clone()) {
                    out.push(n.clone());
                    queue.push_back(n);
                }
            }
        }
        out
    }

    fn conforms(&self, shape: usize, focus: &Term, depth: usize) -> bool {
        let mut results = Vec::new();
        self.validate_shape(shape, focus, &mut results, depth + 1);
        results.is_empty()
    }

    fn validate_shape(
        &self,
        index: usize,
        focus: &Term,
        results: &mut Vec<ValidationResult>,
        depth: usize,
    ) {
        let shape = &self.shapes.shapes[index];
        if shape.deactivated {
            return;
        }
        let mut own = Vec::new();
        let mut report = |c: &Constraint, value: Option<Term>, detail: String| {
            own.push(ValidationResult {
                focus_node: focus.clone(),
                path: shape.path.clone(),
                value,
                source_shape: shape.id.clone(),
                component: c.component(),
                message: shape.message.clone().unwrap_or(detail),
            });
        };
        if depth > MAX_DEPTH {
            if let Some(c) = shape.constraints.first() {
                report(c, None, "shape nesting too deep".into());
            }
            results.extend(own);
            return;
        }
        let values = match &shape.path {
            Some(p) => self.eval_path(p, focus),
            None => vec![focus.clone()],
        };
        for c in &shape.constraints {
            match c {
                Constraint::MinCount(n) => {
                    if values.len() < *n {
                        report(c, None, format!("fewer than {n} values"));
                    }
                }
                Constraint::MaxCount(n) => {
                    if values.len() > *n {
                        report(c, None, format!("more than {n} values ({})", values.len()));
                    }
                }
                Constraint::Equals(p) => {
                    let other: HashSet<Term> =
                        self.objects(focus, p.as_ref()).into_iter().collect();
                    let mine: HashSet<Term> = values.iter().cloned().collect();
                    for v in mine.difference(&other) {
                        report(c, Some(v.clone()), format!("value not also a value of {p}"));
                    }
                    for v in other.difference(&mine) {
                        report(
                            c,
                            Some(v.clone()),
                            format!("value of {p} missing from path"),
                        );
                    }
                }
                Constraint::Disjoint(p) => {
                    let other: HashSet<Term> =
                        self.objects(focus, p.as_ref()).into_iter().collect();
                    for v in values.iter().filter(|v| other.contains(*v)) {
                        report(c, Some(v.clone()), format!("value is also a value of {p}"));
                    }
                }
                Constraint::LessThan(p) | Constraint::LessThanOrEquals(p) => {
                    let strict = matches!(c, Constraint::LessThan(_));
                    for o in self.objects(focus, p.as_ref()) {
                        for v in &values {
                            let ok = match (as_integer(v), as_integer(&o)) {
                                (Some(a), Some(b)) => {
                                    if strict {
                                        a < b
                                    } else {
                                        a <= b
                                    }
                                }
                                _ => false,
                            };
                            if !ok {
                                report(c, Some(v.clone()), format!("value not less than {o}"));
                            }
                        }
                    }
                }
                Constraint::Qualified { shape, min, max } => {
                    let n = values
                        .iter()
                        .filter(|v| self.conforms(*shape, v, depth))
                        .count();
                    if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
                        report(
                            c,
                            None,
                            format!("{n} values conform to the qualified shape"),
                        );
                    }
                }
                _ => {
                    for v in &values {
                        if let Some(detail) = self.value_violation(c, v, results, depth) {
                            report(c, Some(v.clone()), detail);
                        }
                    }
                }
            }
        }
        results.extend(own);
    }

    /// Check a per-value constraint; `Some(reason)` when `v` violates it. `sh:property`
    /// pushes the nested shape's own results instead.
    fn value_violation(
        &self,
        c: &Constraint,
        v: &Term,
        results: &mut Vec<ValidationResult>,
        depth: usize,
    ) -> Option<String> {
        let fail = |cond: bool, why: &dyn Fn() -> String| if cond { Some(why()) } else { None };
        match c {
            Constraint::Class(cl) => fail(!self.is_instance(v, cl), &|| format!("not a {cl}")),
            Constraint::Datatype(dt) => {
                let ok = match v {
                    Term::Literal(l) => {
                        l.datatype() == dt.as_ref() && well_formed(l.value(), dt.as_str())
                    }
                    _ => false,
                };
                fail(!ok, &|| format!("not a literal of datatype {dt}"))
            }
            Constraint::NodeKind(k) => {
                let ok = matches!(
                    (k, v),
                    (NodeKind::Iri, Term::NamedNode(_))
                        | (NodeKind::BlankNode, Term::BlankNode(_))
                        | (NodeKind::Literal, Term::Literal(_))
                        | (
                            NodeKind::BlankNodeOrIri,
                            Term::NamedNode(_) | Term::BlankNode(_)
                        )
                        | (
                            NodeKind::BlankNodeOrLiteral,
                            Term::BlankNode(_) | Term::Literal(_)
                        )
                        | (
                            NodeKind::IriOrLiteral,
                            Term::NamedNode(_) | Term::Literal(_)
                        )
                );
                fail(!ok, &|| format!("wrong node kind {k:?}"))
            }
            Constraint::MinInclusive(n) => fail(as_integer(v).is_none_or(|x| x < *n), &|| {
                format!("less than {n}")
            }),
            Constraint::MaxInclusive(n) => fail(as_integer(v).is_none_or(|x| x > *n), &|| {
                format!("more than {n}")
            }),
            Constraint::MinExclusive(n) => fail(as_integer(v).is_none_or(|x| x <= *n), &|| {
                format!("not more than {n}")
            }),
            Constraint::MaxExclusive(n) => fail(as_integer(v).is_none_or(|x| x >= *n), &|| {
                format!("not less than {n}")
            }),
            Constraint::MinLength(n) | Constraint::MaxLength(n) => {
                let len = match v {
                    Term::Literal(l) => Some(l.value().chars().count()),
                    Term::NamedNode(n) => Some(n.as_str().chars().count()),
                    _ => None,
                };
                let ok = match (c, len) {
                    (Constraint::MinLength(_), Some(l)) => l >= *n,
                    (Constraint::MaxLength(_), Some(l)) => l <= *n,
                    _ => false,
                };
                fail(!ok, &|| format!("length out of bounds ({n})"))
            }
            Constraint::In(list) => fail(!list.contains(v), &|| "not in the allowed list".into()),
            Constraint::HasValue(x) => fail(v != x, &|| format!("not {x}")),
            Constraint::Not(s) => fail(self.conforms(*s, v, depth), &|| {
                format!("conforms to {}", self.shapes.shapes[*s].id)
            }),
            Constraint::And(ss) => fail(!ss.iter().all(|s| self.conforms(*s, v, depth)), &|| {
                "does not conform to every member of sh:and".into()
            }),
            Constraint::Or(ss) => fail(!ss.iter().any(|s| self.conforms(*s, v, depth)), &|| {
                "conforms to no member of sh:or".into()
            }),
            Constraint::Xone(ss) => {
                let n = ss.iter().filter(|s| self.conforms(**s, v, depth)).count();
                fail(n != 1, &|| format!("conforms to {n} members of sh:xone"))
            }
            Constraint::Node(s) => fail(!self.conforms(*s, v, depth), &|| {
                format!("does not conform to {}", self.shapes.shapes[*s].id)
            }),
            Constraint::Property(s) => {
                self.validate_shape(*s, v, results, depth + 1);
                None
            }
            Constraint::Pattern(re, src) => {
                let text = match v {
                    Term::Literal(l) => Some(l.value()),
                    Term::NamedNode(n) => Some(n.as_str()),
                    _ => None,
                };
                fail(!text.is_some_and(|t| re.is_match(t)), &|| {
                    format!("does not match the pattern {src}")
                })
            }
            Constraint::Closed(allowed) => {
                let extra = as_subject(v).and_then(|s| {
                    self.data
                        .triples_for_subject(s)
                        .map(|t| t.predicate.into_owned())
                        .find(|p| !allowed.contains(p))
                });
                fail(extra.is_some(), &|| match &extra {
                    Some(p) => format!("closed shape does not allow the property {p}"),
                    None => String::new(),
                })
            }
            // Handled over the whole value set.
            Constraint::MinCount(_)
            | Constraint::MaxCount(_)
            | Constraint::Equals(_)
            | Constraint::Disjoint(_)
            | Constraint::LessThan(_)
            | Constraint::LessThanOrEquals(_)
            | Constraint::Qualified { .. } => None,
        }
    }
}

/// The structural inverse of a path, used to evaluate `sh:inversePath` of a complex path.
fn invert(p: &Path) -> Path {
    match p {
        Path::Predicate(_) => Path::Inverse(Box::new(p.clone())),
        Path::Inverse(inner) => (**inner).clone(),
        Path::Sequence(steps) => Path::Sequence(steps.iter().rev().map(invert).collect()),
        Path::Alternative(alts) => Path::Alternative(alts.iter().map(invert).collect()),
        Path::ZeroOrMore(inner) => Path::ZeroOrMore(Box::new(invert(inner))),
        Path::OneOrMore(inner) => Path::OneOrMore(Box::new(invert(inner))),
        Path::ZeroOrOne(inner) => Path::ZeroOrOne(Box::new(invert(inner))),
    }
}

/// Lexical well-formedness for the XSD datatypes the chart shapes use.
fn well_formed(value: &str, datatype: &str) -> bool {
    match datatype {
        crate::vocab::XSD_INTEGER => {
            let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
            !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
        }
        crate::vocab::XSD_BOOLEAN => matches!(value, "true" | "false" | "1" | "0"),
        crate::vocab::XSD_LONG => {
            let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
            !digits.is_empty()
                && digits.bytes().all(|b| b.is_ascii_digit())
                && value.parse::<i64>().is_ok()
        }
        crate::vocab::XSD_DECIMAL => {
            // (\+|-)?([0-9]+(\.[0-9]*)?|\.[0-9]+)
            let unsigned = value.strip_prefix(['+', '-']).unwrap_or(value);
            let (int, frac) = match unsigned.split_once('.') {
                Some((i, f)) => (i, Some(f)),
                None => (unsigned, None),
            };
            let digits = |t: &str| t.bytes().all(|b| b.is_ascii_digit());
            digits(int)
                && frac.is_none_or(digits)
                && (!int.is_empty() || frac.is_some_and(|f| !f.is_empty()))
        }
        _ => true,
    }
}
