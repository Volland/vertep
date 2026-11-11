import VertepLean

/-!
# PENDING: Configuration Well-Formedness Theorem (formal-model, task 7.5)

Not part of the library, the build or the axiom gate (`lakefile.lean` builds only `VertepLean`
and `VertepTests`). This file only *states* the theorem, as a `Prop`; nothing here is proven
and nothing here is assumed by the library. Check that it elaborates with
`lake env lean Pending/ConfigWF.lean`.

## Status: false under the current well-formedness rules

The statement below does not hold for every chart that `WellFormed` (and the Rust validator,
which it mirrors) accepts. Two counterexamples, both run by the executable `step`:

1. *A history target next to a sibling target.* `check_target_set` skips history targets, so
   `<transition target="h a2">` with `h` a history child of compound `a` and `a2` a child of `a`
   is accepted. Entering it enters the history default (`a1`) and `a2`: compound `a` ends with two
   active children (`["a", "a1", "a2"]`).
2. *An `initial` attribute naming a history state without a default transition.* The
   synthesized default of such a history state is its parent's initial target, i.e. itself, so
   `addDescendantStatesToEnter` never terminates (Rust recurses without bound; Lean runs out of
   fuel and fails with `illFormed`).

Both are fixed by extending the target-set rule to history targets (a history target counts as
a target at its parent's position, so it conflicts with any other target under that parent that
is not in a different region of a parallel state) and by rejecting `initial` targets (attribute
or `<initial>`) that are history states with no default transition. With those two rules added
to `WellFormed` (and to `validate.rs`), the statement is expected to hold.

## Missing lemmas for the proof

* `compile` correctness on well-formed charts: `parent i < i`, `children`/`histories` are exactly
  the indices whose parent is `i`, `isDesc` agrees with the preorder interval, kinds match, every
  initial/default/transition target is resolved, and the synthesized history defaults are legal.
* Exit lemma: for a conflict-free selected set (from `transition_selection`), the exit set of the
  set is the union of the subtrees strictly below each domain, and those subtrees are pairwise
  nested-or-disjoint, so `config \ exitSet` is legal except below each domain, whose domain is
  active with no active descendant.
* Entry lemma: `entrySet` restricted to each domain is a legal completion below it (for compound
  domains exactly one child chain, for parallel ancestors all regions), using the target-set rule,
  the initial and history-default rules, and the history invariant below.
* History invariant: every recorded value of a shallow history is one child (compound parent) or
  all regions (parallel parent) of its parent, and of a deep history the atomic states of a legal
  sub-configuration below the parent; preserved by `recordHistory`.
* Fuel adequacy: `entryFuel`, `effectiveTargets` and `inFinal` fuel bounds are never reached on
  well-formed charts (depth argument).
* Interpreter invariants through `exitStates`, `enterStates`, `microstep`, `macroLoop`,
  `runExternal`, `runStart` and `exitInterpreter`; executable content cannot change the
  configuration (it runs in `EM`, which only reads it), so only these `M`-level functions matter.
-/

namespace Vertep.Pending

/-- A legal configuration (W3C SCXML 3.11): the root (implicit: Rust and Lean never store it) is
active; active states are proper, non-history states of the chart; every active state's parent is
active or the root; the root and every active compound state have exactly one active child; every
active parallel state has all of its children active. -/
def Legal (cc : CChart) (config : List Nat) : Prop :=
  (∀ s ∈ config, s ≠ 0 ∧ s < cc.size ∧ cc.isHistory s = false) ∧
  (∀ s ∈ config, ∀ p, (cc.st s).parent = some p → p = 0 ∨ p ∈ config) ∧
  ((cc.st 0).children.filter (· ∈ config)).length = 1 ∧
  (∀ s ∈ config, cc.isCompound s = true →
    ((cc.st s).children.filter (· ∈ config)).length = 1) ∧
  (∀ s ∈ config, cc.isParallel s = true → ∀ c ∈ (cc.st s).children, c ∈ config)

/-- Sessions reachable from `start` by `step`. -/
inductive Reachable (cc : CChart) : Session → Prop
  | start (si : StartInput) (tape : Tape) (o : Out cc) :
      Vertep.start cc si tape = .ok o → Reachable cc o.session
  | step (s : Session) (i : Input) (tape : Tape) (o : Out cc) :
      Reachable cc s → Vertep.step cc s i tape = .ok o → Reachable cc o.session

/-- **Configuration Well-Formedness Theorem (statement only).** For every well-formed chart,
every running session reachable by `start` and `step` has a legal configuration. False as stated
(see the counterexamples above) until the two rules are added to `WellFormed`. -/
def ConfigurationWellFormedness : Prop :=
  ∀ c : Chart, WellFormed c → ∀ cc, compile c = .ok cc →
    ∀ s, Reachable cc s → s.terminated = false → Legal cc s.config

/-- Scenario *Compound exclusivity*, as a consequence of the theorem. -/
def CompoundExclusivity : Prop :=
  ∀ c : Chart, WellFormed c → ∀ cc, compile c = .ok cc → ∀ s, Reachable cc s →
    s.terminated = false → ∀ x ∈ s.config, cc.isCompound x = true →
      ∀ a ∈ (cc.st x).children, ∀ b ∈ (cc.st x).children, a ∈ s.config → b ∈ s.config → a = b

/-- Scenario *Parallel completeness*, as a consequence of the theorem. -/
def ParallelCompleteness : Prop :=
  ∀ c : Chart, WellFormed c → ∀ cc, compile c = .ok cc → ∀ s, Reachable cc s →
    s.terminated = false → ∀ x ∈ s.config, cc.isParallel x = true →
      ∀ r ∈ (cc.st x).children, r ∈ s.config

end Vertep.Pending
