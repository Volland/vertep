import Lake

open Lake DSL

/-- The Lean 4 formal model of Vertep's SCXML semantics: the chart IR, `WellFormed` with a
decision procedure proven correct, the executable `step` of W3C SCXML Appendix D with the
datamodel abstracted as an answer tape, and the M0 theorems. -/
-- @tg: implements:: [[openspec:formal-model#Lean Package]]
package «vertep-lean» where
  version := v!"0.1.0"
  description := "Formal model of Vertep's SCXML semantics, checked against the Rust core."
  license := "MIT OR Apache-2.0"
  leanOptions := #[⟨`autoImplicit, false⟩]

/-- LeanRDF (RDF 1.2 data model), pinned to an exact commit on GitHub. -/
require leanrdf from git "https://github.com/Volland/rdf-lean" @ "b407b97424150fec25a1ade514c71ea7abc61b42"

/-- LeanRDF's SHACL Core package (the `shacl/` directory of the same repository and commit), for
the Shapes Equivalence Theorem of the second phase. -/
require «leanrdf-shacl» from git "https://github.com/Volland/rdf-lean" @ "b407b97424150fec25a1ade514c71ea7abc61b42" / "shacl"

/-- The model and its theorems. Warnings are errors, so a `sorry` (reported as a warning by
Lean) fails the build. -/
-- @tg: implements:: [[openspec:formal-model#Axiom Hygiene]]
@[default_target] lean_lib VertepLean where
  leanOptions := #[⟨`autoImplicit, false⟩, ⟨`warningAsError, true⟩]

/-- Tests, checked at build time (`#guard`, `example`, `#check_axioms`): `lake build
VertepTests`. `VertepTests.Axioms` pins the axioms of every main theorem. -/
@[default_target] lean_lib VertepTests where
  globs := #[.andSubmodules `VertepTests]

/-- The differential oracle (formal-model, *Differential Oracle*):
`lake exe vertep-oracle --chart CHART.nq --records RECORDS.jsonl [--out TRACES.jsonl] [--compare]`. -/
lean_exe «vertep-oracle» where
  root := `VertepOracleMain

/-- The chart certifier behind `vt certify` (formal-model, *Chart Certificates*):
`lake exe vt-certify --chart CHART.nq [--out CERT.nt] [--time DATETIME]`. -/
lean_exe «vt-certify» where
  root := `VertepCertifyMain

/-- The journal protocol model as a test oracle (verification-backlog 5.6):
`lake exe vertep-journal --obs OBS.json`. -/
lean_exe «vertep-journal» where
  root := `VertepJournalMain

/-- The lease and fencing model as a differential oracle for fault-injection histories
(m4-cluster 6.5): `lake exe vertep-lease --history HISTORY.jsonl`. -/
lean_exe «vertep-lease» where
  root := `VertepLeaseMain

/-- Decision validation as a differential oracle for agent runs (m3-agents 8.5):
`lake exe vertep-decide --obs DECISIONS.jsonl`. -/
lean_exe «vertep-decide» where
  root := `VertepDecideMain

/-- Empirical agreement of the chart shapes (LeanRDF SHACL Core) with `WellFormed`:
`lake exe vertep-shapes FILE.nq…`. -/
lean_exe «vertep-shapes» where
  root := `VertepShapesMain
