import VertepTests.WellFormed
import VertepTests.Step

/-!
# RDF projection, reconstruction, hashing, the oracle and certificates

The fixtures in `VertepTests/data` were produced by the Rust core and `vertep-rdf` on IRP tests
144 (`ecmascript`) and 436 (`null`): the published projection (`.nq`), the canonical N-Quads
(`.canon.nq`), the chart version (`.version`) and the oracle records of the run (`.jsonl`).
-/

namespace Vertep.Test

open Vertep.Rdf

/-! ## SHA-256 (FIPS 180-4 vectors) -/

#guard Sha256.hex "" == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
#guard Sha256.hex "abc" == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
#guard Sha256.hex "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq" ==
  "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"

/-! ## Round trip of the Lean projection on the test charts -/

def roundTrips (c : Chart) : Bool :=
  match fromRDF (project c "urn:vertep:chart:sha256:test") with
  | .ok (c', b) => c' == c && b == "urn:vertep:chart:sha256:test"
  | .error _ => false

#guard [initialEntry, descendantWins, exitOrder, historyChart .deep, historyChart .shallow, finals,
  sends, ecmaChart, nullIn, internalTr .internal, wfChart, parallelTargets].all roundTrips

/-- Reconstruction after projection is the identity on every well-formed chart, under any base
(the RDF Round Trip Theorem, used on a test chart the decision procedure accepts). -/
-- @tg: verifies:: [[openspec:formal-model#RDF Round Trip Theorem#Round trip]] {level: unit}
example (base : String) : fromRDF (project wfChart base) = .ok (wfChart, base) :=
  fromRDF_project wfChart ((checkWF_iff wfChart).1 (by decide)) base

/-- The canonical form is a function of the chart. -/
-- @tg: verifies:: [[openspec:formal-model#RDF Round Trip Theorem#Canonical form determined by the chart]] {level: unit}
theorem canonical_form_determined (c₁ c₂ : Chart) (h : c₁ = c₂) :
    canonicalNQuads c₁ = canonicalNQuads c₂ ∧ chartVersion c₁ = chartVersion c₂ :=
  ⟨canonical_determined c₁ c₂ h, (version_determined c₁ c₂ h).1⟩

/-! ## Agreement with `vertep-rdf` on the fixtures -/

def fixture (name : String) : IO (String × String × String × String) := do
  pure (← IO.FS.readFile s!"VertepTests/data/{name}.nq",
        ← IO.FS.readFile s!"VertepTests/data/{name}.canon.nq",
        ← IO.FS.readFile s!"VertepTests/data/{name}.version",
        ← IO.FS.readFile s!"VertepTests/data/{name}.jsonl")

/-- Decoding the Rust projection, re-projecting in Lean and hashing gives Rust's canonical
N-Quads and version IRI, and the Lean projection decodes back to the same chart. -/
def rdfAgrees (name : String) : IO Bool := do
  let (nq, canon, version, _) ← fixture name
  match Oracle.loadChart nq with
  | .error _ => pure false
  | .ok (c, _) => pure (canonicalNQuads c == canon && chartVersion c == version.trimAscii.toString &&
      roundTrips c)

#eval show IO Unit from do
  for n in ["144-ecmascript-0", "436-null-0"] do
    unless (← rdfAgrees n) do throw (IO.userError s!"RDF disagreement on {n}")

-- The shapes embedded in Lean are the crate's shapes file.
#eval show IO Unit from do
  let crate ← IO.FS.readFile "../../crates/vertep-rdf/shapes/vt-chart-shapes.ttl"
  unless crate == chartShapesTtl do
    throw (IO.userError "VertepLean/Rdf/Shapes.lean is out of date with vt-chart-shapes.ttl")

/-! ## The differential oracle -/

/-- On a chart and the Rust run's records, the oracle writes one trace record per macrostep,
with the configuration, the effects and (checked against the tape) the datamodel requests, and
they equal the Rust traces. -/
-- @tg: verifies:: [[openspec:formal-model#Differential Oracle#Oracle trace]] {level: integration}
def oracleTrace (name : String) : IO Bool := do
  let (nq, _, _, jsonl) ← fixture name
  match Oracle.loadChart nq, Oracle.parseRecords jsonl with
  | .ok (_, cc), .ok rs =>
    let out := Oracle.replayAll cc rs
    pure (out.length == rs.length && rs.length > 0 && (Oracle.firstDifference out).isNone)
  | _, _ => pure false

#eval show IO Unit from do
  for n in ["144-ecmascript-0", "436-null-0"] do
    unless (← oracleTrace n) do throw (IO.userError s!"oracle disagreement on {n}")

/-- A record whose trace differs from the Lean trace is reported as the first difference. -/
-- @tg: verifies:: [[openspec:formal-model#Differential Oracle#Trace mismatch detected]] {level: integration}
def mismatchDetected : IO Bool := do
  let (nq, _, _, jsonl) ← fixture "144-ecmascript-0"
  match Oracle.loadChart nq, Oracle.parseRecords jsonl with
  | .ok (_, cc), .ok (r :: rs) =>
    -- the recorded trace claims the session did not terminate
    let r' := { r with trace := { r.trace with terminated := !r.trace.terminated } }
    pure ((Oracle.firstDifference (Oracle.replayAll cc (r' :: rs))).map (·.1) == some 0)
  | _, _ => pure false

#eval show IO Unit from do
  unless (← mismatchDetected) do throw (IO.userError "a trace mismatch went unreported")

/-! ## Certificates -/

/-- A well-formed chart's graph gets a certificate whose subject is its version IRI, listing
the toolchain, the LeanRDF revision and the checker hash. -/
-- @tg: verifies:: [[openspec:formal-model#Chart Certificates#Certificate for a well-formed chart]] {level: integration}
def certificateIssued : IO Bool := do
  let (nq, _, version, _) ← fixture "144-ecmascript-0"
  match Certify.check nq with
  | .certified v =>
    let cert := Certify.certificate v "00" none
    pure (v == version.trimAscii.toString &&
      cert.startsWith s!"<{v}> <https://w3id.org/vertep/vt#certifiedBy> " &&
      (cert.splitOn Certify.leanToolchain).length > 1 &&
      (cert.splitOn Certify.leanRdfRevision).length > 1 && (cert.splitOn "sha256:00").length > 1)
  | _ => pure false

#eval show IO Unit from do
  unless (← certificateIssued) do throw (IO.userError "no certificate for a well-formed chart")

/-- The graph of an ill-formed chart gets no certificate, and the violated rule is named; a graph
that is not a chart projection is refused. -/
-- @tg: verifies:: [[openspec:formal-model#Chart Certificates#No certificate for an ill-formed chart]] {level: unit}
def noCertificate : Bool :=
  (match Certify.check (publishNQuads badTargets) with
   | .illFormed _ vs => vs.any (·.startsWith "state rules")
   | _ => false) &&
  (match Certify.check (publishNQuads historyInitial) with
   | .illFormed _ vs => vs.contains "HistoryDefaultCycle"
   | _ => false) &&
  (match Certify.check "<urn:x> <urn:y> <urn:z> .\n" with
   | .notAChart _ => true
   | _ => false)

#guard noCertificate

/-! ## Typed charts -/

open Vertep.Types in
/-- The declaration syntax of `vt:types` (Rust `parse_declarations`). -/
def parsesTo (src : String) (expected : Decls) : Bool :=
  match Types.Syntax.parseDeclarations src with
  | .ok b => reprStr b.types == reprStr expected
  | .error _ => false

def rejects (src : String) : Bool :=
  match Types.Syntax.parseDeclarations src with
  | .ok _ => false
  | .error _ => true

#guard parsesTo "type Pay = { amount: decimal };\nevent pay: Pay; event start"
  [("Pay", .record [("amount", .prim .decimal)])]
#guard parsesTo "# a comment\ntype S = | \"a b\" { \"\\u{fc}\": list<optional<S>> } | Done;"
  [("S", .union [("a b", some (.record [("ü", .list (.optional (.named "S")))])), ("Done", none)])]
#guard parsesTo "type T = Card { last4: string } | Cash; type U = (T);"
  [("T", .union [("Card", some (.record [("last4", .prim .string)])), ("Cash", none)]),
   ("U", .named "T")]
#guard parsesTo "type R = {a: int, b: value,}" [("R", .record [("a", .prim .int), ("b", .value)])]
#guard parsesTo "event \"a b\": { x: int }; event ns.e" []
#guard rejects "type list = int;"
#guard rejects "type A = X | int;"
#guard rejects "type A = int | X;"
#guard rejects "type A = B<int>;"
#guard rejects "type A = int type B = int;"
#guard rejects "type A = \"\\q\";"
#guard rejects "type A = { a int };"
#guard rejects "event ;"

-- The text of a `vt:types` element: entities, CDATA, attributes holding `>`, empty elements.
#guard (Types.Syntax.typesElementText "<vt:types a='x>y'>type A = int;&#10;&lt;&#x3E;</vt:types>").toOption ==
  some (some "type A = int;\n<>")
#guard (Types.Syntax.typesElementText " <vt:types><![CDATA[type A = list<int>;]]> &amp;</vt:types>\n").toOption ==
  some (some "type A = list<int>; &")
#guard (Types.Syntax.typesElementText "<vt:types/>").toOption == some none
#guard (Types.Syntax.typesElementText "<vt:types>a<b/></vt:types>").toOption.isNone
#guard (Types.Syntax.typesElementText "<vt:types>&bogus;</vt:types>").toOption.isNone

-- Type IRIs are the Rust ones (`crates/vertep-dm-rust/tests/charts/order.scxml`).
#guard (typeIris [("Pay", .record [("amount", .prim .decimal)])]).lookup "Pay" ==
  some "urn:vertep:type:sha256:41b1c80782c0edf4c066dd3ac0c6235285690642f6ec20b1bb77e8e7e1bd632f"
-- Unknown names leave a type, and every type using it, without an IRI; a recursive group
-- through a list gets one IRI per member.
#guard (typeIris [("A", .named "Missing"), ("B", .list (.named "A")), ("C", .prim .int)]).map (·.1) ==
  ["C"]
#guard (typeIris [("A", .list (.named "B")), ("B", .optional (.named "A"))]).length == 2

/-- The typed `order` chart as Rust publishes it: decoding and re-projecting in Lean, with the
type triples recomputed from its `vt:types` block, gives Rust's canonical N-Quads and version,
and the graph is certified under that version. -/
-- @tg: verifies:: [[openspec:formal-model#Chart Certificates#Typed chart certified]] {level: integration}
def typedChartCertified : IO Bool := do
  let nq ← IO.FS.readFile "VertepTests/data/order-rust.nq"
  let canon ← IO.FS.readFile "VertepTests/data/order-rust.canon.nq"
  let version := (← IO.FS.readFile "VertepTests/data/order-rust.version").trimAscii.toString
  match Oracle.loadChart nq with
  | .error _ => pure false
  | .ok (c, _) =>
    let certified := match Certify.check nq with
      | .certified v => v == version
      | _ => false
    pure (canonicalNQuads c == canon && chartVersion c == version && certified &&
      !(chartTypes c version).isEmpty)

#eval show IO Unit from do
  unless (← typedChartCertified) do throw (IO.userError "the typed order chart is not certified")

/-- A graph missing a type triple of the chart's `vt:types` block (a declaration or a shape) is
not the projection of the chart, and gets no certificate. -/
def typeTripleMissing : IO Bool := do
  let nq ← IO.FS.readFile "VertepTests/data/order-rust.nq"
  let lines := nq.splitOn "\n"
  let without := fun (s : String) =>
    let ls := lines.filter fun l => (l.splitOn s).length ≤ 1
    (decide (ls.length < lines.length), String.intercalate "\n" ls)
  let refused := fun (s : String) =>
    let (dropped, src) := without s
    dropped && match Certify.check src with
      | .notAChart _ => true
      | _ => false
  pure (refused "declaresType" && refused "#s.f.amount> <http://www.w3.org/ns/shacl#path>")

#eval show IO Unit from do
  unless (← typeTripleMissing) do throw (IO.userError "a graph without its type triples was certified")

-- The LeanRDF revision a certificate names is the one the manifest pins.
#eval show IO Unit from do
  let man ← IO.FS.readFile "lake-manifest.json"
  unless (man.splitOn s!"\"rev\": \"{Certify.leanRdfRevision}\"").length > 1 do
    throw (IO.userError "Certify.leanRdfRevision differs from the pinned LeanRDF revision")

end Vertep.Test
