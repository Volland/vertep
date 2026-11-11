//! Reconstruction of a chart from its RDF projection.
//!
//! RDF is open-world, so a chart is read back only from a graph that the chart shapes have
//! closed: [`reconstruct`] validates first and returns the validation report as the error when
//! the graph does not conform. It then decodes the chart from the `vt:Chart` node down,
//! following containment and `vt:order` only, so any triple outside the projection (an RDF 1.2
//! annotation, a comment, a certificate) is ignored. Finally it re-projects the decoded chart
//! under the graph's base and requires every projected triple to be in the graph, which closes
//! the facts SHACL Core cannot state (see `shapes/vt-chart-shapes.ttl`).

use core::fmt;

use oxrdf::{Graph, LiteralRef, NamedNode, NamedNodeRef, NamedOrBlankNodeRef, TermRef};
use vertep_core::ir::{
    Binding, Block, Chart, Content, Data, DoneData, Exec, ExecKind, Extension, ExtensionValue,
    HistoryType, IfBranch, Initial, Invoke, Param, Script, Send, Span, State, StateElement,
    Transition, TransitionType,
};

use crate::iri::base_of_chart_node;
use crate::project::project_with_base;
use crate::shacl::ValidationReport;
use crate::shapes;
use crate::vocab::{self as vt, expr_datatype};

/// A chart read back from a graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reconstructed {
    /// The chart.
    pub chart: Chart,
    /// The base its node IRIs were minted under: its version IRI for a published graph.
    pub base: String,
}

/// A conforming graph that still does not decode to a chart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError {
    /// The node being decoded.
    pub node: String,
    /// The property involved, if any.
    pub property: Option<String>,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}>", self.node)?;
        if let Some(p) = &self.property {
            write!(f, " <{p}>")?;
        }
        write!(f, ": {}", self.message)
    }
}

/// Reconstruction failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReconstructError {
    /// The graph does not conform to the chart shapes; the report names the offending nodes.
    Invalid(ValidationReport),
    /// The graph conforms but is not the projection of a chart.
    Decode(DecodeError),
}

impl fmt::Display for ReconstructError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReconstructError::Invalid(r) => {
                write!(f, "graph does not conform to the chart shapes: {r}")
            }
            ReconstructError::Decode(e) => write!(f, "graph is not a chart projection: {e}"),
        }
    }
}

impl std::error::Error for ReconstructError {}

impl From<DecodeError> for ReconstructError {
    fn from(e: DecodeError) -> Self {
        ReconstructError::Decode(e)
    }
}

type Result<T> = core::result::Result<T, DecodeError>;

/// Validate a graph against the chart shapes and, when it conforms, rebuild the chart.
// @tg: implements:: [[openspec:chart-rdf#RDF To IR Reconstruction]]
// @tg: implements:: [[openspec:chart-rdf#Annotations Outside The Projection]]
// @tg: implements:: [[openspec:chart-rdf#Round Trip Identity]]
pub fn reconstruct(graph: &Graph) -> core::result::Result<Reconstructed, ReconstructError> {
    let report = shapes::validate(graph);
    if !report.conforms() {
        return Err(ReconstructError::Invalid(report));
    }
    let rdf_type = NamedNodeRef::new_unchecked(vt::RDF_TYPE);
    let charts: Vec<NamedOrBlankNodeRef<'_>> = graph
        .subjects_for_predicate_object(rdf_type, vt::CHART)
        .collect();
    let chart_node = match charts.as_slice() {
        [NamedOrBlankNodeRef::NamedNode(n)] => n.into_owned(),
        _ => {
            return Err(err(
                vt::CHART.as_str(),
                None,
                format!("expected exactly one vt:Chart IRI, found {}", charts.len()),
            )
            .into());
        }
    };
    let base = base_of_chart_node(chart_node.as_str())
        .ok_or_else(|| {
            err(
                chart_node.as_str(),
                None,
                "chart node IRI is not <base>#chart".into(),
            )
        })?
        .to_owned();
    let datamodel = Decoder {
        g: graph,
        expr_dt: NamedNode::new_unchecked(String::new()),
    }
    .string(&chart_node, vt::DATAMODEL)?;
    let d = Decoder {
        g: graph,
        expr_dt: NamedNode::new_unchecked(expr_datatype(&datamodel)),
    };
    let chart = d.chart(&chart_node)?;

    // Closed-world check: the graph contains the whole projection of what was decoded.
    let projection = project_with_base(&chart, &base);
    if let Some(missing) = projection.iter().find(|t| !graph.contains(*t)) {
        return Err(err(
            &missing.subject.to_string(),
            Some(missing.predicate.as_str()),
            format!("graph lacks the projected triple {missing}"),
        )
        .into());
    }
    Ok(Reconstructed { chart, base })
}

fn err(node: &str, property: Option<&str>, message: String) -> DecodeError {
    DecodeError {
        node: node.to_owned(),
        property: property.map(str::to_owned),
        message,
    }
}

struct Decoder<'g> {
    g: &'g Graph,
    expr_dt: NamedNode,
}

impl<'g> Decoder<'g> {
    fn objects(&self, s: &NamedNode, p: NamedNodeRef<'_>) -> Vec<TermRef<'g>> {
        self.g.objects_for_subject_predicate(s, p).collect()
    }

    fn nodes(&self, s: &NamedNode, p: NamedNodeRef<'_>) -> Result<Vec<NamedNode>> {
        self.objects(s, p)
            .into_iter()
            .map(|o| match o {
                TermRef::NamedNode(n) => Ok(n.into_owned()),
                other => Err(err(
                    s.as_str(),
                    Some(p.as_str()),
                    format!("{other} is not an IRI"),
                )),
            })
            .collect()
    }

    fn opt_node(&self, s: &NamedNode, p: NamedNodeRef<'_>) -> Result<Option<NamedNode>> {
        let mut ns = self.nodes(s, p)?;
        if ns.len() > 1 {
            return Err(err(
                s.as_str(),
                Some(p.as_str()),
                "more than one value".into(),
            ));
        }
        Ok(ns.pop())
    }

    fn one_node(&self, s: &NamedNode, p: NamedNodeRef<'_>) -> Result<NamedNode> {
        self.opt_node(s, p)?
            .ok_or_else(|| err(s.as_str(), Some(p.as_str()), "missing value".into()))
    }

    /// The members of an ordered collection, sorted by `vt:order`, which must be `0..n`.
    fn ordered(&self, s: &NamedNode, p: NamedNodeRef<'_>) -> Result<Vec<NamedNode>> {
        let mut members: Vec<(u64, NamedNode)> = self
            .nodes(s, p)?
            .into_iter()
            .map(|n| Ok((self.integer(&n, vt::ORDER)?, n)))
            .collect::<Result<_>>()?;
        members.sort();
        for (i, (o, n)) in members.iter().enumerate() {
            if *o != i as u64 {
                return Err(err(
                    n.as_str(),
                    Some(vt::ORDER.as_str()),
                    format!("orders of {p} are not 0..{}", members.len()),
                ));
            }
        }
        Ok(members.into_iter().map(|(_, n)| n).collect())
    }

    fn literal(&self, s: &NamedNode, p: NamedNodeRef<'_>) -> Result<Option<LiteralRef<'g>>> {
        let os = self.objects(s, p);
        match os.as_slice() {
            [] => Ok(None),
            [TermRef::Literal(l)] => Ok(Some(*l)),
            [other] => Err(err(
                s.as_str(),
                Some(p.as_str()),
                format!("{other} is not a literal"),
            )),
            _ => Err(err(
                s.as_str(),
                Some(p.as_str()),
                "more than one value".into(),
            )),
        }
    }

    fn typed(&self, s: &NamedNode, p: NamedNodeRef<'_>, datatype: &str) -> Result<Option<&'g str>> {
        match self.literal(s, p)? {
            None => Ok(None),
            Some(l) if l.language().is_none() && l.datatype().as_str() == datatype => {
                Ok(Some(l.value()))
            }
            Some(l) => Err(err(
                s.as_str(),
                Some(p.as_str()),
                format!("{l} is not a literal of datatype <{datatype}>"),
            )),
        }
    }

    fn required<T>(&self, s: &NamedNode, p: NamedNodeRef<'_>, v: Option<T>) -> Result<T> {
        v.ok_or_else(|| err(s.as_str(), Some(p.as_str()), "missing value".into()))
    }

    fn opt_string(&self, s: &NamedNode, p: NamedNodeRef<'_>) -> Result<Option<String>> {
        Ok(self.typed(s, p, vt::XSD_STRING)?.map(str::to_owned))
    }

    fn string(&self, s: &NamedNode, p: NamedNodeRef<'_>) -> Result<String> {
        let v = self.opt_string(s, p)?;
        self.required(s, p, v)
    }

    fn opt_expr(&self, s: &NamedNode, p: NamedNodeRef<'_>) -> Result<Option<String>> {
        Ok(self.typed(s, p, self.expr_dt.as_str())?.map(str::to_owned))
    }

    fn expr(&self, s: &NamedNode, p: NamedNodeRef<'_>) -> Result<String> {
        let v = self.opt_expr(s, p)?;
        self.required(s, p, v)
    }

    fn boolean(&self, s: &NamedNode, p: NamedNodeRef<'_>) -> Result<bool> {
        let v = self.typed(s, p, vt::XSD_BOOLEAN)?;
        match self.required(s, p, v)? {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            other => Err(err(
                s.as_str(),
                Some(p.as_str()),
                format!("{other} is not a boolean"),
            )),
        }
    }

    fn integer(&self, s: &NamedNode, p: NamedNodeRef<'_>) -> Result<u64> {
        let v = self.typed(s, p, vt::XSD_INTEGER)?;
        let v = self.required(s, p, v)?;
        v.strip_prefix('+').unwrap_or(v).parse().map_err(|_| {
            err(
                s.as_str(),
                Some(p.as_str()),
                format!("{v} is not a valid order"),
            )
        })
    }

    fn has_type(&self, s: &NamedNode, class: NamedNodeRef<'_>) -> bool {
        self.g.contains(oxrdf::TripleRef::new(
            s,
            NamedNodeRef::new_unchecked(vt::RDF_TYPE),
            class,
        ))
    }

    fn chart(&self, c: &NamedNode) -> Result<Chart> {
        let binding = match self.string(c, vt::BINDING)?.as_str() {
            "early" => Binding::Early,
            "late" => Binding::Late,
            other => {
                return Err(err(
                    c.as_str(),
                    Some(vt::BINDING.as_str()),
                    format!("binding {other}"),
                ));
            }
        };
        Ok(Chart {
            name: self.opt_string(c, vt::NAME)?,
            datamodel: self.string(c, vt::DATAMODEL)?,
            datamodel_explicit: self.boolean(c, vt::DATAMODEL_EXPLICIT)?,
            binding,
            binding_explicit: self.boolean(c, vt::BINDING_EXPLICIT)?,
            initial: self.target_ids(c, vt::INITIAL_REF)?,
            data: self.map(c, vt::DATA, Self::data_item)?,
            script: self
                .opt_node(c, vt::SCRIPT_P)?
                .map(|s| self.script(&s))
                .transpose()?,
            states: self.map(c, vt::CHILD, Self::state)?,
            extensions: self.extensions(c)?,
            span: Span::NONE,
        })
    }

    fn map<T>(
        &self,
        s: &NamedNode,
        p: NamedNodeRef<'_>,
        f: impl Fn(&Self, &NamedNode) -> Result<T>,
    ) -> Result<Vec<T>> {
        self.ordered(s, p)?.iter().map(|n| f(self, n)).collect()
    }

    fn target_ids(&self, owner: &NamedNode, p: NamedNodeRef<'_>) -> Result<Vec<String>> {
        self.map(owner, p, |d, r| {
            let state = d.one_node(r, vt::STATE_P)?;
            d.string(&state, vt::ID)
        })
    }

    fn tokens(&self, owner: &NamedNode, p: NamedNodeRef<'_>) -> Result<Vec<String>> {
        self.map(owner, p, |d, t| d.string(t, vt::VALUE))
    }

    fn state(&self, n: &NamedNode) -> Result<State> {
        let element = if self.has_type(n, vt::HISTORY) {
            let h = match self.string(n, vt::HISTORY_TYPE)?.as_str() {
                "shallow" => HistoryType::Shallow,
                "deep" => HistoryType::Deep,
                other => {
                    return Err(err(
                        n.as_str(),
                        Some(vt::HISTORY_TYPE.as_str()),
                        format!("history type {other}"),
                    ));
                }
            };
            StateElement::History(h)
        } else if self.has_type(n, vt::PARALLEL) {
            StateElement::Parallel
        } else if self.has_type(n, vt::FINAL) {
            StateElement::Final
        } else if self.has_type(n, vt::STATE) {
            StateElement::State
        } else {
            return Err(err(
                n.as_str(),
                None,
                "state node without an element class".into(),
            ));
        };
        let initial_element = match self.opt_node(n, vt::INITIAL_ELEMENT)? {
            Some(i) => Some(Initial {
                transition: self.transition(&self.one_node(&i, vt::INITIAL_TRANSITION)?)?,
                extensions: self.extensions(&i)?,
                span: Span::NONE,
            }),
            None => None,
        };
        Ok(State {
            id: self.string(n, vt::ID)?,
            id_generated: self.boolean(n, vt::ID_GENERATED)?,
            element,
            initial: self.target_ids(n, vt::INITIAL_REF)?,
            initial_element,
            children: self.map(n, vt::CHILD, Self::state)?,
            transitions: self.map(n, vt::TRANSITION_P, Self::transition)?,
            onentry: self.map(n, vt::ONENTRY, |d, h| d.block(h, vt::ACTION))?,
            onexit: self.map(n, vt::ONEXIT, |d, h| d.block(h, vt::ACTION))?,
            data: self.map(n, vt::DATA, Self::data_item)?,
            invokes: self.map(n, vt::INVOKE_P, Self::invoke)?,
            donedata: self
                .opt_node(n, vt::DONEDATA)?
                .map(|d| self.donedata(&d))
                .transpose()?,
            extensions: self.extensions(n)?,
            span: Span::NONE,
        })
    }

    fn transition(&self, n: &NamedNode) -> Result<Transition> {
        let kind = match self.string(n, vt::TRANSITION_TYPE)?.as_str() {
            "external" => TransitionType::External,
            "internal" => TransitionType::Internal,
            other => {
                return Err(err(
                    n.as_str(),
                    Some(vt::TRANSITION_TYPE.as_str()),
                    format!("transition type {other}"),
                ));
            }
        };
        Ok(Transition {
            events: self.tokens(n, vt::EVENT)?,
            cond: self.opt_expr(n, vt::COND)?,
            targets: self.target_ids(n, vt::TARGET)?,
            kind,
            kind_explicit: self.boolean(n, vt::TRANSITION_TYPE_EXPLICIT)?,
            body: self.block(n, vt::ACTION)?,
            extensions: self.extensions(n)?,
            span: Span::NONE,
        })
    }

    fn block(&self, owner: &NamedNode, p: NamedNodeRef<'_>) -> Result<Block> {
        self.map(owner, p, Self::exec)
    }

    fn exec(&self, n: &NamedNode) -> Result<Exec> {
        let kind = if self.has_type(n, vt::RAISE) {
            ExecKind::Raise {
                event: self.string(n, vt::EVENT_NAME)?,
            }
        } else if self.has_type(n, vt::SEND) {
            ExecKind::Send(Box::new(Send {
                event: self.opt_string(n, vt::EVENT_NAME)?,
                eventexpr: self.opt_expr(n, vt::EVENTEXPR)?,
                target: self.opt_string(n, vt::SEND_TARGET)?,
                targetexpr: self.opt_expr(n, vt::TARGETEXPR)?,
                type_: self.opt_string(n, vt::TYPE)?,
                typeexpr: self.opt_expr(n, vt::TYPEEXPR)?,
                id: self.opt_string(n, vt::SEND_ID)?,
                idlocation: self.opt_expr(n, vt::IDLOCATION)?,
                delay: self.opt_string(n, vt::DELAY)?,
                delayexpr: self.opt_expr(n, vt::DELAYEXPR)?,
                namelist: self.tokens(n, vt::NAMELIST)?,
                params: self.params(n)?,
                content: self.content(n)?,
            }))
        } else if self.has_type(n, vt::CANCEL) {
            ExecKind::Cancel {
                sendid: self.opt_string(n, vt::SEND_ID)?,
                sendidexpr: self.opt_expr(n, vt::SENDIDEXPR)?,
            }
        } else if self.has_type(n, vt::LOG) {
            ExecKind::Log {
                label: self.opt_string(n, vt::LABEL)?,
                expr: self.opt_expr(n, vt::EXPR)?,
            }
        } else if self.has_type(n, vt::ASSIGN) {
            ExecKind::Assign {
                location: self.expr(n, vt::LOCATION)?,
                expr: self.opt_expr(n, vt::EXPR)?,
                content: self.opt_string(n, vt::INLINE_CONTENT)?,
            }
        } else if self.has_type(n, vt::SCRIPT) {
            ExecKind::Script(self.script(n)?)
        } else if self.has_type(n, vt::IF) {
            ExecKind::If {
                branches: self.map(n, vt::BRANCH, |d, b| {
                    Ok(IfBranch {
                        cond: d.opt_expr(b, vt::COND)?,
                        body: d.block(b, vt::ACTION)?,
                        span: Span::NONE,
                    })
                })?,
            }
        } else if self.has_type(n, vt::FOREACH) {
            ExecKind::Foreach {
                array: self.expr(n, vt::ARRAY)?,
                item: self.string(n, vt::ITEM)?,
                index: self.opt_string(n, vt::INDEX)?,
                body: self.block(n, vt::ACTION)?,
            }
        } else if self.has_type(n, vt::EXTENSION_CONTENT) {
            ExecKind::Extension(self.extension_fields(n)?)
        } else {
            return Err(err(
                n.as_str(),
                None,
                "executable content of unknown kind".into(),
            ));
        };
        Ok(Exec {
            kind,
            extensions: self.extensions(n)?,
            span: Span::NONE,
        })
    }

    fn script(&self, n: &NamedNode) -> Result<Script> {
        Ok(Script {
            src: self.opt_string(n, vt::SRC)?,
            body: self.opt_expr(n, vt::SCRIPT_BODY)?,
            span: Span::NONE,
        })
    }

    fn params(&self, owner: &NamedNode) -> Result<Vec<Param>> {
        self.map(owner, vt::PARAM_P, |d, p| {
            Ok(Param {
                name: d.string(p, vt::PARAM_NAME)?,
                expr: d.opt_expr(p, vt::EXPR)?,
                location: d.opt_expr(p, vt::LOCATION)?,
                span: Span::NONE,
            })
        })
    }

    fn content(&self, owner: &NamedNode) -> Result<Option<Content>> {
        self.opt_node(owner, vt::CONTENT_P)?
            .map(|c| {
                Ok(Content {
                    expr: self.opt_expr(&c, vt::EXPR)?,
                    body: self.opt_string(&c, vt::INLINE_CONTENT)?,
                    span: Span::NONE,
                })
            })
            .transpose()
    }

    fn data_item(&self, n: &NamedNode) -> Result<Data> {
        Ok(Data {
            id: self.string(n, vt::DATA_ID)?,
            src: self.opt_string(n, vt::SRC)?,
            expr: self.opt_expr(n, vt::EXPR)?,
            content: self.opt_string(n, vt::INLINE_CONTENT)?,
            extensions: self.extensions(n)?,
            span: Span::NONE,
        })
    }

    fn donedata(&self, n: &NamedNode) -> Result<DoneData> {
        Ok(DoneData {
            content: self.content(n)?,
            params: self.params(n)?,
            span: Span::NONE,
        })
    }

    fn invoke(&self, n: &NamedNode) -> Result<Invoke> {
        Ok(Invoke {
            type_: self.opt_string(n, vt::TYPE)?,
            typeexpr: self.opt_expr(n, vt::TYPEEXPR)?,
            src: self.opt_string(n, vt::SRC)?,
            srcexpr: self.opt_expr(n, vt::SRCEXPR)?,
            id: self.opt_string(n, vt::INVOKE_ID)?,
            idlocation: self.opt_expr(n, vt::IDLOCATION)?,
            namelist: self.tokens(n, vt::NAMELIST)?,
            autoforward: self.boolean(n, vt::AUTOFORWARD)?,
            params: self.params(n)?,
            content: self.content(n)?,
            finalize: self.block(n, vt::FINALIZE)?,
            extensions: self.extensions(n)?,
            span: Span::NONE,
        })
    }

    fn extensions(&self, owner: &NamedNode) -> Result<Vec<Extension>> {
        self.map(owner, vt::EXTENSION, Self::extension_fields)
    }

    fn extension_fields(&self, n: &NamedNode) -> Result<Extension> {
        let value = match (
            self.opt_string(n, vt::ATTRIBUTE_VALUE)?,
            self.opt_string(n, vt::ELEMENT_XML)?,
        ) {
            (Some(a), None) => ExtensionValue::Attribute(a),
            (None, Some(e)) => ExtensionValue::Element(e),
            _ => {
                return Err(err(
                    n.as_str(),
                    None,
                    "extension needs exactly one of vt:attributeValue and vt:elementXml".into(),
                ));
            }
        };
        let position = self.integer(n, vt::POSITION)?;
        Ok(Extension {
            ns: self.string(n, vt::NAMESPACE)?,
            local: self.string(n, vt::LOCAL_NAME)?,
            value,
            position: u32::try_from(position).map_err(|_| {
                err(
                    n.as_str(),
                    Some(vt::POSITION.as_str()),
                    "position out of range".into(),
                )
            })?,
            span: Span::NONE,
        })
    }
}
