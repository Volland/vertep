import VertepLean.Step

/-!
# Configuration well-formedness: definitions and interfaces

The Configuration Well-Formedness Theorem (formal-model) says that every session reachable by
`start` and `step` from a well-formed chart has a legal configuration (W3C SCXML 3.11). Its proof
is split into four parts that meet at the definitions of this file:

* `CWF` — the structural facts about a compiled chart the semantics relies on; `compile` of a
  well-formed chart satisfies them (`VertepLean/Config/Compile.lean`);
* `HistOk` — every recorded history value restores a legal fragment below the history's parent;
  `EffSpec` — `effectiveTargets` computes exactly the effective targets (`EffMem`), its fuel is
  never exhausted (`VertepLean/Config/Exit.lean`);
* the exit lemma — a conflict-free selection in a legal configuration exits exactly the active
  proper descendants of the transitions' domains, records legal history values and leaves the
  situation `EntryPre` describes (`VertepLean/Config/Exit.lean`);
* the entry lemma — from `EntryPre`, `entrySet` succeeds and the configuration it completes is
  legal (`VertepLean/Config/Entry.lean`);

and the interpreter invariant that carries them through `start` and `step`
(`VertepLean/Config/Interp.lean`, `VertepLean/Config/Theorem.lean`).
-/

namespace Vertep

/-- Exactly one element of `cs` satisfies `S`. -/
def ExactlyOne (cs : List Nat) (S : Nat → Prop) : Prop :=
  (∃ c ∈ cs, S c) ∧ ∀ c₁ ∈ cs, ∀ c₂ ∈ cs, S c₁ → S c₂ → c₁ = c₂

namespace CChart

variable (cc : CChart)

/-- The position a target takes in a target set: a history state stands for its parent. -/
def pos (x : Nat) : Nat := if cc.isHistory x then ((cc.st x).parent).getD 0 else x

/-- `a` and `b` may be active together without being related: distinct, neither a descendant
of the other, and their lowest common ancestor is a parallel state. -/
def Compatible (a b : Nat) : Prop :=
  a ≠ b ∧ cc.isDesc a b = false ∧ cc.isDesc b a = false ∧
  ∃ q, cc.isParallel q = true ∧ cc.isDesc a q = true ∧ cc.isDesc b q = true ∧
    ∀ q', cc.isDesc a q' = true → cc.isDesc b q' = true → q' = q ∨ cc.isDesc q q' = true

/-- A legal target set (`IllegalTargetSet` on compiled charts): positions pairwise
compatible. -/
def TargetsLegal (ts : List Nat) : Prop := ts.Pairwise (fun x y => cc.Compatible (cc.pos x) (cc.pos y))

/-- `S` is a legal fragment below `p`: proper descendants of `p`, no history states, closed
under parents up to `p`, exactly one child of every compound state in `S` (and of `p` if `p`
is compound or the root), every child of every parallel state in `S` (and of `p` if `p` is
parallel). A legal configuration is a legal fragment below the root (`legal_iff_below`). -/
structure Below (p : Nat) (S : List Nat) : Prop where
  desc : ∀ x ∈ S, cc.isDesc x p = true
  noHist : ∀ x ∈ S, cc.isHistory x = false
  parent : ∀ x ∈ S, ∀ q, (cc.st x).parent = some q → q = p ∨ q ∈ S
  compound : ∀ x ∈ S, cc.isCompound x = true → ExactlyOne (cc.st x).children (· ∈ S)
  parallel : ∀ x ∈ S, cc.isParallel x = true → ∀ c ∈ (cc.st x).children, c ∈ S
  top : cc.isCompoundOrRoot p = true → ExactlyOne (cc.st p).children (· ∈ S)
  topPar : cc.isParallel p = true → ∀ c ∈ (cc.st p).children, c ∈ S

end CChart

/-- A legal configuration (W3C SCXML 3.11): the root (implicit: Rust and Lean never store it)
is active; active states are proper, non-history states of the chart; every active state's
parent is active or the root; the root and every active compound state have exactly one active
child; every active parallel state has all of its children active. -/
def Legal (cc : CChart) (config : List Nat) : Prop :=
  (∀ s ∈ config, s ≠ 0 ∧ s < cc.size ∧ cc.isHistory s = false) ∧
  (∀ s ∈ config, ∀ p, (cc.st s).parent = some p → p = 0 ∨ p ∈ config) ∧
  ((cc.st 0).children.filter (· ∈ config)).length = 1 ∧
  (∀ s ∈ config, cc.isCompound s = true →
    ((cc.st s).children.filter (· ∈ config)).length = 1) ∧
  (∀ s ∈ config, cc.isParallel s = true → ∀ c ∈ (cc.st s).children, c ∈ config)

/-- The structural facts about a compiled chart that the configuration proof relies on, all
established by `compile` for well-formed charts. -/
structure CWF (cc : CChart) : Prop where
  size_pos : 0 < cc.size
  root_kind : (cc.st 0).kind = .root
  root_parent : (cc.st 0).parent = none
  parent_lt : ∀ i, 0 < i → i < cc.size → ∃ p, (cc.st i).parent = some p ∧ p < i
  kind_ne_root : ∀ i, i ≠ 0 → (cc.st i).kind ≠ .root
  children_iff : ∀ i c, c ∈ (cc.st i).children ↔
    c < cc.size ∧ (cc.st c).parent = some i ∧ cc.isHistory c = false
  histories_iff : ∀ i h, h ∈ (cc.st i).histories ↔
    h < cc.size ∧ (cc.st h).parent = some i ∧ cc.isHistory h = true
  children_nodup : ∀ i, (cc.st i).children.Nodup
  histories_nodup : ∀ i, (cc.st i).histories.Nodup
  root_children : (cc.st 0).children ≠ []
  compound_children : ∀ i, cc.isCompound i = true → (cc.st i).children ≠ []
  /-- Atomic, final and history states have no children. -/
  leaf : ∀ i, ((cc.st i).kind = .atomic ∨ (cc.st i).kind = .final ∨ cc.isHistory i = true) →
    (cc.st i).children = [] ∧ (cc.st i).histories = []
  /-- No history state at the root (`HistoryAtRoot`). -/
  history_parent : ∀ h, cc.isHistory h = true → ∃ p, (cc.st h).parent = some p ∧ p ≠ 0
  /-- Declared transitions leave their state, are never default entries, and target states. -/
  trans_source : ∀ i t, t ∈ (cc.st i).transitions → (cc.tr t).source = i
  trans_not_default : ∀ i t, t ∈ (cc.st i).transitions → (cc.tr t).defaultEntry = false
  targets_lt : ∀ t x, x ∈ (cc.tr t).targets → 0 < x ∧ x < cc.size
  targets_legal : ∀ t, cc.TargetsLegal (cc.tr t).targets
  /-- Exactly compound states and the root have a default-entry transition, whose targets are
  proper descendants. -/
  initial_iff : ∀ i, (cc.st i).initial.isSome = cc.isCompoundOrRoot i
  initial_ok : ∀ i t, (cc.st i).initial = some t →
    (cc.tr t).defaultEntry = true ∧ (cc.tr t).source = i ∧ (cc.tr t).targets ≠ [] ∧
    ∀ x ∈ (cc.tr t).targets, cc.isDesc x i = true
  /-- Every history state has exactly one (declared or synthesized) default transition, with
  targets below its parent. -/
  history_default : ∀ h, cc.isHistory h = true → ∃ d, (cc.st h).transitions = [d] ∧
    (cc.tr d).targets ≠ [] ∧
    ∀ x ∈ (cc.tr d).targets, ∀ p, (cc.st h).parent = some p → cc.isDesc x p = true
  /-- History defaults are acyclic (`HistoryDefaultCycle`), as a rank below the chart size. -/
  history_rank : ∃ r : Nat → Nat, (∀ h, cc.isHistory h = true → r h < cc.size) ∧
    ∀ h d x, cc.isHistory h = true → cc.historyDefault h = some d → x ∈ (cc.tr d).targets →
      cc.isHistory x = true → r x < r h

/-- Every recorded history value restores a legal fragment below the history's parent: its
active atomic states (deep) or its active children (shallow). -/
def HistOk (cc : CChart) (hist : List (Nat × List Nat)) : Prop :=
  ∀ h v, CChart.histValue hist h = some v → cc.isHistory h = true ∧
    ∃ p S, (cc.st h).parent = some p ∧ cc.Below p S ∧
      (cc.isDeepHistory h = true → ∀ x, x ∈ v ↔ x ∈ S ∧ cc.isAtomic x = true) ∧
      (cc.isDeepHistory h = false → ∀ x, x ∈ v ↔ x ∈ S ∧ (cc.st x).parent = some p)

/-- The effective targets of a transition (Appendix D `getEffectiveTargetStates`), as a
relation: non-history targets, the recorded states of history targets, and the effective
targets of the default transition of history targets without a recorded value. -/
inductive EffMem (cc : CChart) (hist : List (Nat × List Nat)) : Nat → Nat → Prop
  | plain {t x} : x ∈ (cc.tr t).targets → cc.isHistory x = false → EffMem cc hist t x
  | value {t h v x} : h ∈ (cc.tr t).targets → cc.isHistory h = true →
      CChart.histValue hist h = some v → x ∈ v → EffMem cc hist t x
  | dflt {t h d x} : h ∈ (cc.tr t).targets → cc.isHistory h = true →
      CChart.histValue hist h = none → cc.historyDefault h = some d → EffMem cc hist d x →
      EffMem cc hist t x

/-- `effectiveTargets` computes exactly the effective targets (its fuel suffices). -/
def EffSpec (cc : CChart) (hist : List (Nat × List Nat)) : Prop :=
  ∀ t x, x ∈ cc.effectiveTargets hist t ↔ EffMem cc hist t x

/-- What the entry lemma needs about one transition with targets and its domain `d`: the domain
is active (or the root), compound or the root, has no active proper descendant, contains every
effective target, and every target's position lies below it (a history target's position may be
the domain itself, or, for a history state whose effective targets lie below the domain, above
it: then the domain is a proper descendant of that position). -/
structure DomOk (cc : CChart) (hist : List (Nat × List Nat)) (cfg : List Nat) (t d : Nat) :
    Prop where
  active : d = 0 ∨ d ∈ cfg
  kind : cc.isCompoundOrRoot d = true
  empty : ∀ x ∈ cfg, cc.isDesc x d = false
  eff : ∀ x, EffMem cc hist t x → cc.isDesc x d = true
  targets : ∀ x ∈ (cc.tr t).targets,
    cc.isDesc (cc.pos x) d = true ∨
      (cc.isHistory x = true ∧ (cc.pos x = d ∨ cc.isDesc d (cc.pos x) = true))

/-- The situation `enterStates` starts from: `cfg` is the configuration after the exit, `hist`
the history after recording, `dhist` the history the domains are computed under. Each targeted
transition has a domain satisfying `DomOk`; distinct targeted transitions have unrelated
domains; `cfg` is legal except below the domains. -/
structure EntryPre (cc : CChart) (hist dhist : List (Nat × List Nat)) (cfg ts : List Nat) :
    Prop where
  nodup : ts.Nodup
  histOk : HistOk cc hist
  dom : ∀ t ∈ ts, (cc.tr t).targets ≠ [] → ∃ d, cc.domain dhist t = some d ∧ DomOk cc hist cfg t d
  sep : ∀ t₁ ∈ ts, ∀ t₂ ∈ ts, t₁ ≠ t₂ → (cc.tr t₁).targets ≠ [] → (cc.tr t₂).targets ≠ [] →
    ∀ d₁ d₂, cc.domain dhist t₁ = some d₁ → cc.domain dhist t₂ = some d₂ →
      d₁ ≠ d₂ ∧ cc.isDesc d₁ d₂ = false ∧ cc.isDesc d₂ d₁ = false
  /-- `cfg` is legal outside the domains: proper non-history states, closed under parents,
  every active parallel state complete, and every active compound state (and the root) that is
  not a domain has exactly one active child. -/
  noHist : ∀ x ∈ cfg, x ≠ 0 ∧ x < cc.size ∧ cc.isHistory x = false
  parent : ∀ x ∈ cfg, ∀ q, (cc.st x).parent = some q → q = 0 ∨ q ∈ cfg
  parallel : ∀ x ∈ cfg, cc.isParallel x = true → ∀ c ∈ (cc.st x).children, c ∈ cfg
  compound : ∀ x, (x = 0 ∨ x ∈ cfg) → cc.isCompoundOrRoot x = true →
    (∀ t ∈ ts, (cc.tr t).targets ≠ [] → cc.domain dhist t ≠ some x) →
    ExactlyOne (cc.st x).children (· ∈ cfg)

/-! ## The interfaces between the parts -/

/-- `compile` establishes `CWF` (compile correctness). -/
def CompileCWF : Prop := ∀ c cc, WellFormed c → compile c = .ok cc → CWF cc

/-- Fuel adequacy of `effectiveTargets` under legal history values. -/
def EffAdequate : Prop := ∀ cc hist, CWF cc → HistOk cc hist → EffSpec cc hist

/-- The entry lemma: from `EntryPre`, `entrySet` succeeds (its fuel suffices) and the states
before plus the states it enters form a legal configuration. -/
def EntryLegal : Prop :=
  ∀ cc hist dhist cfg ts, CWF cc → EffSpec cc hist → EntryPre cc hist dhist cfg ts →
    ∃ acc, cc.entrySet hist dhist ts = some acc ∧
      ∀ L : List Nat, (∀ x, x ∈ L ↔ x ∈ cfg ∨ x ∈ acc.toEnter) → cc.Below 0 L

/-- History recording (`recordHistory`) as a pure function: for every exited state, in exit-set
order, each of its history states gets the active atomic descendants (deep) or the active
children (shallow), replacing its earlier value. -/
def CChart.recordPure (cc : CChart) (config exitSet : List Nat) (hist : List (Nat × List Nat)) :
    List (Nat × List Nat) :=
  exitSet.foldl (fun hist s => (cc.st s).histories.foldl (fun hist h =>
    (h, if cc.isDeepHistory h then config.filter (fun x => cc.isAtomic x && cc.isDesc x s)
        else config.filter (fun x => (cc.st x).parent == some s)) ::
      hist.filter (fun e => e.1 != h)) hist) hist

/-- A selection of transitions as `select` returns it (see `transition_selection`): no
duplicates, each one declared on an active atomic state or one of its ancestors, pairwise
disjoint exit sets. -/
structure Selected (cc : CChart) (config : List Nat) (hist : List (Nat × List Nat))
    (ts : List Nat) : Prop where
  nodup : ts.Nodup
  enabled : ∀ t ∈ ts, ∃ a ∈ config, cc.isAtomic a = true ∧ t ∈ cc.candidates a
  disjoint : ∀ x ∈ ts, ∀ y ∈ ts, x ≠ y →
    ∀ s ∈ cc.exitSetOf config hist x, s ∉ cc.exitSetOf config hist y

/-- The exit lemma: from a legal configuration and legal history values, a selection exits
exactly the active proper descendants of the domains, records legal history values, and leaves
the situation of `EntryPre` (with the domains computed under the history before recording). -/
def ExitLegal : Prop :=
  ∀ cc config hist ts, CWF cc → cc.Below 0 config → config.Nodup → HistOk cc hist →
    Selected cc config hist ts →
    let ex := cc.exitSet config hist ts
    HistOk cc (cc.recordPure config ex hist) ∧
      EntryPre cc (cc.recordPure config ex hist) hist (config.filter (fun s => !ex.contains s)) ts

/-- The entry at start: the root's default entry from the empty configuration. -/
def StartEntry : Prop :=
  ∀ cc t, CWF cc → (cc.st 0).initial = some t → EntryPre cc [] [] [] [t]

end Vertep
