import VertepLean.Config.Tree

/-!
# Compile correctness: the IR side

Facts about `State.flatten`, `State.parented`, `Chart.allStates`, `Chart.ids` and `Chart.parentOf`
of a chart with unique identifiers, and the preorder position `pos` a state gets in the compiled
chart (`Compile.idx`).
-/

namespace Vertep.CompileProof

open Vertep

/-! ## Unfolding -/

theorem flatten_eq (s : State) : s.flatten = s :: State.flattenList s.children := by
  cases s; rfl

theorem flattenList_nil : State.flattenList [] = [] := by simp [State.flattenList]

theorem flattenList_cons (c : State) (cs : List State) :
    State.flattenList (c :: cs) = c.flatten ++ State.flattenList cs := by
  simp [State.flattenList]

theorem parented_eq (p : Option State) (s : State) :
    State.parented p s = (p, s) :: State.parentedList (some s) s.children := by
  cases s; rfl

theorem parentedList_nil (p : Option State) : State.parentedList p [] = [] := by
  simp [State.parentedList]

theorem parentedList_cons (p : Option State) (c : State) (cs : List State) :
    State.parentedList p (c :: cs) = State.parented p c ++ State.parentedList p cs := by
  simp [State.parentedList]

/-! ## Structure of `parented` and `flatten` -/

mutual
theorem parented_snd (p : Option State) : ∀ s : State, (State.parented p s).map Prod.snd = s.flatten
  | .mk _ _ _ _ _ cs .. => by
    rw [State.parented, State.flatten, List.map_cons, parentedList_snd]
theorem parentedList_snd (p : Option State) :
    ∀ cs : List State, (State.parentedList p cs).map Prod.snd = State.flattenList cs
  | [] => by simp [State.parentedList, State.flattenList]
  | c :: cs => by
    rw [State.parentedList, State.flattenList, List.map_append, parented_snd, parentedList_snd]
end

mutual
theorem mem_parented (o : Option State) : ∀ (s : State) (e : Option State × State),
    e ∈ State.parented o s ↔
      (e.1 = o ∧ e.2 = s) ∨ ∃ q ∈ s.flatten, e.1 = some q ∧ e.2 ∈ q.children
  | .mk a b el d f cs g h i j k l m, e => by
    have ih := mem_parentedList (some (.mk a b el d f cs g h i j k l m)) cs e
    have hc : (State.mk a b el d f cs g h i j k l m).children = cs := rfl
    generalize State.mk a b el d f cs g h i j k l m = s at ih hc ⊢
    rw [parented_eq, flatten_eq, hc, List.mem_cons, ih]
    rcases e with ⟨e1, e2⟩
    simp only [Prod.mk.injEq, List.mem_cons, exists_eq_or_imp]
    rw [hc]
theorem mem_parentedList (o : Option State) : ∀ (cs : List State) (e : Option State × State),
    e ∈ State.parentedList o cs ↔
      (e.1 = o ∧ e.2 ∈ cs) ∨ ∃ q ∈ State.flattenList cs, e.1 = some q ∧ e.2 ∈ q.children
  | [], e => by simp [State.parentedList, State.flattenList]
  | c :: cs, e => by
    have ih1 := mem_parented o c e
    have ih2 := mem_parentedList o cs e
    rw [State.parentedList, State.flattenList, List.mem_append, ih1, ih2]
    simp only [List.mem_cons, List.mem_append]
    constructor
    · rintro ((h | ⟨q, hq, h⟩) | (h | ⟨q, hq, h⟩))
      · exact Or.inl ⟨h.1, Or.inl h.2⟩
      · exact Or.inr ⟨q, Or.inl hq, h⟩
      · exact Or.inl ⟨h.1, Or.inr h.2⟩
      · exact Or.inr ⟨q, Or.inr hq, h⟩
    · rintro (⟨h1, h | h⟩ | ⟨q, hq | hq, h⟩)
      · exact Or.inl (Or.inl ⟨h1, h⟩)
      · exact Or.inr (Or.inl ⟨h1, h⟩)
      · exact Or.inl (Or.inr ⟨q, hq, h⟩)
      · exact Or.inr (Or.inr ⟨q, hq, h⟩)
end

mutual
/-- A parent comes before its child in `parented`. -/
theorem parented_parent_before (o : Option State) : ∀ (s : State) (k : Nat)
    (hk : k < (State.parented o s).length) (q : State),
    ((State.parented o s)[k]).1 = some q →
      o = some q ∨ ∃ j, ∃ hj : j < k, ((State.parented o s)[j]'(Nat.lt_trans hj hk)).2 = q
  | .mk a b el d f cs g h' i j k' l m, k, hk, q, h => by
    have ih := parentedList_parent_before (some (.mk a b el d f cs g h' i j k' l m)) cs
    have hc : (State.mk a b el d f cs g h' i j k' l m).children = cs := rfl
    generalize State.mk a b el d f cs g h' i j k' l m = s at ih hc hk h ⊢
    have e : State.parented o s = (o, s) :: State.parentedList (some s) cs := by
      rw [parented_eq, hc]
    have hk' : k < ((o, s) :: State.parentedList (some s) cs).length := e ▸ hk
    have h' : (((o, s) :: State.parentedList (some s) cs)[k]'hk').1 = some q := by
      simpa only [e] using h
    suffices o = some q ∨ ∃ j, ∃ hj : j < k,
        (((o, s) :: State.parentedList (some s) cs)[j]'(Nat.lt_trans hj hk')).2 = q by
      simpa only [e] using this
    match k, hk', h' with
    | 0, _, h' => exact Or.inl (by simpa using h')
    | k + 1, hk', h' =>
      simp only [List.getElem_cons_succ] at h'
      rcases ih k (by simpa using hk') q h' with h | ⟨j, hj, h⟩
      · cases h; exact Or.inr ⟨0, Nat.succ_pos _, rfl⟩
      · exact Or.inr ⟨j + 1, by omega, by simpa using h⟩
theorem parentedList_parent_before (o : Option State) : ∀ (cs : List State) (k : Nat)
    (hk : k < (State.parentedList o cs).length) (q : State),
    ((State.parentedList o cs)[k]).1 = some q →
      o = some q ∨ ∃ j, ∃ hj : j < k, ((State.parentedList o cs)[j]'(Nat.lt_trans hj hk)).2 = q
  | [], k, hk, _, _ => by simp [State.parentedList] at hk
  | c :: cs, k, hk, q, h => by
    have e : State.parentedList o (c :: cs) = State.parented o c ++ State.parentedList o cs := by
      rw [State.parentedList]
    have hk' : k < (State.parented o c ++ State.parentedList o cs).length := e ▸ hk
    have h' : ((State.parented o c ++ State.parentedList o cs)[k]'hk').1 = some q := by
      simpa only [e] using h
    suffices o = some q ∨ ∃ j, ∃ hj : j < k,
        ((State.parented o c ++ State.parentedList o cs)[j]'(Nat.lt_trans hj hk')).2 = q by
      simpa only [e] using this
    by_cases hlt : k < (State.parented o c).length
    · rw [List.getElem_append_left hlt] at h'
      rcases parented_parent_before o c k hlt q h' with h | ⟨j, hj, h⟩
      · exact Or.inl h
      · refine Or.inr ⟨j, hj, ?_⟩
        rw [List.getElem_append_left (by omega)]; exact h
    · rw [List.getElem_append_right (by omega)] at h'
      have hk2 : k - (State.parented o c).length < (State.parentedList o cs).length := by
        simp only [List.length_append] at hk'; omega
      rcases parentedList_parent_before o cs _ hk2 q h' with h | ⟨j, hj, h⟩
      · exact Or.inl h
      · refine Or.inr ⟨j + (State.parented o c).length, by omega, ?_⟩
        rw [List.getElem_append_right (by omega)]
        simpa using h
end

mutual
theorem flatten_sub_children : ∀ (s x : State), x ∈ s.flatten →
    (State.flattenList x.children).Sublist s.flatten
  | .mk a b el d f cs g h i j k l m, x, hx => by
    have ih := flattenList_sub_children cs x
    have hc : (State.mk a b el d f cs g h i j k l m).children = cs := rfl
    generalize State.mk a b el d f cs g h i j k l m = s at ih hc hx ⊢
    rw [flatten_eq, hc] at hx ⊢
    rcases List.mem_cons.1 hx with rfl | hx
    · rw [hc]; exact List.sublist_cons_self _ _
    · exact (ih hx).cons _
theorem flattenList_sub_children : ∀ (cs : List State) (x : State), x ∈ State.flattenList cs →
    (State.flattenList x.children).Sublist (State.flattenList cs)
  | [], _, hx => by simp [State.flattenList] at hx
  | c :: cs, x, hx => by
    rw [State.flattenList] at hx ⊢
    rcases List.mem_append.1 hx with hx | hx
    · exact (flatten_sub_children c x hx).trans (List.sublist_append_left _ _)
    · exact (flattenList_sub_children cs x hx).trans (List.sublist_append_right _ _)
end

theorem children_sub : ∀ cs : List State, cs.Sublist (State.flattenList cs)
  | [] => by simp [State.flattenList]
  | c :: cs => by
    rw [flattenList_cons, flatten_eq, List.cons_append]
    exact (children_sub cs).trans (List.sublist_append_right _ _) |>.cons_cons c

/-- Injectivity on a list whose image is duplicate-free. -/
theorem inj_of_nodup_map {α β : Type} (f : α → β) :
    ∀ {l : List α}, (l.map f).Nodup → ∀ {a b}, a ∈ l → b ∈ l → f a = f b → a = b
  | [], _, _, _, ha, _, _ => by simp at ha
  | x :: l, hnd, a, b, ha, hb, he => by
    rw [List.map_cons, List.nodup_cons] at hnd
    rcases List.mem_cons.1 ha with ha' | ha'
    · rcases List.mem_cons.1 hb with hb' | hb'
      · rw [ha', hb']
      · subst ha'; exact absurd (he ▸ List.mem_map_of_mem hb') hnd.1
    · rcases List.mem_cons.1 hb with hb' | hb'
      · subst hb'; exact absurd (he.symm ▸ List.mem_map_of_mem ha') hnd.1
      · exact inj_of_nodup_map f hnd.2 ha' hb' he

theorem flatten_sub_of_mem {s : State} : ∀ {l : List State}, s ∈ l →
    s.flatten.Sublist (State.flattenList l)
  | [], h => by simp at h
  | d :: ds, h => by
    rw [flattenList_cons]
    rcases List.mem_cons.1 h with rfl | h
    · exact List.sublist_append_left _ _
    · exact (flatten_sub_of_mem h).trans (List.sublist_append_right _ _)

/-! ## Charts with unique identifiers -/

section
variable (c : Chart)

/-- The preorder position of a state in the compiled chart (the root is 0). -/
def pos (s : State) : Nat := c.ids.idxOf s.id + 1

theorem parented_map_snd : c.parented.map Prod.snd = c.allStates := parentedList_snd _ _

theorem length_parented : c.parented.length = c.allStates.length := by
  rw [← parented_map_snd, List.length_map]

theorem ids_eq : c.ids = c.allStates.map State.id := rfl

theorem length_ids : c.ids.length = c.allStates.length := by simp [Chart.ids]

theorem parented_getElem_snd (k : Nat) (hk : k < c.parented.length) :
    (c.parented[k]).2 = c.allStates[k]'(by rw [← length_parented]; exact hk) := by
  have := congrArg (fun l => l[k]?) (parented_map_snd c)
  simp only [List.getElem?_map] at this
  rw [List.getElem?_eq_getElem hk, List.getElem?_eq_getElem] at this
  · simpa using this

theorem mem_parented_iff (e : Option State × State) :
    e ∈ c.parented ↔ (e.1 = none ∧ e.2 ∈ c.states) ∨
      ∃ q ∈ c.allStates, e.1 = some q ∧ e.2 ∈ q.children :=
  mem_parentedList none c.states e

theorem states_sub : c.states.Sublist c.allStates := children_sub _

theorem mem_of_child {q t : State} (hq : q ∈ c.allStates) (ht : t ∈ q.children) :
    t ∈ c.allStates :=
  (flattenList_sub_children _ q hq).subset ((children_sub _).subset ht)

theorem desc_sub {q : State} (hq : q ∈ c.allStates) :
    (State.flattenList q.children).Sublist c.allStates :=
  flattenList_sub_children _ q hq

variable {c}

/-- The parent entry of a state. -/
theorem parented_entry {s : State} (hs : s ∈ c.allStates) :
    ∃ o, (o, s) ∈ c.parented := by
  rw [← parented_map_snd] at hs
  obtain ⟨⟨o, t⟩, he, rfl⟩ := List.mem_map.1 hs
  exact ⟨o, he⟩

theorem mem_parented_of_child {q t : State} (hq : q ∈ c.allStates) (ht : t ∈ q.children) :
    (some q, t) ∈ c.parented :=
  (mem_parented_iff c _).2 (Or.inr ⟨q, hq, rfl, ht⟩)

theorem pos_lt {s : State} (hs : s ∈ c.allStates) : pos c s ≤ c.allStates.length := by
  unfold pos
  have := List.idxOf_lt_length_of_mem (List.mem_map_of_mem (f := State.id) hs)
  rw [← ids_eq, length_ids] at this; omega

variable (hnd : c.ids.Nodup)
include hnd

theorem state_inj {s t : State} (hs : s ∈ c.allStates) (ht : t ∈ c.allStates)
    (h : s.id = t.id) : s = t :=
  inj_of_nodup_map State.id hnd hs ht h

theorem parented_inj {e e' : Option State × State} (he : e ∈ c.parented) (he' : e' ∈ c.parented)
    (h : e.2.id = e'.2.id) : e = e' := by
  have : (c.parented.map (fun e => e.2.id)).Nodup := by
    have : c.parented.map (fun e => e.2.id) = (c.parented.map Prod.snd).map State.id := by
      simp [List.map_map, Function.comp_def]
    rw [this, parented_map_snd]; exact hnd
  exact inj_of_nodup_map (fun e => e.2.id) this he he' h

theorem pos_getElem (i : Nat) (hi : i < c.allStates.length) : pos c c.allStates[i] = i + 1 := by
  unfold pos
  have : c.allStates[i].id = c.ids[i]'(by rw [length_ids]; exact hi) := by simp [Chart.ids]
  rw [this, List.Nodup.idxOf_getElem hnd]

theorem getElem_pos {s : State} (hs : s ∈ c.allStates) :
    c.allStates[pos c s - 1]'(by have := pos_lt hs; unfold pos at *; omega) = s := by
  obtain ⟨i, hi, rfl⟩ := List.getElem_of_mem hs
  simp only [pos_getElem hnd i hi, Nat.add_sub_cancel]

theorem pos_inj {s t : State} (hs : s ∈ c.allStates) (ht : t ∈ c.allStates)
    (h : pos c s = pos c t) : s = t := by
  rw [← getElem_pos hnd hs, ← getElem_pos hnd ht]
  simp only [h]

theorem idx_eq {s : State} (hs : s ∈ c.allStates) : Compile.idx c.ids s.id = some (pos c s) := by
  unfold Compile.idx pos
  have hm : s.id ∈ c.ids := List.mem_map_of_mem hs
  have hlt := List.idxOf_lt_length_of_mem hm
  have : c.ids.idxOf? s.id = some (c.ids.idxOf s.id) := by
    rw [List.idxOf?_eq_some_iff]
    refine ⟨hlt, List.getElem_idxOf hlt, fun j hj he => ?_⟩
    have := List.Nodup.idxOf_getElem hnd j (Nat.lt_trans hj hlt)
    rw [he] at this; omega
  rw [this]; rfl

theorem idx_some {x : String} {j : Nat} (h : Compile.idx c.ids x = some j) :
    ∃ s ∈ c.allStates, s.id = x ∧ pos c s = j := by
  unfold Compile.idx at h
  cases hx : c.ids.idxOf? x with
  | none => rw [hx] at h; cases h
  | some i =>
    rw [hx] at h; cases h
    obtain ⟨hi, he, _⟩ := List.idxOf?_eq_some_iff.1 hx
    have hi' : i < c.allStates.length := by rw [← length_ids]; exact hi
    refine ⟨c.allStates[i], List.getElem_mem _, ?_, pos_getElem hnd i hi'⟩
    rw [← he]; simp [Chart.ids]

theorem idx_isSome {x : String} (hx : x ∈ c.ids) : ∃ s ∈ c.allStates, s.id = x ∧
    Compile.idx c.ids x = some (pos c s) := by
  obtain ⟨s, hs, rfl⟩ := List.mem_map.1 hx
  exact ⟨s, hs, rfl, idx_eq hnd hs⟩

theorem find_eq {s : State} (hs : s ∈ c.allStates) : c.find s.id = some s := by
  unfold Chart.find
  cases h : c.allStates.find? (fun x => x.id == s.id) with
  | none =>
    rw [List.find?_eq_none] at h
    exact absurd (by simp) (h s hs)
  | some t =>
    have ht := List.mem_of_find?_eq_some h
    have := List.find?_some h
    simp only [beq_iff_eq] at this
    rw [state_inj hnd ht hs this]

theorem parentOf_eq {o : Option State} {s : State} (h : (o, s) ∈ c.parented) :
    c.parentOf s = o := by
  unfold Chart.parentOf
  cases hf : c.parented.find? (fun p => p.2.id == s.id) with
  | none =>
    rw [List.find?_eq_none] at hf
    exact absurd (by simp) (hf _ h)
  | some e =>
    have he := List.mem_of_find?_eq_some hf
    have := List.find?_some hf
    simp only [beq_iff_eq] at this
    rw [parented_inj hnd he h this]; rfl

theorem parentOf_child {q t : State} (hq : q ∈ c.allStates) (ht : t ∈ q.children) :
    c.parentOf t = some q :=
  parentOf_eq hnd (mem_parented_of_child hq ht)

theorem parentOf_top {t : State} (ht : t ∈ c.states) : c.parentOf t = none :=
  parentOf_eq hnd ((mem_parented_iff c _).2 (Or.inl ⟨rfl, ht⟩))

theorem parentOf_some {s q : State} (hs : s ∈ c.allStates) (h : c.parentOf s = some q) :
    q ∈ c.allStates ∧ s ∈ q.children := by
  obtain ⟨o, ho⟩ := parented_entry hs
  rw [parentOf_eq hnd ho] at h; subst h
  rcases (mem_parented_iff c _).1 ho with ⟨h, _⟩ | ⟨q', hq', h, hc⟩
  · cases h
  · cases h; exact ⟨hq', hc⟩

theorem parentOf_none {s : State} (hs : s ∈ c.allStates) (h : c.parentOf s = none) :
    s ∈ c.states := by
  obtain ⟨o, ho⟩ := parented_entry hs
  rw [parentOf_eq hnd ho] at h; subst h
  rcases (mem_parented_iff c _).1 ho with ⟨_, h⟩ | ⟨_, _, h, _⟩
  · exact h
  · cases h

/-- The parent of a state comes before it. -/
theorem pos_parent_lt {s q : State} (hs : s ∈ c.allStates) (h : c.parentOf s = some q) :
    pos c q < pos c s := by
  obtain ⟨o, ho⟩ := parented_entry hs
  have h' := parentOf_eq hnd ho
  rw [h] at h'; subst h'
  obtain ⟨k, hk, he⟩ := List.getElem_of_mem ho
  rcases parentedList_parent_before none c.states k hk q (by
      show (c.parented[k]).1 = some q; rw [he]) with h | ⟨j, hj, hjq⟩
  · cases h
  · have hjq' : (c.parented[j]'(Nat.lt_trans hj hk)).2 = q := hjq
    rw [parented_getElem_snd] at hjq'
    have hs' : (c.parented[k]).2 = s := by rw [he]
    rw [parented_getElem_snd] at hs'
    rw [← hjq', ← hs', pos_getElem hnd, pos_getElem hnd]; omega

/-- A state is not its own proper descendant. -/
theorem not_mem_desc_self {s : State} (hs : s ∈ c.allStates) :
    s ∉ State.flattenList s.children := by
  intro h
  -- `flatten s` is a sublist of the chart's states, so its identifiers are distinct
  have hfs : s.flatten.Sublist c.allStates := by
    obtain ⟨o, ho⟩ := parented_entry hs
    rcases (mem_parented_iff c _).1 ho with ⟨_, ht⟩ | ⟨q, hq, _, hc⟩
    · exact flatten_sub_of_mem ht
    · exact (flatten_sub_of_mem hc).trans (desc_sub c hq)
  have hnd' : (s.flatten.map State.id).Nodup := hnd.sublist (hfs.map _)
  rw [flatten_eq, List.map_cons, List.nodup_cons] at hnd'
  exact hnd'.1 (List.mem_map_of_mem h)

end

end Vertep.CompileProof
