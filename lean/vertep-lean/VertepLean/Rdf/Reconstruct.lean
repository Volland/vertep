import Std.Data.HashMap
import VertepLean.Rdf.Canon

/-!
# Reconstruction of a chart from its projection

`fromRDF` mirrors the decoder of `crates/vertep-rdf/src/reconstruct.rs`: it finds the one
`vt:Chart` node, takes the base from its IRI `<base>#chart`, decodes the chart from it down
following containment and `vt:order` only (so triples outside the projection are ignored), and
finally re-projects the decoded chart under that base and requires every projected triple to
be in the graph (the closed-world check). SHACL validation against the chart shapes is separate
(`VertepLean.Rdf.Shapes`); the oracle and `vt certify` decide `WellFormed` on the decoded chart
instead, which the decision procedure is proven to compute.
-/

namespace Vertep.Rdf

open LeanRDF

/-- A graph indexed by subject and predicate. -/
structure Index where
  objs : Std.HashMap (Term × Term) (Array Term)
  triples : List Triple

/-- Index a list of triples. -/
def Index.of (ts : List Triple) : Index :=
  { objs := ts.foldl (fun m t => m.insert (t.s, t.p) ((m.getD (t.s, t.p) #[]).push t.o)) {},
    triples := ts }

abbrev D := Except String

namespace Decode

variable (g : Index) (exprDt : IRI)

def errAt (n : Term) (p : String) (msg : String) : String :=
  s!"{writeTerm n} {writeTerm (vt p)}: {msg}"

def objects (s : Term) (p : String) : List Term :=
  ((g.objs.getD (s, vt p) #[]).toList.eraseDups)

def nodes (s : Term) (p : String) : D (List Term) :=
  (objects g s p).mapM fun o => match o with
    | .iri _ => pure o
    | _ => throw (errAt s p s!"{writeTerm o} is not an IRI")

def optNode (s : Term) (p : String) : D (Option Term) := do
  match ← nodes g s p with
  | [] => pure none
  | [n] => pure (some n)
  | _ => throw (errAt s p "more than one value")

def oneNode (s : Term) (p : String) : D Term := do
  match ← optNode g s p with
  | some n => pure n
  | none => throw (errAt s p "missing value")

def typed (s : Term) (p : String) (dt : IRI) : D (Option String) :=
  match objects g s p with
  | [] => pure none
  | [.lit (.typed v d)] => if d == dt then pure (some v)
      else throw (errAt s p s!"not a literal of datatype <{dt.val}>")
  | [_] => throw (errAt s p s!"not a literal of datatype <{dt.val}>")
  | _ => throw (errAt s p "more than one value")

def required {α : Type} (s : Term) (p : String) : Option α → D α
  | some v => pure v
  | none => throw (errAt s p "missing value")

def optString (s : Term) (p : String) : D (Option String) := typed g s p xsdString
def string (s : Term) (p : String) : D String := do required s p (← optString g s p)
def optExpr (s : Term) (p : String) : D (Option String) := typed g s p exprDt
def expr (s : Term) (p : String) : D String := do required s p (← optExpr g exprDt s p)

def boolean (s : Term) (p : String) : D Bool := do
  match ← required s p (← typed g s p xsdBoolean) with
  | "true" | "1" => pure true
  | "false" | "0" => pure false
  | v => throw (errAt s p s!"{v} is not a boolean")

def integer (s : Term) (p : String) : D Nat := do
  let v ← required s p (← typed g s p xsdInteger)
  let v := if v.startsWith "+" then String.ofList (v.toList.drop 1) else v
  match v.toNat? with
  | some n => pure n
  | none => throw (errAt s p s!"{v} is not a valid order")

def hasType (s : Term) (cls : String) : Bool := (g.objs.getD (s, rdfType) #[]).contains (vt cls)

/-- The members of an ordered collection by `vt:order`, which must be `0..n-1`. -/
def ordered (s : Term) (p : String) : D (List Term) := do
  let ms ← (← nodes g s p).mapM fun n => do pure (← integer g n "order", n)
  let ms := ms.mergeSort (fun a b => a.1 < b.1 || (a.1 == b.1 && compare (writeTerm a.2) (writeTerm b.2) != .gt))
  for (o, i) in ms.map (·.1) |>.zipIdx do
    if o != i then throw (errAt s p s!"orders are not 0..{ms.length}")
  pure (ms.map (·.2))

def tokens (s : Term) (p : String) : D (List String) := do
  (← ordered g s p).mapM fun t => string g t "value"

def targetIds (s : Term) (p : String) : D (List String) := do
  (← ordered g s p).mapM fun r => do string g (← oneNode g r "state") "id"

def extensionFields (n : Term) : D Extension := do
  let value ← match ← optString g n "attributeValue", ← optString g n "elementXml" with
    | some a, none => pure (ExtensionValue.attribute a)
    | none, some e => pure (ExtensionValue.element e)
    | _, _ => throw (errAt n "extension" "needs exactly one of vt:attributeValue and vt:elementXml")
  let position ← integer g n "position"
  pure { ns := ← string g n "namespace", localName := ← string g n "localName", value, position }

def extensions (s : Term) : D (List Extension) := do
  (← ordered g s "extension").mapM (extensionFields g)

def params (s : Term) : D (List Param) := do
  (← ordered g s "param").mapM fun p => do
    pure { name := ← string g p "paramName", expr := ← optExpr g exprDt p "expr",
           location := ← optExpr g exprDt p "location" }

def content (s : Term) : D (Option Content) := do
  match ← optNode g s "content" with
  | some c => pure (some { expr := ← optExpr g exprDt c "expr", body := ← optString g c "inlineContent" })
  | none => pure none

def script (n : Term) : D Script := do
  pure { src := ← optString g n "src", body := ← optExpr g exprDt n "scriptBody" }

/-- Executable content, nested to depth `fuel` (bounded by the number of triples). -/
def exec : Nat → Term → D Exec
  | 0, n => throw (errAt n "action" "nesting too deep")
  | fuel + 1, n => do
    let block := fun (o : Term) (p : String) => do (← ordered g o p).mapM (exec fuel)
    let kind ←
      if hasType g n "Raise" then pure (ExecKind.raise (← string g n "eventName"))
      else if hasType g n "Send" then pure (ExecKind.send
        { event := ← optString g n "eventName", eventexpr := ← optExpr g exprDt n "eventexpr",
          target := ← optString g n "sendTarget", targetexpr := ← optExpr g exprDt n "targetexpr",
          type := ← optString g n "type", typeexpr := ← optExpr g exprDt n "typeexpr",
          id := ← optString g n "sendId", idlocation := ← optExpr g exprDt n "idlocation",
          delay := ← optString g n "delay", delayexpr := ← optExpr g exprDt n "delayexpr",
          namelist := ← tokens g n "namelist", params := ← params g exprDt n,
          content := ← content g exprDt n })
      else if hasType g n "Cancel" then
        pure (ExecKind.cancel (← optString g n "sendId") (← optExpr g exprDt n "sendidexpr"))
      else if hasType g n "Log" then
        pure (ExecKind.log (← optString g n "label") (← optExpr g exprDt n "expr"))
      else if hasType g n "Assign" then
        pure (ExecKind.assign (← expr g exprDt n "location") (← optExpr g exprDt n "expr")
          (← optString g n "inlineContent"))
      else if hasType g n "Script" then pure (ExecKind.script (← script g exprDt n))
      else if hasType g n "If" then do
        let bs ← (← ordered g n "branch").mapM fun b => do
          pure ((← optExpr g exprDt b "cond"), (← block b "action"))
        pure (ExecKind.if_ bs)
      else if hasType g n "Foreach" then
        pure (ExecKind.foreach (← expr g exprDt n "array") (← string g n "item")
          (← optString g n "index") (← block n "action"))
      else if hasType g n "ExtensionContent" then pure (ExecKind.extension (← extensionFields g n))
      else throw (errAt n "action" "executable content of unknown kind")
    pure (.mk kind (← extensions g n))

def block (fuel : Nat) (o : Term) (p : String) : D (List Exec) := do
  (← ordered g o p).mapM (exec g exprDt fuel)

def transition (fuel : Nat) (n : Term) : D Transition := do
  let type ← match ← string g n "transitionType" with
    | "external" => pure TransitionType.external
    | "internal" => pure TransitionType.internal
    | other => throw (errAt n "transitionType" s!"transition type {other}")
  pure { events := ← tokens g n "event", cond := ← optExpr g exprDt n "cond",
         targets := ← targetIds g n "target", type, typeExplicit := ← boolean g n "transitionTypeExplicit",
         body := ← block g exprDt fuel n "action", extensions := ← extensions g n }

def dataItem (n : Term) : D Data := do
  pure { id := ← string g n "dataId", src := ← optString g n "src",
         expr := ← optExpr g exprDt n "expr", content := ← optString g n "inlineContent",
         extensions := ← extensions g n }

def invoke (fuel : Nat) (n : Term) : D Invoke := do
  pure { type := ← optString g n "type", typeexpr := ← optExpr g exprDt n "typeexpr",
         src := ← optString g n "src", srcexpr := ← optExpr g exprDt n "srcexpr",
         id := ← optString g n "invokeId", idlocation := ← optExpr g exprDt n "idlocation",
         namelist := ← tokens g n "namelist", autoforward := ← boolean g n "autoforward",
         params := ← params g exprDt n, content := ← content g exprDt n,
         finalize := ← block g exprDt fuel n "finalize", extensions := ← extensions g n }

/-- A state and its descendants, to depth `fuel`. -/
def state (fuel : Nat) : Nat → Term → D State
  | 0, n => throw (errAt n "child" "nesting too deep")
  | depth + 1, n => do
    let element ←
      if hasType g n "History" then
        match ← string g n "historyType" with
        | "shallow" => pure (StateElement.history .shallow)
        | "deep" => pure (StateElement.history .deep)
        | other => throw (errAt n "historyType" s!"history type {other}")
      else if hasType g n "Parallel" then pure StateElement.parallel
      else if hasType g n "Final" then pure StateElement.final
      else if hasType g n "State" then pure StateElement.state
      else throw (errAt n "child" "state node without an element class")
    let initEl ← match ← optNode g n "initialElement" with
      | some i => do
        pure (some { transition := ← transition g exprDt fuel (← oneNode g i "initialTransition"),
                     extensions := ← extensions g i : Initial })
      | none => pure none
    let children ← (← ordered g n "child").mapM (state fuel depth)
    let ts ← (← ordered g n "transition").mapM (transition g exprDt fuel)
    let onentry ← (← ordered g n "onentry").mapM fun h => block g exprDt fuel h "action"
    let onexit ← (← ordered g n "onexit").mapM fun h => block g exprDt fuel h "action"
    let data ← (← ordered g n "data").mapM (dataItem g exprDt)
    let invokes ← (← ordered g n "invoke").mapM (invoke g exprDt fuel)
    let dd ← match ← optNode g n "donedata" with
      | some d => pure (some { content := ← content g exprDt d, params := ← params g exprDt d : DoneData })
      | none => pure none
    pure (.mk (← string g n "id") (← boolean g n "idGenerated") element (← targetIds g n "initial")
      initEl children ts onentry onexit data invokes dd (← extensions g n))

end Decode

/-- The base of a chart node IRI `<base>#chart`. -/
def baseOfChartNode (iri : String) : Option String :=
  if iri.endsWith "#chart" then some (String.ofList (iri.toList.take (iri.length - 6))) else none

/-- Decode a chart from a graph, without the closed-world check: the chart and its base. -/
def decode (ts : List Triple) : D (Chart × String) := do
  let g := Index.of ts
  let charts := (ts.filter (fun t => t.p == rdfType && t.o == vt "Chart")).map (·.s) |>.eraseDups
  let cn ← match charts with
    | [n@(Term.iri _)] => pure n
    | _ => throw s!"expected exactly one vt:Chart IRI, found {charts.length}"
  let base ← match cn with
    | .iri i => match baseOfChartNode i.val with
      | some b => pure b
      | none => throw "chart node IRI is not <base>#chart"
    | _ => throw "chart node is not an IRI"
  let datamodel ← Decode.string g cn "datamodel"
  let exprDt := exprDatatype datamodel
  let fuel := ts.length + 1
  let binding ← match ← Decode.string g cn "binding" with
    | "early" => pure Binding.early
    | "late" => pure Binding.late
    | other => throw (Decode.errAt cn "binding" s!"binding {other}")
  let script ← match ← Decode.optNode g cn "script" with
    | some s => pure (some (← Decode.script g exprDt s))
    | none => pure none
  let c : Chart :=
    { name := ← Decode.optString g cn "name", datamodel,
      datamodelExplicit := ← Decode.boolean g cn "datamodelExplicit", binding,
      bindingExplicit := ← Decode.boolean g cn "bindingExplicit",
      initial := ← Decode.targetIds g cn "initial",
      data := ← (← Decode.ordered g cn "data").mapM (Decode.dataItem g exprDt),
      script,
      states := ← (← Decode.ordered g cn "child").mapM (Decode.state g exprDt fuel fuel),
      extensions := ← Decode.extensions g cn }
  pure (c, base)

/-- The closed-world check: every triple of the projection of `c` under `base` is in `g`. -/
def closedWorld (g : Graph) (c : Chart) (base : String) : Bool :=
  (project c base).all (fun t => decide (t ∈ g))

/-- Every triple of `ts` is in `g`. -/
def closedWorld' (g : Graph) (ts : List Triple) : Bool := ts.all (fun t => decide (t ∈ g))

/-- Reconstruction (Rust `reconstruct`, after validation): decode, then require the graph to
contain the whole projection of what was decoded. -/
-- @tg: implements:: [[openspec:formal-model#RDF Round Trip Theorem]] {note: "the round trip is proven: fromRDF_project"}
def fromRDF (ts : List Triple) : D (Chart × String) := do
  let (c, base) ← decode ts
  if closedWorld (Graph.ofList ts) c base then pure (c, base)
  else throw "graph lacks a triple of the projection of the decoded chart"

/-- **Reconstruction is sound for the projection**: whatever `fromRDF` returns, the graph
contains its whole projection. -/
-- @tg: implements:: [[openspec:formal-model#RDF Round Trip Theorem]] {note: "soundness of reconstruction; the identity is fromRDF_project"}
theorem fromRDF_sound (ts : List Triple) (c : Chart) (base : String)
    (h : fromRDF ts = .ok (c, base)) : ∀ t ∈ project c base, t ∈ Graph.ofList ts := by
  unfold fromRDF at h
  cases hd : decode ts with
  | error e => rw [hd] at h; cases h
  | ok v =>
    rcases v with ⟨c', b'⟩
    rw [hd] at h
    simp only [bind, Except.bind] at h
    by_cases hc : closedWorld (Graph.ofList ts) c' b' = true
    · simp only [hc, ↓reduceIte, pure, Except.pure] at h
      cases h
      intro t ht
      have := List.all_eq_true.1 hc t ht
      simpa using this
    · have hf : closedWorld (Graph.ofList ts) c' b' = false := by simpa using hc
      simp only [hf, Bool.false_eq_true, ↓reduceIte, throw, throwThe, MonadExceptOf.throw] at h
      cases h

end Vertep.Rdf
