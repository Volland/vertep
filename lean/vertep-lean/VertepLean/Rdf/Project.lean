import LeanRDF.Core.Graph
import VertepLean.IR

/-!
# The chart projection on LeanRDF

`toRDF` mirrors `crates/vertep-rdf/src/project.rs` triple for triple: the `vt:` chart ontology,
node IRIs `<base>#<fragment>` minted from state identifiers and document positions, no blank
nodes, explicit `vt:order`, expressions as literals typed `vt:<datamodel>Expr`, and the derived
triples the SHACL Core shapes need (`vt:ancestor`, `vt:source`, `vt:scope`, selection nodes with
`vt:choice`, `vt:historyDefault`).
-/

namespace Vertep.Rdf

open LeanRDF

/-! ## Vocabulary -/

def vtNs : String := "https://w3id.org/vertep/vt#"
def vt (l : String) : Term := .iri ⟨vtNs ++ l⟩
def rdfType : Term := .iri ⟨"http://www.w3.org/1999/02/22-rdf-syntax-ns#type"⟩
def xsdString : IRI := ⟨"http://www.w3.org/2001/XMLSchema#string"⟩
def xsdBoolean : IRI := ⟨"http://www.w3.org/2001/XMLSchema#boolean"⟩
def xsdInteger : IRI := ⟨"http://www.w3.org/2001/XMLSchema#integer"⟩

/-- The base node IRIs are minted under before the version is known. -/
def placeholderBase : String := "urn:vertep:chart:self"
/-- The prefix of chart version IRIs. -/
def versionPrefix : String := "urn:vertep:chart:sha256:"

/-- Rust `percent_encode_into`: every UTF-8 byte outside `A-Z a-z 0-9 - . _ ~` as `%XX`. -/
def percentEncode (s : String) : String :=
  let hex := "0123456789ABCDEF".toList
  String.ofList (s.toUTF8.toList.flatMap fun b =>
    let b : Nat := b.toNat
    let c := Char.ofNat b
    if (b < 128 && (c.isAlphanum || c == '-' || c == '.' || c == '_' || c == '~')) then [c]
    else ['%', hex[b / 16]!, hex[b % 16]!])

/-- The expression datatype of a datamodel: `vt:` + percent-encoded name + `Expr`. -/
def exprDatatype (datamodel : String) : IRI := ⟨vtNs ++ percentEncode datamodel ++ "Expr"⟩

/-- The fragment of the state with this identifier. -/
def stateFragment (id : String) : String := "s." ++ percentEncode id

def strLit (v : String) : Term := .lit (.typed v xsdString)
def boolLit (b : Bool) : Term := .lit (.typed (if b then "true" else "false") xsdBoolean)
def intLit (n : Nat) : Term := .lit (.typed (toString n) xsdInteger)

def add (s p o : Term) : List Triple := [⟨s, p, o⟩]
def str (n : Term) (p : String) (v : String) : List Triple := [⟨n, vt p, strLit v⟩]
def optStr (n : Term) (p : String) : Option String → List Triple
  | some v => str n p v
  | none => []
def bool (n : Term) (p : String) (b : Bool) : List Triple := [⟨n, vt p, boolLit b⟩]
def int (n : Term) (p : String) (v : Nat) : List Triple := [⟨n, vt p, intLit v⟩]
def ordT (n : Term) (i : Nat) : List Triple := int n "order" i

/-! ## The projector -/

/-- What the projection needs besides the node being projected. -/
structure Ctx where
  base : String
  exprDt : IRI
  /-- For each identifier (first in document order): the chain of `(fragment, isParallel)` from
  the chart node to the state (a history state: to its parent; at the root: empty). -/
  chains : List (String × List (String × Bool))
  /-- For each identifier (first in document order): whether it names a history state. -/
  histories : List (String × Bool)

def execClass : ExecKind → String
  | .raise _ => "Raise"
  | .send _ => "Send"
  | .cancel _ _ => "Cancel"
  | .log _ _ => "Log"
  | .assign .. => "Assign"
  | .script _ => "Script"
  | .if_ _ => "If"
  | .foreach .. => "Foreach"
  | .extension _ => "ExtensionContent"

def elementClass : StateElement → String
  | .state => "State"
  | .parallel => "Parallel"
  | .final => "Final"
  | .history _ => "History"

namespace Ctx
variable (k : Ctx)

def iri (frag : String) : Term := .iri ⟨k.base ++ "#" ++ frag⟩


/-- A node linked from its owner, with its classes. -/
def node (owner : Term) (prop : String) (frag : String) (classes : List String) : List Triple :=
  ⟨owner, vt prop, k.iri frag⟩ :: classes.map (fun c => ⟨k.iri frag, rdfType, vt c⟩)

def expr (n : Term) (p : String) (v : String) : List Triple := [⟨n, vt p, .lit (.typed v k.exprDt)⟩]
def optExpr (n : Term) (p : String) : Option String → List Triple
  | some v => k.expr n p v
  | none => []

/-- A target list: one `vt:TargetRef` per member, with the selection nodes. -/
def targets (owner : Term) (prop list refStep : String) (ids : List String) (scope : Term) :
    List Triple :=
  ids.zipIdx.flatMap fun (id, j) =>
    let rfrag := list ++ refStep ++ toString j
    let r := k.iri rfrag
    k.node owner prop rfrag ["TargetRef"] ++ ordT r j ++
      [⟨r, vt "state", k.iri (stateFragment id)⟩, ⟨r, vt "scope", scope⟩] ++
      match k.chains.lookup id with
      | none => []
      | some chain =>
        chain.zipIdx.flatMap fun ((x, parallel), i) =>
          let sel := k.iri (list ++ ":sel:" ++ x)
          let choice := match chain[i + 1]? with
            | none => r
            | some (c, _) => if parallel then k.iri x else k.iri c
          [⟨sel, vt "choice", choice⟩]

/-- An ordered list of strings as `vt:Token`s. -/
def tokens (owner : Term) (prop prefix_ : String) (vs : List String) : List Triple :=
  vs.zipIdx.flatMap fun (v, j) =>
    let f := prefix_ ++ toString j
    k.node owner prop f ["Token"] ++ ordT (k.iri f) j ++ str (k.iri f) "value" v

def extensionFields (n : Term) (x : Extension) : List Triple :=
  str n "namespace" x.ns ++ str n "localName" x.localName ++
    (match x.value with
     | .attribute v => str n "attributeValue" v
     | .element v => str n "elementXml" v) ++
    int n "position" x.position

def extensions (owner : Term) (ownerFrag : String) (xs : List Extension) : List Triple :=
  xs.zipIdx.flatMap fun (x, i) =>
    let f := ownerFrag ++ ":x" ++ toString i
    k.node owner "extension" f ["ExtensionNode"] ++ ordT (k.iri f) i ++
      extensionFields (k.iri f) x

def params (owner : Term) (frag : String) (ps : List Param) : List Triple :=
  ps.zipIdx.flatMap fun (p, j) =>
    let f := frag ++ ":p" ++ toString j
    let n := k.iri f
    k.node owner "param" f ["Param"] ++ ordT n j ++ str n "paramName" p.name ++
      k.optExpr n "expr" p.expr ++ k.optExpr n "location" p.location

def content (owner : Term) (frag : String) (c : Content) : List Triple :=
  let f := frag ++ ":c"
  let n := k.iri f
  k.node owner "content" f ["Content"] ++ k.optExpr n "expr" c.expr ++
    optStr n "inlineContent" c.body

def scriptFields (n : Term) (s : Script) : List Triple :=
  optStr n "src" s.src ++ k.optExpr n "scriptBody" s.body

mutual
/-- One executable content element. -/
def exec (owner : Term) (prop frag : String) (order : Nat) : Exec → List Triple
  | .mk kind exts =>
    let n := k.iri frag
    k.node owner prop frag ["ExecutableContent", execClass kind] ++ ordT n order ++
      execKind n frag kind ++ k.extensions n frag exts
/-- The fields of one element by kind. -/
def execKind (n : Term) (frag : String) : ExecKind → List Triple
  | .raise ev => str n "eventName" ev
  | .send s =>
    optStr n "eventName" s.event ++ k.optExpr n "eventexpr" s.eventexpr ++
    optStr n "sendTarget" s.target ++ k.optExpr n "targetexpr" s.targetexpr ++
    optStr n "type" s.type ++ k.optExpr n "typeexpr" s.typeexpr ++
    optStr n "sendId" s.id ++ k.optExpr n "idlocation" s.idlocation ++
    optStr n "delay" s.delay ++ k.optExpr n "delayexpr" s.delayexpr ++
    k.tokens n "namelist" (frag ++ ":nl") s.namelist ++ k.params n frag s.params ++
    (match s.content with
     | some c => k.content n frag c
     | none => [])
  | .cancel sid sidexpr => optStr n "sendId" sid ++ k.optExpr n "sendidexpr" sidexpr
  | .log label ex => optStr n "label" label ++ k.optExpr n "expr" ex
  | .assign loc ex c => k.expr n "location" loc ++ k.optExpr n "expr" ex ++ optStr n "inlineContent" c
  | .script s => k.scriptFields n s
  | .if_ bs => branches n frag 0 bs
  | .foreach arr item idx body =>
    k.expr n "array" arr ++ str n "item" item ++ optStr n "index" idx ++
      block n "action" (frag ++ ":a") 0 body
  | .extension x => extensionFields n x
/-- A block, element `k` named `<prefix><k>`. -/
def block (owner : Term) (prop prefix_ : String) (i : Nat) : List Exec → List Triple
  | [] => []
  | e :: es => exec owner prop (prefix_ ++ toString i) i e ++ block owner prop prefix_ (i + 1) es
/-- The branches of an `if`. -/
def branches (n : Term) (frag : String) (j : Nat) : List (Option Expr × List Exec) → List Triple
  | [] => []
  | (c, body) :: bs =>
    let bf := frag ++ ":b" ++ toString j
    let bn := k.iri bf
    k.node n "branch" bf ["IfBranch"] ++ ordT bn j ++ k.optExpr bn "cond" c ++
      block bn "action" (bf ++ ":a") 0 body ++ branches n frag (j + 1) bs
end

def transition (t : Transition) (owner : Term) (prop frag : String) (order : Nat)
    (source scope : Term) : List Triple :=
  let n := k.iri frag
  k.node owner prop frag ["Transition"] ++ ordT n order ++ [⟨n, vt "source", source⟩] ++
    k.tokens n "event" (frag ++ ":ev") t.events ++ k.optExpr n "cond" t.cond ++
    k.targets n "target" frag ":tg" t.targets scope ++
    str n "transitionType" (match t.type with
      | .external => "external"
      | .internal => "internal") ++
    bool n "transitionTypeExplicit" t.typeExplicit ++
    k.block n "action" (frag ++ ":a") 0 t.body ++ k.extensions n frag t.extensions

def dataItem (owner : Term) (ownerFrag : String) (i : Nat) (d : Data) : List Triple :=
  let f := ownerFrag ++ ":d" ++ toString i
  let n := k.iri f
  k.node owner "data" f ["DataItem"] ++ ordT n i ++ str n "dataId" d.id ++
    optStr n "src" d.src ++ k.optExpr n "expr" d.expr ++ optStr n "inlineContent" d.content ++
    k.extensions n f d.extensions

def donedata (owner : Term) (frag : String) (dd : DoneData) : List Triple :=
  let n := k.iri frag
  k.node owner "donedata" frag ["DoneData"] ++
    (match dd.content with
     | some c => k.content n frag c
     | none => []) ++ k.params n frag dd.params

def invoke (owner : Term) (frag : String) (order : Nat) (v : Invoke) : List Triple :=
  let n := k.iri frag
  k.node owner "invoke" frag ["Invoke"] ++ ordT n order ++
    optStr n "type" v.type ++ k.optExpr n "typeexpr" v.typeexpr ++
    optStr n "src" v.src ++ k.optExpr n "srcexpr" v.srcexpr ++
    optStr n "invokeId" v.id ++ k.optExpr n "idlocation" v.idlocation ++
    k.tokens n "namelist" (frag ++ ":nl") v.namelist ++ bool n "autoforward" v.autoforward ++
    k.params n frag v.params ++
    (match v.content with
     | some c => k.content n frag c
     | none => []) ++
    k.block n "finalize" (frag ++ ":f") 0 v.finalize ++ k.extensions n frag v.extensions

mutual
/-- One state and its descendants. `ancestors` starts with the chart node. -/
def state (order : Nat) (parent : Term) (ancestors : List Term) : State → List Triple
  | .mk id idGen el initial initEl children ts onentry onexit data invokes dd exts =>
    let frag := stateFragment id
    let s := k.iri frag
    let chart := ancestors.headD s
    let scope := match el with
      | .history _ => parent
      | _ => chart
    k.node parent "child" frag ["StateNode", elementClass el] ++
    (match el with
     | .history h => str s "historyType" (match h with
        | .shallow => "shallow"
        | .deep => "deep")
     | _ => []) ++
    str s "id" id ++ bool s "idGenerated" idGen ++ ordT s order ++
    ancestors.map (fun a => ⟨s, vt "ancestor", a⟩) ++
    k.targets s "initial" (frag ++ ":ia") "" initial s ++
    (match initEl with
     | some ie =>
       let ifrag := frag ++ ":init"
       let n := k.iri ifrag
       k.node s "initialElement" ifrag ["Initial"] ++
         k.transition ie.transition n "initialTransition" (ifrag ++ ":t") 0 s s ++
         k.extensions n ifrag ie.extensions
     | none => []) ++
    (ts.zipIdx.flatMap fun (t, i) =>
      k.transition t s "transition" (frag ++ ":t" ++ toString i) i s scope) ++
    (onentry.zipIdx.flatMap fun (b, i) =>
      let hf := frag ++ ":en" ++ toString i
      k.node s "onentry" hf ["OnEntry"] ++ ordT (k.iri hf) i ++
        k.block (k.iri hf) "action" (hf ++ ":a") 0 b) ++
    (onexit.zipIdx.flatMap fun (b, i) =>
      let hf := frag ++ ":ex" ++ toString i
      k.node s "onexit" hf ["OnExit"] ++ ordT (k.iri hf) i ++
        k.block (k.iri hf) "action" (hf ++ ":a") 0 b) ++
    (data.zipIdx.flatMap fun (d, i) => k.dataItem s frag i d) ++
    (invokes.zipIdx.flatMap fun (v, i) => k.invoke s (frag ++ ":inv" ++ toString i) i v) ++
    (match dd with
     | some d => k.donedata s (frag ++ ":dd") d
     | none => []) ++
    k.extensions s frag exts ++
    states 0 s (ancestors ++ [s]) children
/-- Sibling states, the first at position `i`. -/
def states (i : Nat) (parent : Term) (ancestors : List Term) : List State → List Triple
  | [] => []
  | c :: cs => state i parent ancestors c ++ states (i + 1) parent ancestors cs
end

end Ctx

/-! ## Indexing and history defaults -/

mutual
/-- `Projector::index`: chains and history flags in document order. -/
def indexState (chain : List (String × Bool)) :
    State → List (String × List (String × Bool)) × List (String × Bool)
  | .mk id _ el _ _ children .. =>
    let chain := chain ++ [(stateFragment id, el == .parallel)]
    let entry := match el with
      | .history _ => if chain.length > 2 then chain.dropLast else []
      | _ => chain
    let (cs, hs) := indexStates chain children
    ((id, entry) :: cs, (id, match el with
      | .history _ => true
      | _ => false) :: hs)
/-- `index` over siblings. -/
def indexStates (chain : List (String × Bool)) :
    List State → List (String × List (String × Bool)) × List (String × Bool)
  | [] => ([], [])
  | c :: cs =>
    let (a, b) := indexState chain c
    let (a', b') := indexStates chain cs
    (a ++ a', b ++ b')
end

/-- The default entry targets of a history state for `vt:historyDefault` (Rust
`history_defaults`): its default transition's, or with none its compound parent's `initial`
attribute or `<initial>` targets. -/
def historyDefaultTargets (s : State) (parent : Option State) : List String :=
  match s.transitions.head?, parent with
  | some t, _ => t.targets
  | none, some p =>
    if p.kind == .compound then
      if !p.initial.isEmpty then p.initial
      else match p.initialElement with
        | some i => i.transition.targets
        | none => []
    else []
  | none, none => []

mutual
/-- `vt:historyDefault` edges of a state and its descendants. -/
def historyDefaults (k : Ctx) (parent : Option State) : State → List Triple
  | s@(.mk id _ el _ _ children ..) =>
    (match el with
     | .history _ =>
       (historyDefaultTargets s parent).filterMap fun t =>
         if k.histories.lookup t == some true then
           some ⟨k.iri (stateFragment id), vt "historyDefault", k.iri (stateFragment t)⟩
         else none
     | _ => []) ++ historyDefaultsList k (some s) children
/-- `historyDefaults` over siblings. -/
def historyDefaultsList (k : Ctx) (parent : Option State) : List State → List Triple
  | [] => []
  | c :: cs => historyDefaults k parent c ++ historyDefaultsList k parent cs
end

/-- Rust `project_with_base`: the projection of a chart with node IRIs under `base`, as a list
of triples (with repetitions; `toGraph` makes it a set). -/
def project (c : Chart) (base : String) : List Triple :=
  let (chains, hs) := indexStates [("chart", false)] c.states
  let k : Ctx := { base, exprDt := exprDatatype c.datamodel, chains, histories := hs }
  let cn := k.iri "chart"
  [⟨cn, rdfType, vt "Chart"⟩] ++ optStr cn "name" c.name ++ str cn "datamodel" c.datamodel ++
    bool cn "datamodelExplicit" c.datamodelExplicit ++
    str cn "binding" (match c.binding with
      | .early => "early"
      | .late => "late") ++
    bool cn "bindingExplicit" c.bindingExplicit ++
    k.targets cn "initial" "chart:ia" "" c.initial cn ++
    (c.data.zipIdx.flatMap fun (d, i) => k.dataItem cn "chart" i d) ++
    (match c.script with
     | some s => k.node cn "script" "chart:scr" ["Script"] ++ k.scriptFields (k.iri "chart:scr") s
     | none => []) ++
    k.states 0 cn [cn] c.states ++ k.extensions cn "chart" c.extensions ++
    historyDefaultsList k none c.states

/-- The projection as a graph. -/
-- @tg: implements:: [[openspec:formal-model#RDF Round Trip Theorem]] {confidence: partial, note: "projection mirrored from vertep-rdf; round trip pending"}
def toRDF (c : Chart) (base : String := placeholderBase) : Graph := Graph.ofList (project c base)

end Vertep.Rdf
