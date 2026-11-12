import VertepLean.Config.CompileIR

/-!
# Compile correctness: the loops of `compile`

`compile` builds the states and transitions with two `for` loops over `Array.modify` and
`Array.push`. `compile_spec` summarizes the result as `Spec`: the compiled states are
`Compile.states` up to `initial` and `transitions` (`strip`), every compound state has a default
entry with the resolved initial targets, every state's transitions are its declared ones
resolved, and every history state without one gets the synthesized default; every transition's
targets are either resolved from a legal IR target list (`Good1`) or the children of a parent of
a history state without default entry (`Good2`).
-/

namespace Vertep.CompileProof

open Vertep

/-! ## Arrays and `forIn` -/

theorem getD_modify {α : Type} (a : Array α) (j i : Nat) (f : α → α) (d : α) :
    (a.modify j f).getD i d = if j = i ∧ i < a.size then f (a.getD i d) else a.getD i d := by
  simp only [Array.getD_eq_getD_getElem?, Array.getElem?_modify]
  by_cases h : i < a.size
  · simp [h]; split <;> simp_all
  · simp [h]

theorem getD_push {α : Type} (a : Array α) (x : α) (i : Nat) (d : α) :
    (a.push x).getD i d = if i < a.size then a.getD i d else if i = a.size then x else d := by
  simp only [Array.getD_eq_getD_getElem?, Array.getElem?_push]
  by_cases h : i < a.size
  · simp [h, Nat.ne_of_lt h]
  · by_cases h2 : i = a.size
    · simp [h2]
    · simp [h, h2]

theorem getD_push_size {α : Type} (a : Array α) (x : α) (d : α) : (a.push x).getD a.size d = x := by
  rw [getD_push]; simp

theorem getD_push_lt {α : Type} (a : Array α) (x : α) {i : Nat} (d : α) (h : i < a.size) :
    (a.push x).getD i d = a.getD i d := by
  rw [getD_push]; simp [h]

theorem getD_modify_ne {α : Type} (a : Array α) {j i : Nat} (f : α → α) (d : α) (h : j ≠ i) :
    (a.modify j f).getD i d = a.getD i d := by
  rw [getD_modify]; simp [h]

theorem getD_modify_eq {α : Type} (a : Array α) {i : Nat} (f : α → α) (d : α) (h : i < a.size) :
    (a.modify i f).getD i d = f (a.getD i d) := by
  rw [getD_modify]; simp [h]

theorem toArray_getD {α : Type} (l : List α) (i : Nat) (d : α) : l.toArray.getD i d = l.getD i d := by
  simp [Array.getD_eq_getD_getElem?, List.getD_eq_getElem?_getD]

theorem forIn_inv {α β ε : Type} {l : List α} {init : β} {f : α → β → Except ε (ForInStep β)}
    (P : Nat → β → Prop) (h0 : P 0 init)
    (hs : ∀ k (hk : k < l.length) b, P k b → ∃ b', f l[k] b = .ok (.yield b') ∧ P (k + 1) b') :
    ∃ b, forIn l init f = .ok b ∧ P l.length b := by
  induction l generalizing init P with
  | nil => exact ⟨init, rfl, h0⟩
  | cons a l ih =>
    obtain ⟨b', hb', hP⟩ := hs 0 (by simp) init h0
    obtain ⟨b, hb, hPb⟩ := ih (init := b') (fun k b => P (k + 1) b) hP
      (fun k hk b hb => hs (k + 1) (by simp; omega) b hb)
    refine ⟨b, ?_, by simpa using hPb⟩
    rw [List.forIn_cons]
    simp only [List.getElem_cons_zero] at hb'
    rw [hb']; exact hb

theorem forIn_bind_ok {α β γ ε : Type} {l : List α} {init : β}
    {f : α → β → Except ε (ForInStep β)} {g : β → Except ε γ} {r : γ}
    (P : Nat → β → Prop) (h0 : P 0 init)
    (hs : ∀ k (hk : k < l.length) b, P k b → ∃ b', f l[k] b = .ok (.yield b') ∧ P (k + 1) b')
    (h : (forIn l init f >>= g) = .ok r) : ∃ b, P l.length b ∧ g b = .ok r := by
  obtain ⟨b, hb, hP⟩ := forIn_inv P h0 hs
  rw [hb] at h; exact ⟨b, hP, h⟩

theorem forIn_bind_yield {α β δ ε : Type} {l : List α} {init : β}
    {f : α → β → Except ε (ForInStep β)} {g : β → Except ε (ForInStep δ)}
    (P : Nat → β → Prop) (R : δ → Prop) (h0 : P 0 init)
    (hs : ∀ k (hk : k < l.length) b, P k b → ∃ b', f l[k] b = .ok (.yield b') ∧ P (k + 1) b')
    (hg : ∀ b, P l.length b → ∃ d, g b = .ok (.yield d) ∧ R d) :
    ∃ d, (forIn l init f >>= g) = .ok (.yield d) ∧ R d := by
  obtain ⟨b, hb, hP⟩ := forIn_inv P h0 hs
  rw [hb]; exact hg b hP

/-! ## Invariants -/

/-- A compiled state without its `initial` and `transitions`, which the loops fill in. -/
def strip (x : CState) : CState := { x with transitions := [], initial := none }

theorem strip_kind {x y : CState} (h : strip x = strip y) : x.kind = y.kind :=
  by have := congrArg CState.kind h; exact this
theorem strip_parent {x y : CState} (h : strip x = strip y) : x.parent = y.parent :=
  by have := congrArg CState.parent h; exact this
theorem strip_children {x y : CState} (h : strip x = strip y) : x.children = y.children :=
  by have := congrArg CState.children h; exact this
theorem strip_histories {x y : CState} (h : strip x = strip y) : x.histories = y.histories :=
  by have := congrArg CState.histories h; exact this

/-- Targets resolved from a legal IR target list. -/
def Good1 (c : Chart) (tg : List Nat) : Prop :=
  ∃ L : List String, tg = Compile.resolve c.ids L ∧ (∀ x ∈ L, x ∈ c.ids) ∧ TargetSetOk c L

/-- The children of the parent of a history state, the parent without default entry. -/
def Good2 (S : Array CState) (tg : List Nat) : Prop :=
  ∃ h p, tg = (S.getD p CState.empty).children ∧ (S.getD h CState.empty).parent.getD 0 = p ∧
    (∃ ht, (S.getD h CState.empty).kind = .history ht) ∧ (S.getD p CState.empty).initial = none

/-- The targets of the root's default entry. -/
def rootTargets (c : Chart) : List Nat :=
  Compile.resolve c.ids (if c.initial.isEmpty = true then Compile.firstChild c.states else c.initial)

/-- The default entry of state `j` (IR state `s`). -/
def InitOk (c : Chart) (ts : Array CTrans) (j : Nat) (ini : Option Nat) (s : State) : Prop :=
  (s.kind = .compound → ∃ t, ini = some t ∧ t < ts.size ∧ (ts.getD t CTrans.empty).source = j ∧
    (ts.getD t CTrans.empty).defaultEntry = true ∧
    (ts.getD t CTrans.empty).targets = Compile.resolve c.ids (Compile.resolveInitial s).1) ∧
  (s.kind ≠ .compound → ini = none)

/-- The declared transitions of state `j` (IR state `s`). -/
def TransOk (c : Chart) (ts : Array CTrans) (j : Nat) (L : List Nat) (s : State) : Prop :=
  ∀ t ∈ L, t < ts.size ∧ (ts.getD t CTrans.empty).source = j ∧
    (ts.getD t CTrans.empty).defaultEntry = false ∧
    ∃ tr ∈ s.transitions, (ts.getD t CTrans.empty).targets = Compile.resolve c.ids tr.targets

def DeclOk (c : Chart) (ts : Array CTrans) (j : Nat) (L : List Nat) (s : State) : Prop :=
  L.length = s.transitions.length ∧ TransOk c ts j L s

/-- `ts'` extends `ts`. -/
def Ext (ts ts' : Array CTrans) : Prop :=
  ts.size ≤ ts'.size ∧ ∀ t, t < ts.size → ts'.getD t CTrans.empty = ts.getD t CTrans.empty

theorem Ext.refl (ts : Array CTrans) : Ext ts ts := ⟨Nat.le_refl _, fun _ _ => rfl⟩

theorem Ext.trans {a b d : Array CTrans} (h₁ : Ext a b) (h₂ : Ext b d) : Ext a d :=
  ⟨Nat.le_trans h₁.1 h₂.1, fun t ht => by rw [h₂.2 t (Nat.lt_of_lt_of_le ht h₁.1), h₁.2 t ht]⟩

theorem Ext.push (ts : Array CTrans) (x : CTrans) : Ext ts (ts.push x) :=
  ⟨by simp, fun _ ht => getD_push_lt ts x _ ht⟩

theorem InitOk.mono {c : Chart} {ts ts' : Array CTrans} {j : Nat} {ini : Option Nat} {s : State}
    (he : Ext ts ts') (h : InitOk c ts j ini s) : InitOk c ts' j ini s := by
  refine ⟨fun hc => ?_, h.2⟩
  obtain ⟨t, h1, h2, h3, h4, h5⟩ := h.1 hc
  refine ⟨t, h1, Nat.lt_of_lt_of_le h2 he.1, ?_⟩
  rw [he.2 t h2]; exact ⟨h3, h4, h5⟩

theorem TransOk.mono {c : Chart} {ts ts' : Array CTrans} {j : Nat} {L : List Nat} {s : State}
    (he : Ext ts ts') (h : TransOk c ts j L s) : TransOk c ts' j L s := by
  intro t ht
  obtain ⟨h1, h2⟩ := h t ht
  refine ⟨Nat.lt_of_lt_of_le h1 he.1, ?_⟩
  rw [he.2 t h1]; exact h2

theorem DeclOk.mono {c : Chart} {ts ts' : Array CTrans} {j : Nat} {L : List Nat} {s : State}
    (he : Ext ts ts') (h : DeclOk c ts j L s) : DeclOk c ts' j L s :=
  ⟨h.1, h.2.mono he⟩

/-- What every intermediate state of the first loop satisfies. -/
structure Common (c : Chart) (sts : Array CState) (ts : Array CTrans) : Prop where
  size : sts.size = c.allStates.length + 1
  skel : ∀ j, strip (sts.getD j CState.empty) = strip ((Compile.states c).getD j CState.empty)
  root_init : (sts.getD 0 CState.empty).initial = some 0
  root_trans : (sts.getD 0 CState.empty).transitions = []
  ts_pos : 0 < ts.size
  ts_root : (ts.getD 0 CTrans.empty).source = 0 ∧ (ts.getD 0 CTrans.empty).defaultEntry = true ∧
    (ts.getD 0 CTrans.empty).targets = rootTargets c
  good : ∀ t, t < ts.size → Good1 c (ts.getD t CTrans.empty).targets

/-- The invariant of the first loop after `m` states. -/
structure Inv1 (c : Chart) (m : Nat) (b : Array CState × Array CTrans × Nat) : Prop where
  common : Common c b.1 b.2.1
  idx : b.2.2 = m + 1
  done : ∀ k (hk : k < c.allStates.length), k < m →
    InitOk c b.2.1 (k + 1) (b.1.getD (k + 1) CState.empty).initial c.allStates[k] ∧
    DeclOk c b.2.1 (k + 1) (b.1.getD (k + 1) CState.empty).transitions c.allStates[k]
  todo : ∀ k, m ≤ k → k < c.allStates.length →
    (b.1.getD (k + 1) CState.empty).initial = none ∧ (b.1.getD (k + 1) CState.empty).transitions = []

/-- The invariant of the inner loop over the transitions of state `m + 1`, after `q` of them. -/
structure InvT (c : Chart) (m q : Nat) (hm : m < c.allStates.length)
    (b : Array CState × Array CTrans × Nat) : Prop where
  common : Common c b.1 b.2.1
  cnt : b.2.2 = q
  done : ∀ k (hk : k < c.allStates.length), k < m →
    InitOk c b.2.1 (k + 1) (b.1.getD (k + 1) CState.empty).initial c.allStates[k] ∧
    DeclOk c b.2.1 (k + 1) (b.1.getD (k + 1) CState.empty).transitions c.allStates[k]
  cur_init : InitOk c b.2.1 (m + 1) (b.1.getD (m + 1) CState.empty).initial c.allStates[m]
  cur_len : (b.1.getD (m + 1) CState.empty).transitions.length = q
  cur_trans : TransOk c b.2.1 (m + 1) (b.1.getD (m + 1) CState.empty).transitions c.allStates[m]
  todo : ∀ k, m < k → k < c.allStates.length →
    (b.1.getD (k + 1) CState.empty).initial = none ∧ (b.1.getD (k + 1) CState.empty).transitions = []

/-! ## The compiled states before the loops -/

section
variable (c : Chart)

theorem states_length : (Compile.states c).length = c.allStates.length + 1 := by
  simp [Compile.states, length_parented]

theorem states_getD_zero : (Compile.states c).getD 0 CState.empty =
    { id := "", kind := .root, parent := none,
      children := ((c.states.filter (fun s => !s.isHistory)).map (·.id)).filterMap (Compile.idx c.ids),
      histories := ((c.states.filter (fun s => s.isHistory)).map (·.id)).filterMap (Compile.idx c.ids),
      transitions := [], initial := none, onentry := [], onexit := [], data := c.data,
      invokes := [], donedata := none } := by
  simp [Compile.states]

theorem states_getD_succ (k : Nat) (hk : k < c.parented.length) :
    (Compile.states c).getD (k + 1) CState.empty =
    { id := (c.parented[k]).2.id, kind := Compile.kindOf (c.parented[k]).2,
      parent := some (match (c.parented[k]).1 with
        | some ps => (Compile.idx c.ids ps.id).getD 0
        | none => 0),
      children := (((c.parented[k]).2.children.filter (fun c => !c.isHistory)).map (·.id)).filterMap
        (Compile.idx c.ids),
      histories := (((c.parented[k]).2.children.filter (fun c => c.isHistory)).map (·.id)).filterMap
        (Compile.idx c.ids),
      transitions := [], initial := none, onentry := (c.parented[k]).2.onentry,
      onexit := (c.parented[k]).2.onexit,
      data := (c.parented[k]).2.data, invokes := (c.parented[k]).2.invokes,
      donedata := (c.parented[k]).2.donedata } := by
  simp only [Compile.states, List.getD_eq_getElem?_getD, List.getElem?_cons_succ,
    List.getElem?_map, List.getElem?_eq_getElem hk, Option.map_some, Option.getD_some]
  rfl

theorem states_getD_ge (j : Nat) (hj : c.allStates.length < j) :
    (Compile.states c).getD j CState.empty = CState.empty := by
  rw [List.getD_eq_getElem?_getD, List.getElem?_eq_none (by rw [states_length]; omega)]; rfl

theorem states_trans (j : Nat) : ((Compile.states c).getD j CState.empty).transitions = [] := by
  rcases j with _ | k
  · rw [states_getD_zero]
  · by_cases hk : k < c.parented.length
    · rw [states_getD_succ c k hk]
    · rw [states_getD_ge c _ (by rw [← length_parented]; omega)]; rfl

theorem states_init (j : Nat) : ((Compile.states c).getD j CState.empty).initial = none := by
  rcases j with _ | k
  · rw [states_getD_zero]
  · by_cases hk : k < c.parented.length
    · rw [states_getD_succ c k hk]
    · rw [states_getD_ge c _ (by rw [← length_parented]; omega)]; rfl

end

/-! ## Facts from well-formedness used by the loops -/

section
variable {c : Chart} (hwf : WellFormed c)
include hwf

theorem wf_nodup : c.ids.Nodup := hwf.2.2.1

theorem wf_stateOk {o : Option State} {s : State} (h : (o, s) ∈ c.parented) : StateOk c o s :=
  hwf.2.2.2.2.2.2.2.2.1 _ h

theorem wf_stateOk_k (k : Nat) (hk : k < c.allStates.length) :
    StateOk c (c.parented[k]'(by rw [length_parented]; exact hk)).1 c.allStates[k] := by
  have hk' : k < c.parented.length := by rw [length_parented]; exact hk
  have hm : ((c.parented[k]).1, (c.parented[k]).2) ∈ c.parented := List.getElem_mem hk'
  have := wf_stateOk hwf hm
  simpa only [parented_getElem_snd] using this

omit hwf in
theorem targetSetOk_single (x : String) : TargetSetOk c [x] := by
  unfold TargetSetOk
  simp only [List.filterMap_cons, List.filterMap_nil]
  split <;> simp

omit hwf in
theorem good_firstChild {cs : List State} (hcs : ∀ s ∈ cs, s ∈ c.allStates) :
    Good1 c (Compile.resolve c.ids (Compile.firstChild cs)) := by
  refine ⟨Compile.firstChild cs, rfl, ?_, ?_⟩
  · unfold Compile.firstChild
    split
    · rename_i s hs
      simp only [List.mem_singleton, forall_eq]
      exact List.mem_map_of_mem (hcs s (List.mem_of_find?_eq_some hs))
    · simp
  · unfold Compile.firstChild
    split
    · exact targetSetOk_single _
    · unfold TargetSetOk; simp

theorem good_root : Good1 c (rootTargets c) := by
  unfold rootTargets
  split
  · exact good_firstChild (fun s hs => (states_sub c).subset hs)
  · exact ⟨c.initial, rfl, hwf.2.2.2.1, hwf.2.2.2.2.1⟩

theorem good_initial (k : Nat) (hk : k < c.allStates.length) :
    Good1 c (Compile.resolve c.ids (Compile.resolveInitial c.allStates[k]).1) := by
  obtain ⟨_, _, hia, hie, _⟩ := wf_stateOk_k hwf k hk
  have hmem : c.allStates[k] ∈ c.allStates := List.getElem_mem hk
  unfold Compile.resolveInitial
  split
  · exact ⟨_, rfl, fun x hx => (hia.1 x hx).1, hia.2⟩
  · split
    · rename_i ie hie'
      obtain ⟨_, _, _, hok, _⟩ := hie ie hie'
      exact ⟨_, rfl, fun x hx => (hok.1 x hx).1, hok.2⟩
    · exact good_firstChild (fun s hs => mem_of_child c hmem hs)

theorem good_trans (k : Nat) (hk : k < c.allStates.length) {tr : Transition}
    (htr : tr ∈ c.allStates[k].transitions) : Good1 c (Compile.resolve c.ids tr.targets) := by
  obtain ⟨_, _, _, _, htrs, _⟩ := wf_stateOk_k hwf k hk
  obtain ⟨h1, h2, _⟩ := htrs tr htr
  exact ⟨_, rfl, h1, h2⟩

end

/-! ## The first loop -/

theorem common_modify {c : Chart} {sts : Array CState} {ts : Array CTrans} {i : Nat}
    {f : CState → CState} (hf : ∀ x, strip (f x) = strip x) (hi : i ≠ 0)
    (h : Common c sts ts) : Common c (sts.modify i f) ts := by
  refine ⟨by rw [Array.size_modify, h.size], fun j => ?_, ?_, ?_, h.ts_pos, h.ts_root, h.good⟩
  · rw [getD_modify]; split
    · rw [hf, h.skel]
    · exact h.skel j
  · rw [getD_modify_ne _ _ _ hi]; exact h.root_init
  · rw [getD_modify_ne _ _ _ hi]; exact h.root_trans

theorem common_push {c : Chart} {sts : Array CState} {ts : Array CTrans} {x : CTrans}
    (hx : Good1 c x.targets) (h : Common c sts ts) : Common c sts (ts.push x) := by
  refine ⟨h.size, h.skel, h.root_init, h.root_trans, by simp, ?_, fun t ht => ?_⟩
  · rw [getD_push_lt _ _ _ h.ts_pos]; exact h.ts_root
  · rw [getD_push]
    by_cases h1 : t < ts.size
    · simp only [h1, ↓reduceIte]; exact h.good t h1
    · have h2 : t = ts.size := by simp at ht; omega
      subst h2; simp only [Nat.lt_irrefl, ↓reduceIte]; exact hx

section
variable {c : Chart} (hwf : WellFormed c)
include hwf

theorem inv1_init :
    Inv1 c 0 ((Compile.states c).toArray.modify 0 (fun r => { r with initial := some 0 }),
      #[{ source := 0, index := none, events := [], cond := none, targets := rootTargets c,
          internal := true, body := [], defaultEntry := true }], 1) := by
  have hsz : (Compile.states c).toArray.size = c.allStates.length + 1 := by
    simp [states_length]
  refine ⟨⟨?_, fun j => ?_, ?_, ?_, by simp, ?_, fun t ht => ?_⟩, rfl, fun _ _ h => absurd h (by omega),
    fun k _ hk => ?_⟩
  · simp [states_length]
  · rw [getD_modify]; split
    · rename_i h; rw [← h.1]; simp only [toArray_getD]; rfl
    · rw [toArray_getD]
  · rw [getD_modify_eq _ _ _ (by omega)]
  · rw [getD_modify_eq _ _ _ (by omega)]; simp only [toArray_getD]; exact states_trans c 0
  · simp [Array.getD_eq_getD_getElem?]
  · have : t = 0 := by simp at ht; omega
    subst this
    simp only [Array.getD_eq_getD_getElem?, List.getElem?_toArray, List.getElem?_cons_zero,
      Option.getD_some]
    exact good_root hwf
  · simp only
    rw [getD_modify_ne _ _ _ (by omega), toArray_getD]
    exact ⟨states_init c _, states_trans c _⟩

omit hwf in
theorem invT_start_plain (k : Nat) (hk : k < c.allStates.length) (sts : Array CState)
    (ts : Array CTrans) (i : Nat) (h : Inv1 c k (sts, ts, i)) (hc : c.allStates[k].kind ≠ .compound) :
    InvT c k 0 hk (sts, ts, 0) := by
  obtain ⟨hi, ht⟩ := h.todo k (Nat.le_refl _) hk
  refine ⟨h.common, rfl, h.done, ⟨fun h' => absurd h' hc, fun _ => hi⟩, ?_, ?_,
    fun k' hk' hk'' => h.todo k' (Nat.le_of_lt hk') hk''⟩
  · simp only; rw [ht]; rfl
  · simp only; rw [ht]; intro t ht; simp at ht

theorem invT_start_compound (k : Nat) (hk : k < c.allStates.length) (sts : Array CState)
    (ts : Array CTrans) (i : Nat) (h : Inv1 c k (sts, ts, i)) (hc : c.allStates[k].kind = .compound)
    (x : CTrans) (f : CState → CState) (hx1 : x.source = k + 1) (hx2 : x.defaultEntry = true)
    (hx3 : x.targets = Compile.resolve c.ids (Compile.resolveInitial c.allStates[k]).1)
    (hf : ∀ y, strip (f y) = strip y ∧ (f y).initial = some ts.size ∧ (f y).transitions = y.transitions) :
    InvT c k 0 hk (sts.modify (k + 1) f, ts.push x, 0) := by
  have hsz := h.common.size
  obtain ⟨_, ht⟩ := h.todo k (Nat.le_refl _) hk
  have hext := Ext.push ts x
  refine ⟨common_push (by rw [hx3]; exact good_initial hwf k hk)
      (common_modify (fun y => (hf y).1) (by omega) h.common), rfl, fun k' hk' hlt => ?_, ?_, ?_, ?_,
    fun k' hk' hk'' => ?_⟩
  · simp only
    rw [getD_modify_ne _ _ _ (by omega)]
    obtain ⟨h1, h2⟩ := h.done k' hk' hlt
    exact ⟨h1.mono hext, h2.mono hext⟩
  · simp only
    rw [getD_modify_eq _ _ _ (by simp only at hsz; omega), (hf _).2.1]
    refine ⟨fun _ => ⟨ts.size, rfl, by simp, ?_⟩, fun h' => absurd hc h'⟩
    rw [getD_push_size]; exact ⟨hx1, hx2, hx3⟩
  · simp only
    rw [getD_modify_eq _ _ _ (by simp only at hsz; omega), (hf _).2.2]
    simp only at ht; rw [ht]; rfl
  · simp only
    rw [getD_modify_eq _ _ _ (by simp only at hsz; omega), (hf _).2.2]
    simp only at ht; rw [ht]; intro t ht; simp at ht
  · simp only
    rw [getD_modify_ne _ _ _ (by omega)]
    exact h.todo k' (Nat.le_of_lt hk') hk''

theorem invT_step (k : Nat) (hk : k < c.allStates.length) (q : Nat)
    (hq : q < c.allStates[k].transitions.length) (sts : Array CState) (ts : Array CTrans) (n' : Nat)
    (h : InvT c k q hk (sts, ts, n')) (x : CTrans) (f : CState → CState)
    (hx1 : x.source = k + 1) (hx2 : x.defaultEntry = false)
    (hx3 : x.targets = Compile.resolve c.ids c.allStates[k].transitions[q].targets)
    (hf : ∀ y, strip (f y) = strip y ∧ (f y).initial = y.initial ∧
      (f y).transitions = y.transitions ++ [ts.size]) :
    InvT c k (q + 1) hk (sts.modify (k + 1) f, ts.push x, n' + 1) := by
  have hsz := h.common.size
  have hext := Ext.push ts x
  have hcnt : n' = q := h.cnt
  refine ⟨common_push (by rw [hx3]; exact good_trans hwf k hk (List.getElem_mem hq))
      (common_modify (fun y => (hf y).1) (by omega) h.common), by simp [hcnt],
      fun k' hk' hlt => ?_, ?_, ?_, ?_, fun k' hk' hk'' => ?_⟩
  · simp only
    rw [getD_modify_ne _ _ _ (by omega)]
    obtain ⟨h1, h2⟩ := h.done k' hk' hlt
    exact ⟨h1.mono hext, h2.mono hext⟩
  · simp only
    rw [getD_modify_eq _ _ _ (by simp only at hsz; omega), (hf _).2.1]
    exact h.cur_init.mono hext
  · simp only
    rw [getD_modify_eq _ _ _ (by simp only at hsz; omega), (hf _).2.2, List.length_append,
      List.length_singleton]
    have := h.cur_len; simp only at this; omega
  · simp only
    rw [getD_modify_eq _ _ _ (by simp only at hsz; omega), (hf _).2.2]
    intro t ht
    rcases List.mem_append.1 ht with ht | ht
    · exact (h.cur_trans.mono hext) t ht
    · simp only [List.mem_singleton] at ht; subst ht
      refine ⟨by simp, ?_⟩
      rw [getD_push_size]
      exact ⟨hx1, hx2, _, List.getElem_mem hq, hx3⟩
  · simp only
    rw [getD_modify_ne _ _ _ (by omega)]
    exact h.todo k' hk' hk''

omit hwf in
theorem inv1_of_invT (k : Nat) (hk : k < c.allStates.length)
    (b : Array CState × Array CTrans × Nat) (h : InvT c k c.allStates[k].transitions.length hk b) :
    Inv1 c (k + 1) (b.1, b.2.1, k + 1 + 1) := by
  refine ⟨h.common, rfl, fun k' hk' hlt => ?_, fun k' hk' hk'' => h.todo k' (by omega) hk''⟩
  rcases Nat.lt_or_ge k' k with hlt' | hge
  · exact h.done k' hk' hlt'
  · have : k' = k := by omega
    subst this
    exact ⟨h.cur_init, h.cur_len, h.cur_trans⟩

end

/-! ## The second loop -/

/-- The synthesized default targets of history state `j`. -/
def synth (S : Array CState) (T : Array CTrans) (j : Nat) : List Nat :=
  match (S.getD ((S.getD j CState.empty).parent.getD 0) CState.empty).initial with
  | some ti => (T.getD ti CTrans.empty).targets
  | none => (S.getD ((S.getD j CState.empty).parent.getD 0) CState.empty).children

def IsHist (x : CState) : Prop := ∃ ht, x.kind = .history ht

structure Inv2 (c : Chart) (sts1 : Array CState) (ts1 : Array CTrans) (m : Nat)
    (b : Array CState × Array CTrans) : Prop where
  size : b.1.size = sts1.size
  same : ∀ j, strip (b.1.getD j CState.empty) = strip (sts1.getD j CState.empty) ∧
    (b.1.getD j CState.empty).initial = (sts1.getD j CState.empty).initial
  ext : Ext ts1 b.2
  hist : ∀ j, j < m → IsHist (sts1.getD j CState.empty) →
    (sts1.getD j CState.empty).transitions = [] →
    ∃ t, (b.1.getD j CState.empty).transitions = [t] ∧ t < b.2.size ∧
      (b.2.getD t CTrans.empty).source = j ∧ (b.2.getD t CTrans.empty).defaultEntry = false ∧
      (b.2.getD t CTrans.empty).targets = synth sts1 ts1 j
  keep : ∀ j, (j < m → ¬(IsHist (sts1.getD j CState.empty) ∧
      (sts1.getD j CState.empty).transitions = [])) ∨ m ≤ j →
    (b.1.getD j CState.empty).transitions = (sts1.getD j CState.empty).transitions
  good : ∀ t, t < b.2.size → Good1 c (b.2.getD t CTrans.empty).targets ∨
    Good2 sts1 (b.2.getD t CTrans.empty).targets

theorem inv2_init {c : Chart} (sts1 : Array CState) (ts1 : Array CTrans)
    (hg : ∀ t, t < ts1.size → Good1 c (ts1.getD t CTrans.empty).targets) :
    Inv2 c sts1 ts1 0 (sts1, ts1) :=
  ⟨rfl, fun _ => ⟨rfl, rfl⟩, Ext.refl _, fun _ h => absurd h (Nat.not_lt_zero _),
    fun _ _ => rfl, fun t ht => Or.inl (hg t ht)⟩

theorem inv2_skip {c : Chart} {sts1 : Array CState} {ts1 : Array CTrans} (m : Nat)
    (sts : Array CState) (ts : Array CTrans) (h : Inv2 c sts1 ts1 m (sts, ts))
    (hn : ¬(IsHist (sts1.getD m CState.empty) ∧ (sts1.getD m CState.empty).transitions = [])) :
    Inv2 c sts1 ts1 (m + 1) (sts, ts) := by
  refine ⟨h.size, h.same, h.ext, fun j hj hh he => ?_, fun j hj => ?_, h.good⟩
  · rcases Nat.lt_or_ge j m with hlt | hge
    · exact h.hist j hlt hh he
    · have : j = m := by omega
      subst this; exact absurd ⟨hh, he⟩ hn
  · apply h.keep
    rcases hj with hj | hj
    · rcases Nat.lt_or_ge j m with hlt | hge
      · exact Or.inl (fun _ => hj (by omega))
      · exact Or.inr hge
    · exact Or.inr (by omega)

theorem inv2_hist {c : Chart} {sts1 : Array CState} {ts1 : Array CTrans} (m : Nat)
    (hm : m < sts1.size)
    (hti : ∀ j ti, (sts1.getD j CState.empty).initial = some ti → ti < ts1.size)
    (sts : Array CState) (ts : Array CTrans) (h : Inv2 c sts1 ts1 m (sts, ts))
    (hh : IsHist (sts1.getD m CState.empty)) (he : (sts1.getD m CState.empty).transitions = [])
    (hg1 : ∀ t, t < ts1.size → Good1 c (ts1.getD t CTrans.empty).targets)
    (x : CTrans) (f : CState → CState)
    (hx1 : x.source = m) (hx2 : x.defaultEntry = false)
    (hx3 : x.targets = synth sts ts m)
    (hf : ∀ y, strip (f y) = strip y ∧ (f y).initial = y.initial ∧ (f y).transitions = [ts.size]) :
    Inv2 c sts1 ts1 (m + 1) (sts.modify m f, ts.push x) := by
  have hext := Ext.push ts x
  have hsyn : synth sts ts m = synth sts1 ts1 m := by
    unfold synth
    have hp : (sts.getD m CState.empty).parent = (sts1.getD m CState.empty).parent :=
      strip_parent (h.same m).1
    rw [hp]
    generalize ((sts1.getD m CState.empty).parent.getD 0) = p
    rw [(h.same p).2, strip_children (h.same p).1]
    split
    · rename_i ti hti'
      have := h.ext.2 ti (hti p ti hti')
      simp only at this; rw [this]
    · rfl
  refine ⟨by rw [Array.size_modify, h.size], fun j => ?_, h.ext.trans hext, fun j hj hh' he' => ?_,
    fun j hj => ?_, fun t ht => ?_⟩
  · rw [getD_modify]; split
    · rw [(hf _).1, (hf _).2.1]; exact h.same j
    · exact h.same j
  · rcases Nat.lt_or_ge j m with hlt | hge
    · obtain ⟨t, h1, h2, h3⟩ := h.hist j hlt hh' he'
      refine ⟨t, ?_, Nat.lt_of_lt_of_le h2 hext.1, ?_⟩
      · rw [getD_modify_ne _ _ _ (by omega)]; exact h1
      · rw [hext.2 t h2]; exact h3
    · have : j = m := by omega
      subst this
      refine ⟨ts.size, ?_, by simp, ?_⟩
      · rw [getD_modify_eq _ _ _ (by rw [h.size]; exact hm), (hf _).2.2]
      · rw [getD_push_size]; exact ⟨hx1, hx2, hx3.trans hsyn⟩
  · have hne : m ≠ j := by
      rintro rfl
      rcases hj with hj | hj
      · exact hj (by omega) ⟨hh, he⟩
      · omega
    rw [getD_modify_ne _ _ _ hne]
    apply h.keep
    rcases hj with hj | hj
    · rcases Nat.lt_or_ge j m with hlt | hge
      · exact Or.inl (fun _ => hj (by omega))
      · exact Or.inr hge
    · exact Or.inr (by omega)
  · rw [getD_push]
    by_cases h1 : t < ts.size
    · simp only [h1, ↓reduceIte]; exact h.good t h1
    · have h2 : t = ts.size := by simp at ht; omega
      subst h2; simp only [Nat.lt_irrefl, ↓reduceIte]
      rw [hx3, hsyn]
      unfold synth
      split
      · rename_i ti hti'
        exact Or.inl (hg1 ti (hti _ ti hti'))
      · rename_i hnone
        refine Or.inr ⟨m, _, rfl, rfl, hh, ?_⟩
        exact hnone

/-! ## The result -/

theorem stateKind_beq (a b : StateKind) : (a == b) = true ↔ a = b := by
  cases a <;> cases b <;> (try (rename_i x y; cases x <;> cases y)) <;> (try (rename_i x; cases x)) <;>
    decide

/-- What `compile` produces. -/
structure Spec (c : Chart) (cc : CChart) : Prop where
  size : cc.size = c.allStates.length + 1
  skel : ∀ j, strip (cc.st j) = strip ((Compile.states c).getD j CState.empty)
  root_init : (cc.st 0).initial = some 0
  root_trans : (cc.st 0).transitions = []
  root_tr : (cc.tr 0).source = 0 ∧ (cc.tr 0).defaultEntry = true ∧ (cc.tr 0).targets = rootTargets c
  init : ∀ k (hk : k < c.allStates.length),
    InitOk c cc.trans (k + 1) (cc.st (k + 1)).initial c.allStates[k]
  decl : ∀ k (hk : k < c.allStates.length),
    ¬(IsHist (cc.st (k + 1)) ∧ c.allStates[k].transitions = []) →
    DeclOk c cc.trans (k + 1) (cc.st (k + 1)).transitions c.allStates[k]
  hist : ∀ k (hk : k < c.allStates.length), IsHist (cc.st (k + 1)) →
    c.allStates[k].transitions = [] →
    ∃ t, (cc.st (k + 1)).transitions = [t] ∧ (cc.tr t).source = k + 1 ∧
      (cc.tr t).defaultEntry = false ∧ (cc.tr t).targets = synth cc.states cc.trans (k + 1)
  good : ∀ t, Good1 c (cc.tr t).targets ∨ Good2 cc.states (cc.tr t).targets

theorem inv1_initial_lt {c : Chart} {m : Nat} {b : Array CState × Array CTrans × Nat}
    (hm : c.allStates.length ≤ m) (h : Inv1 c m b) :
    ∀ j ti, (b.1.getD j CState.empty).initial = some ti → ti < b.2.1.size := by
  intro j ti hj
  rcases j with _ | k
  · rw [h.common.root_init] at hj; cases hj; exact h.common.ts_pos
  · by_cases hk : k < c.allStates.length
    · obtain ⟨⟨h1, h2⟩, _⟩ := h.done k hk (by omega)
      by_cases hc : c.allStates[k].kind = .compound
      · obtain ⟨t, ht, hlt, _⟩ := h1 hc
        rw [hj] at ht; cases ht; exact hlt
      · rw [h2 hc] at hj; cases hj
    · have : b.1.getD (k + 1) CState.empty = CState.empty := by
        rw [Array.getD_eq_getD_getElem?, Array.getElem?_eq_none (by rw [h.common.size]; omega)]; rfl
      rw [this] at hj; cases hj

theorem compile_spec {c : Chart} {cc : CChart} (hwf : WellFormed c) (h : compile c = .ok cc) :
    Spec c cc := by
  have hck : checkWF c = true := (checkWF_iff c).2 hwf
  unfold compile at h
  simp only [hck, Bool.not_true, Bool.false_eq_true, ↓reduceIte] at h
  have H := forIn_bind_ok (Inv1 c) (inv1_init hwf) ?step h
  case step =>
    clear h
    intro k hk ⟨sts, ts, i⟩ hb
    obtain rfl : i = k + 1 := hb.idx
    have hk' : k < c.allStates.length := by rw [← length_parented]; exact hk
    simp only [parented_getElem_snd]
    split
    · rename_i hc
      refine forIn_bind_yield (fun q b => InvT c k q hk' b) (Inv1 c (k + 1))
        (invT_start_compound hwf k hk' sts ts (k + 1) hb ((stateKind_beq _ _).1 hc) _ _ rfl rfl rfl
          (fun _ => ⟨rfl, rfl, rfl⟩)) ?_ ?_
      · rintro q hq ⟨s', t', n'⟩ hq'
        exact ⟨_, rfl, invT_step hwf k hk' q hq s' t' n' hq' _ _ rfl rfl rfl
          (fun _ => ⟨rfl, rfl, rfl⟩)⟩
      · intro b hb'
        exact ⟨_, rfl, inv1_of_invT k hk' b hb'⟩
    · rename_i hc
      refine forIn_bind_yield (fun q b => InvT c k q hk' b) (Inv1 c (k + 1))
        (invT_start_plain k hk' sts ts (k + 1) hb (fun h' => by rw [← stateKind_beq] at h'; simp_all)) ?_ ?_
      · rintro q hq ⟨s', t', n'⟩ hq'
        exact ⟨_, rfl, invT_step hwf k hk' q hq s' t' n' hq' _ _ rfl rfl rfl
          (fun _ => ⟨rfl, rfl, rfl⟩)⟩
      · intro b hb'
        exact ⟨_, rfl, inv1_of_invT k hk' b hb'⟩
  obtain ⟨⟨sts1, ts1, i1⟩, hinv1, h⟩ := H
  rw [length_parented] at hinv1
  have hti := inv1_initial_lt (Nat.le_refl _) hinv1
  simp only at hti
  have hg1 := hinv1.common.good
  simp only at hg1
  have H2 := forIn_bind_ok (Inv2 c sts1 ts1) (inv2_init sts1 ts1 hg1) ?step2 h
  case step2 =>
    clear h
    intro m hm ⟨sts, ts⟩ hb
    simp only [List.length_range] at hm
    simp only [List.getElem_range]
    have hkind := strip_kind (hb.same m).1
    have htr := hb.keep m (Or.inr (Nat.le_refl _))
    simp only at hkind htr
    split
    · rename_i ty heq
      split
      · rename_i hemp
        have hh : IsHist (sts1.getD m CState.empty) := ⟨ty, hkind ▸ heq⟩
        have he : (sts1.getD m CState.empty).transitions = [] := by
          rw [← htr]; simpa using hemp
        exact ⟨_, rfl, inv2_hist m hm hti sts ts hb hh he hg1 _ _ rfl rfl rfl
          (fun _ => ⟨rfl, rfl, rfl⟩)⟩
      · rename_i hemp
        refine ⟨_, rfl, inv2_skip m sts ts hb ?_⟩
        rintro ⟨_, he⟩
        rw [← htr] at he; rw [he] at hemp; exact hemp rfl
    · rename_i hnot
      refine ⟨_, rfl, inv2_skip m sts ts hb ?_⟩
      rintro ⟨⟨ty, hh⟩, _⟩
      exact hnot ty (hkind ▸ hh)
  obtain ⟨⟨sts2, ts2⟩, hinv2, h⟩ := H2
  cases h
  have hsz1 := hinv1.common.size
  simp only at hsz1
  simp only [List.length_range] at hinv2
  have hsame (j : Nat) := hinv2.same j
  simp only at hsame
  have hext := hinv2.ext
  have hext' (t : Nat) (ht : t < ts1.size) : ts2.getD t CTrans.empty = ts1.getD t CTrans.empty :=
    hext.2 t ht
  have hskel (j : Nat) : strip (sts2.getD j CState.empty) =
      strip ((Compile.states c).getD j CState.empty) := by
    rw [(hsame j).1]; exact hinv1.common.skel j
  have hkeep := hinv2.keep
  simp only at hkeep
  refine ⟨?_, hskel, ?_, ?_, ?_, fun k hk => ?_, fun k hk hn => ?_, fun k hk hh he => ?_, fun t => ?_⟩
  · show sts2.size = _; rw [hinv2.size, hsz1]
  · show (sts2.getD 0 CState.empty).initial = _; rw [(hsame 0).2]; exact hinv1.common.root_init
  · show (sts2.getD 0 CState.empty).transitions = _
    rw [hkeep 0 (Or.inl fun _ ⟨⟨ty, hh⟩, _⟩ => ?_)]
    · exact hinv1.common.root_trans
    · have := strip_kind (hinv1.common.skel 0)
      rw [states_getD_zero] at this; simp only at this
      rw [this] at hh; cases hh
  · show (ts2.getD 0 CTrans.empty).source = 0 ∧ (ts2.getD 0 CTrans.empty).defaultEntry = true ∧
      (ts2.getD 0 CTrans.empty).targets = rootTargets c
    rw [hext' 0 hinv1.common.ts_pos]; exact hinv1.common.ts_root
  · show InitOk c ts2 (k + 1) (sts2.getD (k + 1) CState.empty).initial c.allStates[k]
    rw [(hsame _).2]; exact (hinv1.done k hk hk).1.mono hext
  · show DeclOk c ts2 (k + 1) (sts2.getD (k + 1) CState.empty).transitions c.allStates[k]
    have hd := (hinv1.done k hk hk).2
    have hn' : ¬(IsHist (sts1.getD (k + 1) CState.empty) ∧
        (sts1.getD (k + 1) CState.empty).transitions = []) := by
      rintro ⟨⟨ty, hh⟩, he⟩
      refine hn ⟨⟨ty, ?_⟩, ?_⟩
      · show (sts2.getD (k + 1) CState.empty).kind = _
        rw [strip_kind (hsame _).1]; exact hh
      · have := hd.1; simp only at this; rw [he] at this; exact List.length_eq_zero_iff.1 this.symm
    rw [hkeep (k + 1) (Or.inl fun _ => hn')]
    exact hd.mono hext
  · have hd := (hinv1.done k hk hk).2
    have hh' : IsHist (sts1.getD (k + 1) CState.empty) := by
      obtain ⟨ty, hh⟩ := hh
      refine ⟨ty, ?_⟩
      have : (sts2.getD (k + 1) CState.empty).kind = _ := hh
      rw [strip_kind (hsame _).1] at this; exact this
    have he' : (sts1.getD (k + 1) CState.empty).transitions = [] := by
      have := hd.1; simp only at this; rw [he] at this; exact List.length_eq_zero_iff.1 this
    obtain ⟨t, h1, _, h3, h4, h5⟩ := hinv2.hist (k + 1) (by omega) hh' he'
    refine ⟨t, h1, h3, h4, ?_⟩
    show (ts2.getD t CTrans.empty).targets = synth sts2 ts2 (k + 1)
    rw [h5]
    unfold synth
    rw [strip_parent (hsame (k + 1)).1]
    generalize ((sts1.getD (k + 1) CState.empty).parent.getD 0) = p
    rw [(hsame p).2, strip_children (hsame p).1]
    split
    · rename_i ti hti'
      rw [hext' ti (hti p ti hti')]
    · rfl
  · show Good1 c (ts2.getD t CTrans.empty).targets ∨ Good2 sts2 (ts2.getD t CTrans.empty).targets
    by_cases ht : t < ts2.size
    · rcases hinv2.good t ht with hg | ⟨hh, p, h1, h2, ⟨ty, h3⟩, h4⟩
      · exact Or.inl hg
      · refine Or.inr ⟨hh, p, ?_, ?_, ⟨ty, ?_⟩, ?_⟩
        · rw [h1, strip_children (hsame p).1]
        · rw [strip_parent (hsame hh).1]; exact h2
        · rw [strip_kind (hsame hh).1]; exact h3
        · rw [(hsame p).2]; exact h4
    · have : ts2.getD t CTrans.empty = CTrans.empty := by
        rw [Array.getD_eq_getD_getElem?, Array.getElem?_eq_none (by omega)]; rfl
      rw [this]
      exact Or.inl ⟨[], rfl, by simp, by simp [TargetSetOk]⟩

end Vertep.CompileProof
