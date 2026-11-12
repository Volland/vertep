import VertepLean.Config.Tree
import VertepLean.Config.CompileRank

/-!
# Compile correctness

`compile` of a well-formed chart satisfies `CWF`. The proof reads the loops of `compile` as
invariants (`VertepLean/Config/CompileLoop.lean`, summarized by `Spec`), relates compiled
positions to IR states in preorder (`CompileIR`, `CompileTree`), proves the structural fields
(`CompileFields`), transports legal IR target sets and history defaults (`CompileLegal`), and
builds the history rank from `HistoryDefaultsAcyclic` by pigeonhole (`CompileRank`).
-/

namespace Vertep

open CompileProof

/-- `compile` establishes `CWF` (compile correctness). -/
-- @tg: implements:: [[openspec:formal-model#Configuration Well-Formedness Theorem]] {note: "compile correctness"}
theorem compile_cwf : CompileCWF := by
  intro c cc hwf h
  have hsp := compile_spec hwf h
  exact {
    size_pos := by rw [hsp.size]; omega
    root_kind := st_zero_kind hsp
    root_parent := st_zero_parent hsp
    parent_lt := parent_lt'' hwf hsp
    kind_ne_root := kind_ne_root' hwf hsp
    children_iff := children_iff' hwf hsp
    histories_iff := histories_iff' hwf hsp
    children_nodup := children_nodup' hwf hsp
    histories_nodup := histories_nodup' hwf hsp
    root_children := root_children' hwf hsp
    compound_children := compound_children' hwf hsp
    leaf := leaf' hwf hsp
    history_parent := history_parent' hwf hsp
    trans_source := fun i t ht => (trans_info hwf hsp i t ht).1
    trans_not_default := fun i t ht => (trans_info hwf hsp i t ht).2
    targets_lt := fun t x hx => good_targets_lt hwf hsp t x hx
    targets_legal := targets_legal' hwf hsp
    initial_iff := initial_iff' hwf hsp
    initial_ok := initial_ok' hwf hsp
    history_default := history_default' hwf hsp
    history_rank := history_rank' hwf hsp }

end Vertep
