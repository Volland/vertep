import VertepLean.Oracle
import VertepLean.Theorems

/-!
# `vt certify`

A Lean-compiled checker (formal-model, *Chart Certificates*): given a chart as N-Quads of its
projection, it decodes it with `Rdf.fromRDF` (whose result the graph provably contains,
`Rdf.fromRDF_sound`), decides `WellFormed` (`checkWF_iff`, with the violated rules named by
`violations`, `violations_eq_nil_iff`), recomputes the chart version IRI from the canonical
N-Quads, and, when the chart is well-formed, writes a certificate as N-Triples. A typed chart's
`vt:types` block is read back too (`Rdf.chartDecls`): the type triples it projects to
(`Rdf.chartTypes`: `vt:declaresType`, definitions and SHACL shapes under content-addressed type
IRIs) must be in the graph, and they take part in the version as in Rust:

```
<urn:vertep:chart:sha256:H> vt:certifiedBy <urn:vertep:cert:sha256:C> .
<urn:vertep:cert:sha256:C> a vt:Certificate ;
    vt:theorem "Vertep.checkWF_iff", … ;
    vt:leanToolchain "leanprover/lean4:v4.35.0" ;
    vt:leanRdfRevision "b407…" ;
    vt:checkerHash "sha256:…" ;
    prov:generatedAtTime "…"^^xsd:dateTime .     (only with --time)
```

`C` is the SHA-256 of the certificate's other lines, so equal inputs give equal certificates.

```
vt-certify --chart CHART.nq [--out CERT.nt] [--time 2026-11-11T12:00:00Z] [--checker-hash HEX]
```

The checker hash is the SHA-256 of the running binary (about 2 s for the 100 MB executable);
`--checker-hash` lets a harness that already hashed the binary pass it in.

Exit codes: `0` certificate written; `1` the chart is not well-formed (no certificate; the
violated rules on standard error); `2` the graph is not the projection of a chart, or its node
IRIs are minted under another chart's version; `3` usage or I/O error.
-/

namespace Vertep.Certify

open LeanRDF

/-- The pinned LeanRDF revision (checked against `lake-manifest.json` by the tests). -/
def leanRdfRevision : String := "b407b97424150fec25a1ade514c71ea7abc61b42"

/-- The Lean toolchain the checker was built with. -/
def leanToolchain : String := "leanprover/lean4:v" ++ Lean.versionString

/-- The theorems a certificate relies on: the decision procedure is correct, the decoded chart's
projection is in the graph, and the M0 step theorems that hold for every well-formed chart. -/
def theorems : List String :=
  [ "Vertep.checkWF_iff", "Vertep.violations_eq_nil_iff", "Vertep.Rdf.fromRDF_sound",
    "Vertep.step_deterministic", "Vertep.transition_selection",
    "Vertep.step_requests_declared" ]

def certTerm (s : String) : String := "\"" ++ Rdf.escapeLiteral s ++ "\""

/-- The certificate as N-Triples, for a chart version and a checker hash. -/
def certificate (version checkerHash : String) (time : Option String) : String :=
  let vt := fun (l : String) => "<" ++ Rdf.vtNs ++ l ++ ">"
  let body := fun (cert : String) =>
    [s!"<{cert}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> {vt "Certificate"} .\n"] ++
    theorems.map (fun t => s!"<{cert}> {vt "theorem"} {certTerm t} .\n") ++
    [s!"<{cert}> {vt "leanToolchain"} {certTerm leanToolchain} .\n",
     s!"<{cert}> {vt "leanRdfRevision"} {certTerm leanRdfRevision} .\n",
     s!"<{cert}> {vt "checkerHash"} {certTerm ("sha256:" ++ checkerHash)} .\n"] ++
    (match time with
     | some t => [s!"<{cert}> <http://www.w3.org/ns/prov#generatedAtTime> {certTerm t}^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n"]
     | none => [])
  let id := "urn:vertep:cert:sha256:" ++ Sha256.hex (version ++ "\n" ++ String.join (body "_"))
  s!"<{version}> {vt "certifiedBy"} <{id}> .\n" ++ String.join (body id)

/-- What certification concludes about a chart graph. -/
inductive Outcome where
  | certified (version : String)
  | illFormed (version : String) (violations : List String)
  | notAChart (reason : String)

/-- Decode a graph and decide whether it can be certified. -/
def check (src : String) : Outcome :=
  match LeanRDF.NQuads.parse src with
  | .error e => .notAChart s!"N-Quads: {e}"
  | .ok ds =>
    match Rdf.fromRDF (ds.toList.map (·.triple)) with
    | .error e => .notAChart e
    | .ok (c, base) =>
      let version := Rdf.chartVersion c
      if base != version && base != Rdf.placeholderBase then
        .notAChart s!"node IRIs are minted under <{base}>, but the chart's version is <{version}>"
      else if !Rdf.closedWorld' (Graph.ofList (ds.toList.map (·.triple))) (Rdf.chartTypes c base) then
        .notAChart "graph lacks a type triple (declaration, definition or shape) of the chart's vt:types block"
      else match violations c with
        | [] => .certified version
        | vs => .illFormed version vs

/-- A certified chart is well-formed, its version is the certificate's subject, and the graph
holds its whole projection: the chart triples and the type triples of its `vt:types` block. -/
-- @tg: implements:: [[openspec:formal-model#Chart Certificates]]
theorem check_certified_wellFormed (src : String) (v : String) (h : check src = .certified v) :
    ∃ ds c base, LeanRDF.NQuads.parse src = .ok ds ∧
      Rdf.fromRDF (ds.toList.map (·.triple)) = .ok (c, base) ∧ WellFormed c ∧
      v = Rdf.chartVersion c ∧
      ∀ t ∈ Rdf.projectFull c base, t ∈ Graph.ofList (ds.toList.map (·.triple)) := by
  unfold check at h
  cases hp : LeanRDF.NQuads.parse src with
  | error e => rw [hp] at h; cases h
  | ok ds =>
    rw [hp] at h
    simp only at h
    cases hr : Rdf.fromRDF (ds.toList.map (·.triple)) with
    | error e => rw [hr] at h; cases h
    | ok r =>
      rcases r with ⟨c, base⟩
      rw [hr] at h
      simp only at h
      split at h
      · cases h
      · split at h
        · cases h
        · rename_i hct
          cases hv : violations c with
          | nil =>
            rw [hv] at h
            cases h
            refine ⟨ds, c, base, rfl, hr, (violations_eq_nil_iff c).1 hv, rfl, ?_⟩
            intro t ht
            rcases List.mem_append.1 ht with ht | ht
            · exact Rdf.fromRDF_sound _ c base hr t ht
            · have hc : Rdf.closedWorld' (Graph.ofList (ds.toList.map (·.triple)))
                  (Rdf.chartTypes c base) = true := by simpa using hct
              simpa using List.all_eq_true.1 hc t ht
          | cons x xs => rw [hv] at h; cases h

/-- The command line. -/
-- @tg: implements:: [[openspec:formal-model#Chart Certificates]]
def run (args : List String) : IO UInt32 := do
  let rec opt : List String → String → Option String
    | k :: v :: rest, key => if k == key then some v else opt (v :: rest) key
    | _, _ => none
  let some chartPath := opt args "--chart"
    | IO.eprintln "usage: vt-certify --chart CHART.nq [--out CERT.nt] [--time DATETIME] [--checker-hash HEX]"; return 3
  let src ← IO.FS.readFile chartPath
  match check src with
  | .notAChart r =>
    IO.eprintln s!"not the projection of a chart: {r}"
    return 2
  | .illFormed v vs =>
    IO.eprintln s!"<{v}> is not well-formed; violated rules: {String.intercalate "; " vs}"
    return 1
  | .certified v =>
    let hash ← match opt args "--checker-hash" with
      | some h => pure h
      | none => do pure (Sha256.hexOfBytes (← IO.FS.readBinFile (← IO.appPath)))
    let cert := certificate v hash (opt args "--time")
    match opt args "--out" with
    | some p => IO.FS.writeFile p cert
    | none => IO.print cert
    return 0

end Vertep.Certify
