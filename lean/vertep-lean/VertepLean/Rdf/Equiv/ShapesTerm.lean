import LeanRDFShacl
import VertepLean.Rdf.Project

/-!
# The chart shapes as a Lean term

`chartShapesL` is `crates/vertep-rdf/shapes/vt-chart-shapes.ttl` transcribed into LeanRDF's
`Shacl.Shape` terms, one top-level shape per Turtle shape with a target. The Turtle parser and the
SHACL shapes reader of LeanRDF are `partial` (opaque to the kernel), so `Rdf.chartShapes`, which
reads the Turtle, cannot be evaluated in a proof; the Shapes Equivalence Theorem is stated over
this term instead, and `VertepTests/ShapesEquiv.lean` checks (`#guard`) that reading the Turtle
gives the same shapes up to the parts SHACL validation ignores: shape identifiers, messages and
severities, and the order of components, targets and top-level shapes (`canonShapes`).
-/

namespace Vertep.Rdf.ShapesL

open LeanRDF LeanRDF.Shacl

/-! ## Building blocks -/

/-- A `vt:` IRI. -/
def vi (l : String) : IRI := ⟨vtNs ++ l⟩
/-- The predicate path `vt:l`. -/
def P (l : String) : Path := .pred (vi l)
/-- The inverse path of `vt:l`. -/
def I (l : String) : Path := .inv (P l)

def xsdInt : Term := .lit (.typed "0" xsdInteger)

/-- An anonymous shape (identifiers, severities and messages do not take part in validation). -/
def mk (path : Option Path) (targets : List Target) (comps : List Component) : Shape :=
  .mk (.iri ⟨""⟩) path targets comps false (.iri sh.Violation) []

/-- A node shape without targets. -/
def nodeS (comps : List Component) : Shape := mk none [] comps
/-- A property shape. -/
def propS (path : Path) (comps : List Component) : Shape := mk (some path) [] comps
/-- A property shape on `vt:l`. -/
def pr (l : String) (comps : List Component) : Component := .property (propS (P l) comps)

def cls (l : String) : Component := .class_ (vt l)
def dt (d : IRI) : Component := .test (.datatype d)
def lit : Component := .test (.nodeKind .literal)
def minC (n : Nat) : Component := .minCount n
def maxC (n : Nat) : Component := .maxCount n

/-- `[ sh:path vt:l ; sh:maxCount 0 ]` as a shape. -/
def none0 (l : String) : Shape := propS (P l) [maxC 0]
/-- `[ sh:path vt:l ; sh:minCount 1 ]` as a shape. -/
def some1 (l : String) : Shape := propS (P l) [minC 1]
/-- Not both of two properties. -/
def notBoth (a b : String) : Component := .or [none0 a, none0 b]

/-- `vt:l` at most once, an `xsd:string`. -/
def optStrP (l : String) : Component := pr l [dt xsdString, maxC 1]
/-- `vt:l` exactly once, an `xsd:string`. -/
def strP (l : String) : Component := pr l [dt xsdString, minC 1, maxC 1]
/-- `vt:l` at most once, a literal. -/
def optLitP (l : String) : Component := pr l [lit, maxC 1]
/-- `vt:l` exactly once, an `xsd:boolean`. -/
def boolP (l : String) : Component := pr l [dt xsdBoolean, minC 1, maxC 1]
/-- Every `vt:l` is an instance of `vt:c`. -/
def clsP (l c : String) : Component := pr l [cls c]

/-- `vt:OrderedShape`. -/
def orderedS : Shape :=
  nodeS [pr "order" [dt xsdInteger, minC 1, maxC 1, .test (.range .minInclusive xsdInt)]]

/-- `vt:TargetRefShape`. -/
def targetRefS : Shape :=
  nodeS [cls "TargetRef", .not (propS (.seq [P "state", P "ancestor"]) [.disjoint (vi "scope")]),
    .node orderedS, pr "state" [cls "StateNode", minC 1, maxC 1], pr "scope" [minC 1, maxC 1]]

/-- `vt:ExtensionFieldsShape`. -/
def extensionFieldsS : Shape :=
  nodeS [.xone [some1 "attributeValue", some1 "elementXml"], strP "namespace", strP "localName",
    optStrP "attributeValue", optStrP "elementXml",
    pr "position" [dt xsdInteger, minC 1, maxC 1, .test (.range .minInclusive xsdInt)]]

/-- `vt:EventlessTargetedShape`. -/
def eventlessTargetedS : Shape :=
  nodeS [pr "event" [maxC 0], pr "cond" [maxC 0], pr "target" [minC 1]]

/-- The containment predicates, in the order of `vt:ContainedShape`. -/
def containers : List String :=
  ["child", "initial", "initialElement", "initialTransition", "transition", "onentry", "onexit",
   "action", "finalize", "branch", "data", "script", "invoke", "donedata", "param", "content",
   "extension", "target", "event", "namelist"]

/-- One step from a node to its container. -/
def upPath : Path := .alt (containers.map I)

/-- The classes `vt:ContainedShape` targets. -/
def containedClasses : List String :=
  ["StateNode", "Initial", "Transition", "OnEntry", "OnExit", "ExecutableContent", "Raise", "Send",
   "Cancel", "Log", "Assign", "Script", "If", "IfBranch", "Foreach", "ExtensionContent", "Param",
   "Content", "DataItem", "DoneData", "Invoke", "ExtensionNode", "TargetRef", "Token", "State",
   "Parallel", "Final", "History"]

def tcls (l : String) : Target := .class_ (vt l)

/-- A top-level node shape. -/
def top (targets : List Target) (comps : List Component) : Shape := mk none targets comps

/-! ## The shapes -/

def containedS : Shape :=
  top (containedClasses.map tcls)
    [.property (propS upPath [minC 1, maxC 1]),
     .property (propS (.oneOrMore upPath) [.qualifiedMin (nodeS [cls "Chart"]) 1 []])]

def chartS : Shape :=
  top [tcls "Chart"]
    [.property (propS (.alt [I "child", I "action", I "extension"]) [maxC 0]),
     optStrP "name", strP "datamodel", boolP "datamodelExplicit",
     pr "binding" [minC 1, maxC 1, .test (.in_ [strLit "early", strLit "late"])],
     boolP "bindingExplicit",
     pr "child" [cls "StateNode", minC 1],
     pr "initial" [.node targetRefS],
     clsP "data" "DataItem",
     pr "script" [cls "Script", maxC 1],
     clsP "extension" "ExtensionNode",
     pr "child" [.qualifiedMax (nodeS [cls "History"]) 0 []]]

def stateNodeS : Shape :=
  top [tcls "StateNode"]
    [.xone [nodeS [cls "State"], nodeS [cls "Parallel"], nodeS [cls "Final"], nodeS [cls "History"]],
     .or [none0 "initial", none0 "initialElement"],
     .node orderedS,
     pr "id" [dt xsdString, minC 1, maxC 1, .test (.minLength 1)],
     boolP "idGenerated",
     .property (propS (.oneOrMore (I "child")) [.equals (vi "ancestor")]),
     .property (propS (I "child") [maxC 1]),
     clsP "child" "StateNode",
     clsP "transition" "Transition",
     pr "initial" [.node targetRefS],
     pr "initialElement" [cls "Initial", maxC 1],
     clsP "onentry" "OnEntry",
     clsP "onexit" "OnExit",
     clsP "data" "DataItem",
     clsP "invoke" "Invoke",
     pr "donedata" [cls "DoneData", maxC 1],
     clsP "extension" "ExtensionNode"]

def uniqueStateIdS : Shape :=
  top [.objectsOf (vi "id")] [.property (propS (I "id") [maxC 1])]

/-- No history-only children (`vt:StateShape`, `vt:ParallelShape`). -/
def notHistoryOnly : Component :=
  .or [none0 "child", propS (P "child") [.qualifiedMin (nodeS [.not (nodeS [cls "History"])]) 1 []]]

/-- The scope of a transition's targets is the chart. -/
def chartScope : Component :=
  .property (propS (.seq [P "transition", P "target", P "scope"]) [cls "Chart"])

def stateS : Shape :=
  top [tcls "State"]
    [notHistoryOnly, pr "historyType" [maxC 0], pr "donedata" [maxC 0], chartScope]

def parallelS : Shape :=
  top [tcls "Parallel"]
    [notHistoryOnly, pr "historyType" [maxC 0],
     .property (propS (.alt [P "initial", P "initialElement"]) [maxC 0]),
     pr "donedata" [maxC 0], chartScope]

def finalS : Shape :=
  top [tcls "Final"]
    [pr "historyType" [maxC 0],
     .property (propS (.alt [P "child", P "transition", P "invoke", P "data", P "initial",
       P "initialElement"]) [maxC 0])]

def historyS : Shape :=
  top [tcls "History"]
    [pr "historyType" [minC 1, maxC 1, .test (.in_ [strLit "shallow", strLit "deep"])],
     pr "transition" [maxC 1, .node eventlessTargetedS],
     .property (propS (.alt [P "child", P "onentry", P "onexit", P "invoke", P "data", P "initial",
       P "initialElement", P "donedata"]) [maxC 0]),
     .property (propS (I "child") [cls "StateNode"]),
     .property (propS (.seq [.oneOrMore (P "historyDefault"), P "id"]) [.disjoint (vi "id")]),
     .property (propS (.seq [P "transition", P "target"])
       [.property (propS (.seq [I "target", I "transition", I "child"]) [.equals (vi "scope")])])]

def initialRefScopeS : Shape :=
  top [.objectsOf (vi "initial")] [.property (propS (I "initial") [.equals (vi "scope")])]

def initialS : Shape :=
  top [tcls "Initial"]
    [pr "initialTransition" [cls "Transition", minC 1, maxC 1, .node eventlessTargetedS],
     .property (propS (.seq [P "initialTransition", P "target"])
       [.property (propS (.seq [I "target", I "initialTransition", I "initialElement"])
         [.equals (vi "scope")])]),
     clsP "extension" "ExtensionNode"]

def transitionS : Shape :=
  top [tcls "Transition"]
    [.node orderedS,
     pr "source" [cls "StateNode", minC 1, maxC 1],
     .property (propS (.alt [I "transition", .seq [I "initialTransition", I "initialElement"]])
       [.equals (vi "source")]),
     clsP "event" "Token",
     optLitP "cond",
     pr "target" [.node targetRefS],
     pr "transitionType" [minC 1, maxC 1, .test (.in_ [strLit "external", strLit "internal"])],
     boolP "transitionTypeExplicit",
     clsP "action" "ExecutableContent",
     clsP "extension" "ExtensionNode"]

def selectionS : Shape :=
  top [.subjectsOf (vi "choice")] [pr "choice" [maxC 1]]

def tokenS : Shape :=
  top [tcls "Token"] [.node orderedS, strP "value"]

def handlerS : Shape :=
  top [tcls "OnEntry", tcls "OnExit"] [.node orderedS, clsP "action" "ExecutableContent"]

def execS : Shape :=
  top [tcls "ExecutableContent"]
    [.xone [nodeS [cls "Raise"], nodeS [cls "Send"], nodeS [cls "Cancel"], nodeS [cls "Log"],
      nodeS [cls "Assign"], nodeS [cls "Script"], nodeS [cls "If"], nodeS [cls "Foreach"],
      nodeS [cls "ExtensionContent"]],
     .node orderedS, clsP "extension" "ExtensionNode"]

def raiseS : Shape := top [tcls "Raise"] [strP "eventName"]

def sendS : Shape :=
  top [tcls "Send"]
    [.and [nodeS [notBoth "eventName" "eventexpr"], nodeS [notBoth "sendTarget" "targetexpr"],
      nodeS [notBoth "type" "typeexpr"], nodeS [notBoth "sendId" "idlocation"],
      nodeS [notBoth "delay" "delayexpr"],
      nodeS [.or [none0 "content", propS (.alt [P "param", P "namelist"]) [maxC 0]]]],
     optStrP "eventName", optLitP "eventexpr", optStrP "sendTarget", optLitP "targetexpr",
     optStrP "type", optLitP "typeexpr", optStrP "sendId", optLitP "idlocation", optStrP "delay",
     optLitP "delayexpr", clsP "namelist" "Token", clsP "param" "Param",
     pr "content" [cls "Content", maxC 1]]

def cancelS : Shape :=
  top [tcls "Cancel"]
    [.xone [some1 "sendId", some1 "sendidexpr"], optStrP "sendId", optLitP "sendidexpr"]

def logS : Shape := top [tcls "Log"] [optStrP "label", optLitP "expr"]

def assignS : Shape :=
  top [tcls "Assign"]
    [notBoth "expr" "inlineContent", pr "location" [lit, minC 1, maxC 1], optLitP "expr",
     optStrP "inlineContent"]

def scriptS : Shape := top [tcls "Script"] [optStrP "src", optLitP "scriptBody"]

def ifS : Shape := top [tcls "If"] [clsP "branch" "IfBranch"]

def ifBranchS : Shape :=
  top [tcls "IfBranch"]
    [.or [some1 "cond",
      nodeS [.and [propS (P "order") [.test (.range .minExclusive xsdInt)],
        propS (.seq [I "branch", P "branch", P "order"]) [.lessThanOrEquals (vi "order")]]]],
     .node orderedS, optLitP "cond", clsP "action" "ExecutableContent"]

def foreachS : Shape :=
  top [tcls "Foreach"]
    [pr "array" [lit, minC 1, maxC 1], pr "item" [dt xsdString, minC 1, maxC 1, .test (.minLength 1)],
     optStrP "index", clsP "action" "ExecutableContent"]

def extensionContentS : Shape := top [tcls "ExtensionContent"] [.node extensionFieldsS]

def paramS : Shape :=
  top [tcls "Param"]
    [.xone [some1 "expr", some1 "location"], .node orderedS, strP "paramName", optLitP "expr",
     optLitP "location"]

def contentS : Shape := top [tcls "Content"] [optLitP "expr", optStrP "inlineContent"]

def dataItemS : Shape :=
  top [tcls "DataItem"]
    [.or [none0 "expr", propS (.alt [P "src", P "inlineContent"]) [maxC 0]],
     .node orderedS, strP "dataId", optStrP "src", optLitP "expr", optStrP "inlineContent",
     clsP "extension" "ExtensionNode"]

def uniqueDataIdS : Shape :=
  top [.objectsOf (vi "dataId")] [.property (propS (I "dataId") [maxC 1])]

def doneDataS : Shape :=
  top [tcls "DoneData"]
    [notBoth "content" "param", pr "content" [cls "Content", maxC 1], clsP "param" "Param"]

def invokeS : Shape :=
  top [tcls "Invoke"]
    [.and [nodeS [notBoth "type" "typeexpr"], nodeS [notBoth "src" "srcexpr"],
      nodeS [notBoth "invokeId" "idlocation"],
      nodeS [.or [none0 "content",
        nodeS [.and [none0 "srcexpr", nodeS [.or [none0 "src",
          propS (.seq [P "content", P "expr"]) [maxC 0]]]]]]],
      nodeS [notBoth "param" "namelist"]],
     .node orderedS, optStrP "type", optLitP "typeexpr", optStrP "src", optLitP "srcexpr",
     optStrP "invokeId", optLitP "idlocation", clsP "namelist" "Token",
     boolP "autoforward", clsP "param" "Param", pr "content" [cls "Content", maxC 1],
     clsP "finalize" "ExecutableContent", clsP "extension" "ExtensionNode"]

def extensionNodeS : Shape :=
  top [tcls "ExtensionNode"] [.node orderedS, .node extensionFieldsS]

/-- The chart shapes: every shape of `vt-chart-shapes.ttl` with a target. -/
def chartShapesL : List Shape :=
  [containedS, chartS, stateNodeS, uniqueStateIdS, stateS, parallelS, finalS, historyS,
   initialRefScopeS, initialS, transitionS, selectionS, tokenS, handlerS, execS, raiseS, sendS,
   cancelS, logS, assignS, scriptS, ifS, ifBranchS, foreachS, extensionContentS, paramS,
   contentS, dataItemS, uniqueDataIdS, doneDataS, invokeS, extensionNodeS]

/-! ## Canonical form (for the test against the Turtle) -/

def sortS (l : List String) : List String := l.mergeSort (fun a b => decide (a ≤ b))

mutual
def canonPathL : List Path → List String
  | [] => []
  | p :: ps => canonPath p :: canonPathL ps
def canonPath : Path → String
  | .pred p => s!"<{p.val}>"
  | .inv p => s!"^({canonPath p})"
  | .seq ps => s!"seq[{", ".intercalate (canonPathL ps)}]"
  | .alt ps => s!"alt[{", ".intercalate (canonPathL ps)}]"
  | .zeroOrMore p => s!"({canonPath p})*"
  | .oneOrMore p => s!"({canonPath p})+"
  | .zeroOrOne p => s!"({canonPath p})?"
end

def canonTarget : Target → String
  | .node t => s!"node {reprStr t}"
  | .class_ c => s!"class {reprStr c}"
  | .subjectsOf p => s!"subjectsOf <{p.val}>"
  | .objectsOf p => s!"objectsOf <{p.val}>"

mutual
/-- A shape without its identifier, severity and messages, its targets and components sorted. -/
def canonShape : Shape → String
  | .mk _ path targets comps deact _ _ =>
    let p := match path with
      | some p => canonPath p
      | none => "-"
    s!"shape({p}; {sortS (targets.map canonTarget)}; {sortS (canonComps comps)}; {deact})"
def canonComps : List Component → List String
  | [] => []
  | c :: cs => canonComp c :: canonComps cs
def canonShapes : List Shape → List String
  | [] => []
  | s :: ss => canonShape s :: canonShapes ss
def canonComp : Component → String
  | .test t => s!"test {reprStr t}"
  | .class_ c => s!"class {reprStr c}"
  | .minCount n => s!"minCount {n}"
  | .maxCount n => s!"maxCount {n}"
  | .uniqueLang => "uniqueLang"
  | .equals p => s!"equals <{p.val}>"
  | .disjoint p => s!"disjoint <{p.val}>"
  | .lessThan p => s!"lessThan <{p.val}>"
  | .lessThanOrEquals p => s!"lessThanOrEquals <{p.val}>"
  | .not s => s!"not {canonShape s}"
  | .and ss => s!"and {canonShapes ss}"
  | .or ss => s!"or {canonShapes ss}"
  | .xone ss => s!"xone {canonShapes ss}"
  | .node s => s!"node {canonShape s}"
  | .property s => s!"property {canonShape s}"
  | .qualifiedMin s n sibs => s!"qualifiedMin {canonShape s} {n} {canonShapes sibs}"
  | .qualifiedMax s n sibs => s!"qualifiedMax {canonShape s} {n} {canonShapes sibs}"
  | .closed allowed => s!"closed {allowed.map (·.val)}"
  | .hasValue t => s!"hasValue {reprStr t}"
end

/-- The canonical form of a list of shapes: each shape's, sorted. -/
def canon (ss : List Shape) : List String := sortS (canonShapes ss)

end Vertep.Rdf.ShapesL
