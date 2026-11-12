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
    ``Vertep.Rdf.version_determined, ``Vertep.Certify.check_certified_wellFormed,
    ``Vertep.Rdf.RT.percentEncode_inj, ``Vertep.Rdf.RT.ordered_eq, ``Vertep.Rdf.RT.Cap.kid,
    ``Vertep.Rdf.RT.dec_exec, ``Vertep.Rdf.RT.dec_transition,
    ``Vertep.Rdf.RT.dec_state, ``Vertep.Rdf.RT.decode_project, ``Vertep.Rdf.fromRDF_project,
    ``Vertep.Journal.replay_determinism, ``Vertep.Journal.replay_no_divergence,
    ``Vertep.Journal.replay_equals_live, ``Vertep.Journal.turn_idempotent,
    ``Vertep.Journal.accepted_once, ``Vertep.Journal.outbox_safe,
    ``Vertep.Journal.outbox_complete, ``Vertep.Journal.no_message_lost,
    ``Vertep.Journal.cas_single, ``Vertep.Journal.turn_cas_exclusive,
    ``Vertep.Journal.accepts_iff, ``Vertep.Journal.reachable_admissible,
    ``Vertep.Agent.decider_safety, ``Vertep.Agent.stale_not_taken,
    ``Vertep.Agent.unknown_not_taken, ``Vertep.Agent.decision_dedup,
    ``Vertep.Agent.decision_once, ``Vertep.Agent.replay_live, ``Vertep.Agent.chainGrant_mem,
    ``Vertep.Agent.grants_do_not_escalate, ``Vertep.Agent.grant_enforced,
    ``Vertep.Agent.ungranted_not_delivered, ``Vertep.Agent.granted_delivered,
    ``Vertep.Lease.grant_unique, ``Vertep.Lease.accepted_granted,
    ``Vertep.Lease.one_writer_per_epoch, ``Vertep.Lease.journal_epochs_ordered,
    ``Vertep.Lease.shard_epochs_ordered, ``Vertep.Lease.message_once, ``Vertep.Lease.stale_append_fenced,
    ``Vertep.Lease.frames_ordered, ``Vertep.Lease.stale_frame_dropped,
    ``Vertep.Custody.quorum_inter, ``Vertep.Custody.inv_of_reachable,
    ``Vertep.Custody.one_writer_per_epoch, ``Vertep.Custody.one_adoption_per_epoch,
    ``Vertep.Custody.committed_survives_election, ``Vertep.Custody.committed_survives_accept,
    ``Vertep.Custody.committed_in_custodian, ``Vertep.Custody.committed_in_writer_log,
    ``Vertep.Custody.Check.adoptsB_sound,
    ``Vertep.Custody.Joint.rquorum_inter, ``Vertep.Custody.Joint.exec_noChange_project,
    ``Vertep.Custody.Joint.reachableNC_project, ``Vertep.Custody.Joint.one_writer_per_epoch_noChange,
    ``Vertep.Custody.Joint.committed_survives_noChange,
    ``Vertep.Custody.Unsafe.rust_switch_loses_committed,
    ``Vertep.Custody.Unsafe.rust_pending_loses_committed,
    ``Vertep.Custody.Unsafe.rust_pending_two_writers,
    ``Vertep.Types.check_iff, ``Vertep.Types.check_sound, ``Vertep.Types.Chain.dedup,
    ``Vertep.Types.JsonSchema.validate_proj, ``Vertep.Types.JsonSchema.jsonSchema_agrees,
    ``Vertep.Types.Shacl.shacl_agrees, ``Vertep.Types.Shacl.validator_agrees,
    ``Vertep.Types.Shacl.conforms_mono, ``Vertep.Types.Shacl.agrees_all,
    ``Vertep.Types.Shacl.checkF_sound, ``Vertep.Types.Shacl.checkF_complete,
    ``Vertep.Types.Shacl.enc_encGraph,
    ``Vertep.CWF.legal_iff_below, ``Vertep.eff_adequate, ``Vertep.start_entry,
    ``Vertep.exit_legal, ``Vertep.Config.microstep_spec, ``Vertep.Config.reachable_inv,
    ``Vertep.configuration_wellformedness_of, ``Vertep.compile_cwf,
    ``Vertep.configuration_wellformedness_of_entry, ``Vertep.entry_legal,
    ``Vertep.configuration_wellformedness, ``Vertep.compound_exclusivity,
    ``Vertep.parallel_completeness,
    ``Vertep.Typed.typed_step_soundness, ``Vertep.Typed.typed_start_soundness,
    ``Vertep.Typed.step_typed, ``Vertep.Typed.stepFull_typed, ``Vertep.Typed.start_typed,
    ``Vertep.Typed.startFull_typed, ``Vertep.Typed.datamodel_typed,
    ``Vertep.Typed.vertep_typed_soundness,
    ``Vertep.Rdf.Equiv.nodeShapes_iff, ``Vertep.Rdf.Equiv.shapes_equiv_nodes,
    ``Vertep.Rdf.Equiv.wellFormed_iff, ``Vertep.Rdf.Equiv.uniqueStateId_ok,
    ``Vertep.Rdf.Equiv.goodAll_iff, ``Vertep.Rdf.Equiv.reg_exec, ``Vertep.Rdf.Equiv.reg_state,
    ``Vertep.Rdf.Equiv.mem_descIds_iff,
    ``Vertep.Rdf.Equiv.shapes_equivalence, ``Vertep.Rdf.Equiv.shapes_forward,
    ``Vertep.Rdf.Equiv.shapes_backward, ``Vertep.Rdf.Equiv.conforms_nodup,
    ``Vertep.Rdf.Equiv.selection_iff, ``Vertep.Rdf.Equiv.cycle_iff, ``Vertep.Rdf.Equiv.dataId_iff,
    ``Vertep.Rdf.Equiv.acyclic_iff, ``Vertep.Rdf.Equiv.hd_iff, ``Vertep.Rdf.Equiv.pairOk_iff,
    ``Vertep.Rdf.Equiv.compat_iff, ``Vertep.Rdf.Equiv.agree_iff_tso, ``Vertep.Rdf.Equiv.wp_prefix,
    ``Vertep.Rdf.Equiv.chain_func ]

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
