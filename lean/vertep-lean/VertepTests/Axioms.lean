import Lean
import VertepLean

/-!
# Axiom hygiene

Pins the axioms of every main theorem at build time: `#check_axioms` collects the axioms each
theorem depends on, prints them (the `#print axioms` report), and fails the build if anything
beyond `propext`, `Classical.choice` and `Quot.sound` appears. A `sorry` shows up as the axiom
`sorryAx`, so it fails here too, in addition to `warningAsError` on the library and the
`scripts/check.sh` source scan.
-/

open Lean Elab Command

namespace Vertep.AxiomCheck

/-- The only axioms the library may depend on. -/
-- @tg: implements:: [[openspec:formal-model#Axiom Hygiene]]
def allowedAxioms : List Name := [``propext, ``Classical.choice, ``Quot.sound]

/-- The main theorems whose axioms are pinned. -/
def mainTheorems : List Name :=
  [ ``Vertep.checkWF_iff, ``Vertep.execOkB_iff,
    ``Vertep.step_deterministic, ``Vertep.step_deterministic_out, ``Vertep.start_deterministic,
    ``Vertep.Select.choices_run, ``Vertep.Select.selected_enabled,
    ``Vertep.Select.selected_conflictFree, ``Vertep.Select.selected_optimal,
    ``Vertep.Select.selected_nodup, ``Vertep.select_spec, ``Vertep.transition_selection,
    ``Vertep.step_requests_declared, ``Vertep.start_requests_declared,
    ``Vertep.dynamic_typed_soundness, ``Vertep.violations_eq_nil_iff,
    ``Vertep.Rdf.fromRDF_sound, ``Vertep.Rdf.canonical_determined,
    ``Vertep.Rdf.version_determined, ``Vertep.Certify.check_certified_wellFormed ]

/-- The axioms of `n` outside the allowed set. -/
def disallowed (n : Name) : CoreM (Array Name) := do
  let axs ← collectAxioms n
  pure (axs.filter (fun a => !allowedAxioms.contains a))

/-- Print the axioms of every main theorem and fail on any outside the allowed set. -/
-- @tg: implements:: [[openspec:formal-model#Axiom Hygiene]]
elab "#check_axioms" : command => do
  for n in mainTheorems do
    let axs ← liftCoreM (collectAxioms n)
    logInfo m!"'{n}' depends on axioms: {axs.toList}"
    let bad ← liftCoreM (disallowed n)
    unless bad.isEmpty do
      throwError "'{n}' depends on disallowed axioms {bad.toList}"

/-- Fail unless `n` is rejected by the gate (used to test the gate on a `sorry`). -/
elab "#expect_rejected " n:ident : command => do
  let name ← liftCoreM (realizeGlobalConstNoOverload n)
  let bad ← liftCoreM (disallowed name)
  if bad.isEmpty then throwError "the axiom gate accepted '{name}'"
  logInfo m!"the axiom gate rejects '{name}': {bad.toList}"

end Vertep.AxiomCheck

-- @tg: verifies:: [[openspec:formal-model#Axiom Hygiene#Axioms pinned]] {level: unit}
#check_axioms

set_option warningAsError false in
/-- A deliberately unproven statement, outside the library, to test the gate. -/
theorem Vertep.AxiomCheck.sorryProbe : 1 = 2 := by sorry

-- @tg: verifies:: [[openspec:formal-model#Axiom Hygiene#Sorry rejected]] {level: unit}
#expect_rejected Vertep.AxiomCheck.sorryProbe
