//! The terms of the `vt:` chart ontology module, as IRI constants.
//!
//! The normative definitions (labels, comments, domains and ranges) are in
//! `ontology/vt-chart.ttl`; this module only names the terms so that the projection, the
//! reconstruction and the tests share one spelling. [`NodeKind`] is the list of IR node kinds
//! and the one class each maps to.

use oxrdf::NamedNodeRef;

/// The Vertep namespace, shared with the SCXML extension namespace.
pub const VT: &str = vertep_core::ir::VT_NS;
/// `rdf:type`.
pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
/// `rdfs:subClassOf`.
pub const RDFS_SUB_CLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
/// `xsd:string`.
pub const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
/// `xsd:boolean`.
pub const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";
/// `xsd:integer`.
pub const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";

macro_rules! vt_terms {
    ($($(#[$doc:meta])* $name:ident = $local:literal;)*) => {
        $(
            $(#[$doc])*
            pub const $name: NamedNodeRef<'static> =
                NamedNodeRef::new_unchecked(concat!("https://w3id.org/vertep/vt#", $local));
        )*
    };
}

vt_terms! {
    // Classes.
    /// `vt:Chart`: the root `<scxml>` element.
    CHART = "Chart";
    /// `vt:StateNode`: any state node (`ir::State`).
    STATE_NODE = "StateNode";
    /// `vt:State`: a state written as `<state>`.
    STATE = "State";
    /// `vt:Parallel`: a state written as `<parallel>`.
    PARALLEL = "Parallel";
    /// `vt:Final`: a state written as `<final>`.
    FINAL = "Final";
    /// `vt:History`: a state written as `<history>`.
    HISTORY = "History";
    /// `vt:Initial`: the `<initial>` pseudo-state.
    INITIAL = "Initial";
    /// `vt:Transition`.
    TRANSITION = "Transition";
    /// `vt:OnEntry`: one `<onentry>` handler.
    ON_ENTRY = "OnEntry";
    /// `vt:OnExit`: one `<onexit>` handler.
    ON_EXIT = "OnExit";
    /// `vt:ExecutableContent`: any executable content element (`ir::Exec`).
    EXECUTABLE_CONTENT = "ExecutableContent";
    /// `vt:Raise`.
    RAISE = "Raise";
    /// `vt:Send`.
    SEND = "Send";
    /// `vt:Cancel`.
    CANCEL = "Cancel";
    /// `vt:Log`.
    LOG = "Log";
    /// `vt:Assign`.
    ASSIGN = "Assign";
    /// `vt:Script`.
    SCRIPT = "Script";
    /// `vt:If`.
    IF = "If";
    /// `vt:IfBranch`: one `<if>`, `<elseif>` or `<else>` branch.
    IF_BRANCH = "IfBranch";
    /// `vt:Foreach`.
    FOREACH = "Foreach";
    /// `vt:ExtensionContent`: an executable content element from another namespace.
    EXTENSION_CONTENT = "ExtensionContent";
    /// `vt:Param`.
    PARAM = "Param";
    /// `vt:Content`.
    CONTENT = "Content";
    /// `vt:DataItem`: a `<data>` element.
    DATA_ITEM = "DataItem";
    /// `vt:DoneData`.
    DONE_DATA = "DoneData";
    /// `vt:Invoke`.
    INVOKE = "Invoke";
    /// `vt:ExtensionNode`: an extension attribute or element kept on its owner.
    EXTENSION_NODE = "ExtensionNode";
    /// `vt:TargetRef`: one member of a state identifier list (`target`, `initial`).
    TARGET_REF = "TargetRef";
    /// `vt:Token`: one member of a name list (`event`, `namelist`).
    TOKEN = "Token";

    // Containment properties.
    /// `vt:child`.
    CHILD = "child";
    /// `vt:initial`.
    INITIAL_REF = "initial";
    /// `vt:initialElement`.
    INITIAL_ELEMENT = "initialElement";
    /// `vt:initialTransition`.
    INITIAL_TRANSITION = "initialTransition";
    /// `vt:transition`.
    TRANSITION_P = "transition";
    /// `vt:onentry`.
    ONENTRY = "onentry";
    /// `vt:onexit`.
    ONEXIT = "onexit";
    /// `vt:action`.
    ACTION = "action";
    /// `vt:finalize`.
    FINALIZE = "finalize";
    /// `vt:branch`.
    BRANCH = "branch";
    /// `vt:data`.
    DATA = "data";
    /// `vt:script`.
    SCRIPT_P = "script";
    /// `vt:invoke`.
    INVOKE_P = "invoke";
    /// `vt:donedata`.
    DONEDATA = "donedata";
    /// `vt:param`.
    PARAM_P = "param";
    /// `vt:content`.
    CONTENT_P = "content";
    /// `vt:extension`.
    EXTENSION = "extension";
    /// `vt:target`.
    TARGET = "target";
    /// `vt:event`.
    EVENT = "event";
    /// `vt:namelist`.
    NAMELIST = "namelist";

    // Reference and derived properties.
    /// `vt:state`: the state a target reference points to.
    STATE_P = "state";
    /// `vt:scope`: the node whose proper descendants a target reference may name.
    SCOPE = "scope";
    /// `vt:source`: the source state of a transition (derived).
    SOURCE = "source";
    /// `vt:ancestor`: a proper ancestor of a state, the chart included (derived).
    ANCESTOR = "ancestor";
    /// `vt:historyDefault`: a history state targeted by a history's default entry (derived).
    HISTORY_DEFAULT = "historyDefault";
    /// `vt:choice`: the branch a target takes at an ancestor, on a selection node (derived).
    CHOICE = "choice";

    // Attribute properties.
    /// `vt:order`.
    ORDER = "order";
    /// `vt:name`.
    NAME = "name";
    /// `vt:datamodel`.
    DATAMODEL = "datamodel";
    /// `vt:datamodelExplicit`.
    DATAMODEL_EXPLICIT = "datamodelExplicit";
    /// `vt:binding`.
    BINDING = "binding";
    /// `vt:bindingExplicit`.
    BINDING_EXPLICIT = "bindingExplicit";
    /// `vt:id`.
    ID = "id";
    /// `vt:idGenerated`.
    ID_GENERATED = "idGenerated";
    /// `vt:historyType`.
    HISTORY_TYPE = "historyType";
    /// `vt:cond`.
    COND = "cond";
    /// `vt:transitionType`.
    TRANSITION_TYPE = "transitionType";
    /// `vt:transitionTypeExplicit`.
    TRANSITION_TYPE_EXPLICIT = "transitionTypeExplicit";
    /// `vt:value`.
    VALUE = "value";
    /// `vt:eventName`.
    EVENT_NAME = "eventName";
    /// `vt:eventexpr`.
    EVENTEXPR = "eventexpr";
    /// `vt:sendTarget`.
    SEND_TARGET = "sendTarget";
    /// `vt:targetexpr`.
    TARGETEXPR = "targetexpr";
    /// `vt:type`.
    TYPE = "type";
    /// `vt:typeexpr`.
    TYPEEXPR = "typeexpr";
    /// `vt:sendId`.
    SEND_ID = "sendId";
    /// `vt:sendidexpr`.
    SENDIDEXPR = "sendidexpr";
    /// `vt:idlocation`.
    IDLOCATION = "idlocation";
    /// `vt:delay`.
    DELAY = "delay";
    /// `vt:delayexpr`.
    DELAYEXPR = "delayexpr";
    /// `vt:label`.
    LABEL = "label";
    /// `vt:expr`.
    EXPR = "expr";
    /// `vt:location`.
    LOCATION = "location";
    /// `vt:inlineContent`.
    INLINE_CONTENT = "inlineContent";
    /// `vt:src`.
    SRC = "src";
    /// `vt:srcexpr`.
    SRCEXPR = "srcexpr";
    /// `vt:scriptBody`.
    SCRIPT_BODY = "scriptBody";
    /// `vt:array`.
    ARRAY = "array";
    /// `vt:item`.
    ITEM = "item";
    /// `vt:index`.
    INDEX = "index";
    /// `vt:paramName`.
    PARAM_NAME = "paramName";
    /// `vt:dataId`.
    DATA_ID = "dataId";
    /// `vt:invokeId`.
    INVOKE_ID = "invokeId";
    /// `vt:autoforward`.
    AUTOFORWARD = "autoforward";
    /// `vt:namespace`.
    NAMESPACE = "namespace";
    /// `vt:localName`.
    LOCAL_NAME = "localName";
    /// `vt:attributeValue`.
    ATTRIBUTE_VALUE = "attributeValue";
    /// `vt:elementXml`.
    ELEMENT_XML = "elementXml";
    /// `vt:position`.
    POSITION = "position";
}

/// Every containment property: each node other than the chart is the object of exactly one
/// triple with one of these predicates.
pub const CONTAINMENT: [NamedNodeRef<'static>; 20] = [
    CHILD,
    INITIAL_REF,
    INITIAL_ELEMENT,
    INITIAL_TRANSITION,
    TRANSITION_P,
    ONENTRY,
    ONEXIT,
    ACTION,
    FINALIZE,
    BRANCH,
    DATA,
    SCRIPT_P,
    INVOKE_P,
    DONEDATA,
    PARAM_P,
    CONTENT_P,
    EXTENSION,
    TARGET,
    EVENT,
    NAMELIST,
];

/// The node kinds of the chart IR, each projected as instances of exactly one `vt:` class.
///
/// A kind is either an IR type (`ir::Chart`, `ir::Transition`, ...), a variant that the IR
/// distinguishes by an enum (`StateElement::Parallel`, `ExecKind::Raise`, ...), or a member of
/// an ordered list of strings that the projection reifies so that it can carry `vt:order`
/// (`TargetRef` for state identifier lists, `Token` for name lists).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NodeKind {
    /// `ir::Chart`.
    Chart,
    /// `ir::State`, any element.
    StateNode,
    /// `ir::StateElement::State`.
    State,
    /// `ir::StateElement::Parallel`.
    Parallel,
    /// `ir::StateElement::Final`.
    Final,
    /// `ir::StateElement::History`.
    History,
    /// `ir::Initial`.
    Initial,
    /// `ir::Transition`.
    Transition,
    /// One `ir::Block` of `State::onentry`.
    OnEntry,
    /// One `ir::Block` of `State::onexit`.
    OnExit,
    /// `ir::Exec`, any kind.
    ExecutableContent,
    /// `ir::ExecKind::Raise`.
    Raise,
    /// `ir::ExecKind::Send` (`ir::Send`).
    Send,
    /// `ir::ExecKind::Cancel`.
    Cancel,
    /// `ir::ExecKind::Log`.
    Log,
    /// `ir::ExecKind::Assign`.
    Assign,
    /// `ir::Script`, top-level or `ir::ExecKind::Script`.
    Script,
    /// `ir::ExecKind::If`.
    If,
    /// `ir::IfBranch`.
    IfBranch,
    /// `ir::ExecKind::Foreach`.
    Foreach,
    /// `ir::ExecKind::Extension`.
    ExtensionContent,
    /// `ir::Param`.
    Param,
    /// `ir::Content`.
    Content,
    /// `ir::Data`.
    DataItem,
    /// `ir::DoneData`.
    DoneData,
    /// `ir::Invoke`.
    Invoke,
    /// `ir::Extension`.
    ExtensionNode,
    /// A member of `Chart::initial`, `State::initial` or `Transition::targets`.
    TargetRef,
    /// A member of `Transition::events`, `Send::namelist` or `Invoke::namelist`.
    Token,
}

impl NodeKind {
    /// Every node kind, in declaration order.
    pub const ALL: [NodeKind; 29] = [
        NodeKind::Chart,
        NodeKind::StateNode,
        NodeKind::State,
        NodeKind::Parallel,
        NodeKind::Final,
        NodeKind::History,
        NodeKind::Initial,
        NodeKind::Transition,
        NodeKind::OnEntry,
        NodeKind::OnExit,
        NodeKind::ExecutableContent,
        NodeKind::Raise,
        NodeKind::Send,
        NodeKind::Cancel,
        NodeKind::Log,
        NodeKind::Assign,
        NodeKind::Script,
        NodeKind::If,
        NodeKind::IfBranch,
        NodeKind::Foreach,
        NodeKind::ExtensionContent,
        NodeKind::Param,
        NodeKind::Content,
        NodeKind::DataItem,
        NodeKind::DoneData,
        NodeKind::Invoke,
        NodeKind::ExtensionNode,
        NodeKind::TargetRef,
        NodeKind::Token,
    ];

    /// The class this node kind is projected as.
    pub const fn class(self) -> NamedNodeRef<'static> {
        match self {
            NodeKind::Chart => CHART,
            NodeKind::StateNode => STATE_NODE,
            NodeKind::State => STATE,
            NodeKind::Parallel => PARALLEL,
            NodeKind::Final => FINAL,
            NodeKind::History => HISTORY,
            NodeKind::Initial => INITIAL,
            NodeKind::Transition => TRANSITION,
            NodeKind::OnEntry => ON_ENTRY,
            NodeKind::OnExit => ON_EXIT,
            NodeKind::ExecutableContent => EXECUTABLE_CONTENT,
            NodeKind::Raise => RAISE,
            NodeKind::Send => SEND,
            NodeKind::Cancel => CANCEL,
            NodeKind::Log => LOG,
            NodeKind::Assign => ASSIGN,
            NodeKind::Script => SCRIPT,
            NodeKind::If => IF,
            NodeKind::IfBranch => IF_BRANCH,
            NodeKind::Foreach => FOREACH,
            NodeKind::ExtensionContent => EXTENSION_CONTENT,
            NodeKind::Param => PARAM,
            NodeKind::Content => CONTENT,
            NodeKind::DataItem => DATA_ITEM,
            NodeKind::DoneData => DONE_DATA,
            NodeKind::Invoke => INVOKE,
            NodeKind::ExtensionNode => EXTENSION_NODE,
            NodeKind::TargetRef => TARGET_REF,
            NodeKind::Token => TOKEN,
        }
    }
}

/// The datatype of expression, location and script literals for a datamodel: `vt:<name>Expr`,
/// with the name percent-encoded where it is not an IRI-safe unreserved character, so that
/// `ecmascript` gives `vt:ecmascriptExpr` and `null` gives `vt:nullExpr`.
pub fn expr_datatype(datamodel: &str) -> String {
    let mut iri = String::from(VT);
    crate::iri::percent_encode_into(datamodel, &mut iri);
    iri.push_str("Expr");
    iri
}
