import VertepLean.Rdf.Equiv.Theorem
import VertepLean.Config.CompileRank
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false
set_option linter.unusedSectionVars false

/-!
# Paths from the top of the chart

`WP c` lists every state of a chart with its path from a top-level state down to itself. The
projector's `indexStates` is a map over it (`index_states`, `chains_eq`, `histories_eq`), and on a
chart with unique state identifiers a path is determined by its last state (`wp_uniq`,
`wp_prefix`), lists the state's proper ancestors (`wp_desc`, `wp_ancestors`) and its parent
(`wp_parent_some`, `wp_parent_none`).
-/

namespace Vertep.Rdf.Equiv

open LeanRDF Vertep.Rdf.RT

/-! ## Paths from the top of the chart -/

mutual
/-- Every state of a subtree with its path from the top (`pre` is the path above the subtree). -/
def withPath (pre : List State) : State → List (State × List State)
  | s@(.mk _ _ _ _ _ cs ..) => (s, pre ++ [s]) :: withPathList (pre ++ [s]) cs
/-- `withPath` over siblings. -/
def withPathList (pre : List State) : List State → List (State × List State)
  | [] => []
  | c :: cs => withPath pre c ++ withPathList pre cs
end

theorem withPath_eq (pre : List State) (s : State) :
    withPath pre s = (s, pre ++ [s]) :: withPathList (pre ++ [s]) s.children := by
  cases s; rfl

/-- The paths of the chart's states. -/
def WP (c : Chart) : List (State × List State) := withPathList [] c.states

/-- A selection-chain entry of a state. -/
def fr (s : State) : String × Bool := (stateFragment s.id, s.element == .parallel)

/-- The chain entry `indexState` records for a state with chain `ch` (itself included). -/
def entryOf (ch : List (String × Bool)) (s : State) : List (String × Bool) :=
  match s.element with
  | .history _ => if ch.length > 2 then ch.dropLast else []
  | _ => ch

mutual
theorem index_state (B : List (String × Bool)) (pre : List State) : ∀ s : State,
    indexState (B ++ pre.map fr) s =
      ((withPath pre s).map (fun e => (e.1.id, entryOf (B ++ e.2.map fr) e.1)),
       s.flatten.map (fun s => (s.id, s.isHistory)))
  | .mk id ig el ini ie cs trs en ex da inv dd xs => by
    have hch : B ++ pre.map fr ++ [(stateFragment id, el == .parallel)] =
        B ++ (pre ++ [State.mk id ig el ini ie cs trs en ex da inv dd xs]).map fr := by
      simp [fr, State.id, State.element]
    have ih := index_states B (pre ++ [State.mk id ig el ini ie cs trs en ex da inv dd xs]) cs
    rw [← hch] at ih
    rw [withPath_eq, flatten_eq]
    cases el <;> simp only [indexState, ih, List.map_cons, entryOf, State.element, State.isHistory, State.id,
      State.children] <;> simp <;> rfl
theorem index_states (B : List (String × Bool)) (pre : List State) : ∀ cs : List State,
    indexStates (B ++ pre.map fr) cs =
      ((withPathList pre cs).map (fun e => (e.1.id, entryOf (B ++ e.2.map fr) e.1)),
       (State.flattenList cs).map (fun s => (s.id, s.isHistory)))
  | [] => by simp [indexStates, withPathList, State.flattenList]
  | c :: cs => by
    rw [indexStates, withPathList, flattenList_cons, index_state B pre c, index_states B pre cs]
    simp
end

theorem chains_eq (c : Chart) (base : String) :
    (ctxOf c base).chains = (WP c).map (fun e => (e.1.id, entryOf (("chart", false) :: e.2.map fr) e.1)) := by
  have := index_states [("chart", false)] [] c.states
  simp only [List.map_nil, List.append_nil] at this
  simp [ctxOf, this, WP]

theorem histories_eq (c : Chart) (base : String) :
    (ctxOf c base).histories = c.allStates.map (fun s => (s.id, s.isHistory)) := by
  have := index_states [("chart", false)] [] c.states
  simp only [List.map_nil, List.append_nil] at this
  simp [ctxOf, this, Chart.allStates]

/-! ## Lookups -/

theorem lookup_of_mem {β : Type} : ∀ {l : List (String × β)}, (l.map Prod.fst).Nodup →
    ∀ {k : String} {v : β}, (k, v) ∈ l → l.lookup k = some v
  | [], _, _, _, h => by simp at h
  | (a, b) :: l, hnd, k, v, h => by
    simp only [List.map_cons, List.nodup_cons] at hnd
    rcases List.mem_cons.1 h with h | h
    · simp only [Prod.mk.injEq] at h; obtain ⟨rfl, rfl⟩ := h
      simp [List.lookup]
    · have hka : k ≠ a := fun hk => hnd.1 (hk ▸ List.mem_map_of_mem (f := Prod.fst) h)
      simp only [List.lookup, show (k == a) = false from by simp [hka]]
      exact lookup_of_mem hnd.2 h

theorem lookup_none {β : Type} : ∀ {l : List (String × β)} {k : String}, k ∉ l.map Prod.fst →
    l.lookup k = none
  | [], _, _ => rfl
  | (a, b) :: l, k, h => by
    simp only [List.map_cons, List.mem_cons, not_or] at h
    simp only [List.lookup, show (k == a) = false from by simp [h.1]]
    exact lookup_none h.2

theorem lookup_mem {β : Type} : ∀ {l : List (String × β)} {k : String} {v : β}, l.lookup k = some v →
    (k, v) ∈ l
  | [], _, _, h => by simp [List.lookup] at h
  | (a, b) :: l, k, v, h => by
    by_cases hk : k = a
    · subst hk; simp [List.lookup] at h; subst h; exact List.mem_cons_self
    · simp only [List.lookup, show (k == a) = false from by simp [hk]] at h
      exact List.mem_cons_of_mem _ (lookup_mem h)

/-! ## Shape of the paths -/

/-- Each state of the list is a child of the one before. -/
def Down : List State → Prop
  | [] => True
  | [_] => True
  | a :: b :: l => b ∈ a.children ∧ Down (b :: l)

theorem down_cons {a : State} {q : List State} (hq : ∀ h, q.head? = some h → h ∈ a.children)
    (hd : Down q) : Down (a :: q) := by
  cases q with
  | nil => trivial
  | cons b l => exact ⟨hq b rfl, hd⟩

mutual
theorem withPath_fst (pre : List State) : ∀ s : State, (withPath pre s).map Prod.fst = s.flatten
  | .mk id ig el ini ie cs trs en ex da inv dd xs => by
    rw [withPath_eq, flatten_eq, List.map_cons, withPathList_fst]
theorem withPathList_fst (pre : List State) : ∀ cs : List State,
    (withPathList pre cs).map Prod.fst = State.flattenList cs
  | [] => by simp [withPathList, State.flattenList]
  | c :: cs => by
    rw [withPathList, flattenList_cons, List.map_append, withPath_fst, withPathList_fst]
end

theorem WP_fst (c : Chart) : (WP c).map Prod.fst = c.allStates := withPathList_fst [] c.states

mutual
theorem withPath_shape (pre : List State) : ∀ s : State, ∀ e ∈ withPath pre s,
    ∃ q, e.2 = pre ++ q ∧ Down q ∧ q.head? = some s ∧ q.getLast? = some e.1
  | .mk id ig el ini ie cs trs en ex da inv dd xs, e, he => by
    rw [withPath_eq] at he
    rcases List.mem_cons.1 he with rfl | he
    · exact ⟨[_], rfl, trivial, rfl, rfl⟩
    · obtain ⟨q, hq, hd, hh, hl⟩ := withPathList_shape _ cs e he
      refine ⟨_ :: q, by simp [hq], down_cons (fun h hh' => ?_) hd.1, rfl, ?_⟩
      · obtain ⟨c', hc', hh2⟩ := hd.2; rw [hh2] at hh'; cases hh'; exact hc'
      · cases q with
        | nil => simp at hh
        | cons a l => simpa [List.getLast?_cons_cons] using hl
theorem withPathList_shape (pre : List State) : ∀ cs : List State, ∀ e ∈ withPathList pre cs,
    ∃ q, e.2 = pre ++ q ∧ (Down q ∧ ∃ c ∈ cs, q.head? = some c) ∧ q.head? ≠ none ∧ q.getLast? = some e.1
  | [], e, he => by simp [withPathList] at he
  | c :: cs, e, he => by
    rw [withPathList, List.mem_append] at he
    rcases he with he | he
    · obtain ⟨q, h1, h2, h3, h4⟩ := withPath_shape pre c e he
      exact ⟨q, h1, ⟨h2, c, by simp, h3⟩, by simp [h3], h4⟩
    · obtain ⟨q, h1, ⟨h2, c', hc', h3⟩, h3', h4⟩ := withPathList_shape pre cs e he
      exact ⟨q, h1, ⟨h2, c', by simp [hc'], h3⟩, h3', h4⟩
end

mutual
theorem withPath_closed (pre : List State) : ∀ s : State, ∀ e ∈ withPath pre s,
    ∀ k (hk : k < e.2.length), pre.length ≤ k → (e.2[k], e.2.take (k + 1)) ∈ withPath pre s
  | .mk id ig el ini ie cs trs en ex da inv dd xs, e, he, k, hk, hpk => by
    rw [withPath_eq] at he ⊢
    rcases List.mem_cons.1 he with rfl | he
    · simp only [List.length_append, List.length_singleton] at hk
      have : k = pre.length := by omega
      subst this
      refine List.mem_cons.2 (.inl ?_)
      refine Prod.ext ?_ ?_
      · simp
      · exact List.take_of_length_le (by simp)
    · obtain ⟨q, hq, _, _, _⟩ := withPathList_shape _ cs e he
      by_cases hk' : pre.length + 1 ≤ k
      · exact List.mem_cons_of_mem _ (withPathList_closed _ cs e he k hk (by simpa using hk'))
      · have : k = pre.length := by omega
        subst this
        rcases e with ⟨e1, e2⟩
        dsimp only at hq hk ⊢
        subst hq
        refine List.mem_cons.2 (.inl (Prod.ext ?_ ?_))
        · simp [List.getElem_append_left, List.getElem_append_right]
        · dsimp only
          have hl : pre.length + 1 = (pre ++ [State.mk id ig el ini ie cs trs en ex da inv dd xs]).length := by
            simp
          rw [hl, List.take_left]
theorem withPathList_closed (pre : List State) : ∀ cs : List State, ∀ e ∈ withPathList pre cs,
    ∀ k (hk : k < e.2.length), pre.length ≤ k → (e.2[k], e.2.take (k + 1)) ∈ withPathList pre cs
  | [], e, he, _, _, _ => by simp [withPathList] at he
  | c :: cs, e, he, k, hk, hpk => by
    rw [withPathList, List.mem_append] at he ⊢
    rcases he with he | he
    · exact .inl (withPath_closed pre c e he k hk hpk)
    · exact .inr (withPathList_closed pre cs e he k hk hpk)
end

mutual
theorem withPath_sub (pre : List State) : ∀ s : State, ∀ e ∈ withPath pre s,
    ∀ e' ∈ withPathList e.2 e.1.children, e' ∈ withPath pre s
  | .mk id ig el ini ie cs trs en ex da inv dd xs, e, he, e', he' => by
    rw [withPath_eq] at he ⊢
    rcases List.mem_cons.1 he with rfl | he
    · exact List.mem_cons_of_mem _ he'
    · exact List.mem_cons_of_mem _ (withPathList_sub _ cs e he e' he')
theorem withPathList_sub (pre : List State) : ∀ cs : List State, ∀ e ∈ withPathList pre cs,
    ∀ e' ∈ withPathList e.2 e.1.children, e' ∈ withPathList pre cs
  | [], e, he, _, _ => by simp [withPathList] at he
  | c :: cs, e, he, e', he' => by
    rw [withPathList, List.mem_append] at he ⊢
    rcases he with he | he
    · exact .inl (withPath_sub pre c e he e' he')
    · exact .inr (withPathList_sub pre cs e he e' he')
end

section
variable {c : Chart}

theorem wp_shape {e : State × List State} (he : e ∈ WP c) :
    Down e.2 ∧ (∃ t ∈ c.states, e.2.head? = some t) ∧ e.2.getLast? = some e.1 := by
  obtain ⟨q, h1, ⟨h2, t, ht, h3⟩, _, h4⟩ := withPathList_shape [] c.states e he
  simp only [List.nil_append] at h1
  subst h1
  exact ⟨h2, ⟨t, ht, h3⟩, h4⟩

theorem wp_ne_nil {e : State × List State} (he : e ∈ WP c) : e.2 ≠ [] := by
  intro h; have := (wp_shape he).2.2; rw [h] at this; cases this

theorem wp_closed {e : State × List State} (he : e ∈ WP c) (k : Nat) (hk : k < e.2.length) :
    (e.2[k], e.2.take (k + 1)) ∈ WP c :=
  withPathList_closed [] c.states e he k hk (Nat.zero_le _)

theorem wp_mem {e : State × List State} (he : e ∈ WP c) : e.1 ∈ c.allStates := by
  rw [← WP_fst]; exact List.mem_map_of_mem he

theorem wp_elems {e : State × List State} (he : e ∈ WP c) : ∀ x ∈ e.2, x ∈ c.allStates := by
  intro x hx
  obtain ⟨k, hk, rfl⟩ := List.getElem_of_mem hx
  exact wp_mem (wp_closed he k hk)

theorem wp_of_mem {s : State} (hs : s ∈ c.allStates) : ∃ p, (s, p) ∈ WP c := by
  rw [← WP_fst] at hs
  obtain ⟨⟨s', p⟩, hp, rfl⟩ := List.mem_map.1 hs
  exact ⟨p, hp⟩

theorem down_desc : ∀ (p : List State), Down p → ∀ (k : Nat) (hk : k + 1 < p.length),
    p.getLast (List.ne_nil_of_length_pos (by omega)) ∈ State.flattenList p[k].children
  | [], _, k, hk => by simp at hk
  | [_], _, k, hk => by simp at hk
  | a :: b :: l, ⟨hab, hd⟩, k, hk => by
    have hb : b.flatten.Sublist (State.flattenList a.children) := CompileProof.flatten_sub_of_mem hab
    rcases k with _ | k
    · simp only [List.getElem_cons_zero]
      rw [List.getLast_cons (by simp)]
      cases l with
      | nil => simp only [List.getLast_singleton]; exact hb.subset (by rw [flatten_eq]; exact List.mem_cons_self)
      | cons x l =>
        have := down_desc (b :: x :: l) hd 0 (by simp)
        simp only [List.getElem_cons_zero] at this
        refine hb.subset ?_
        rw [flatten_eq]; exact List.mem_cons_of_mem _ this
    · simp only [List.getElem_cons_succ]
      rw [List.getLast_cons (by simp)]
      exact down_desc (b :: l) hd k (by simp at hk ⊢; omega)

theorem down_child : ∀ (p : List State), Down p → ∀ (k : Nat) (hk : k + 1 < p.length),
    p[k + 1] ∈ p[k].children
  | [], _, k, hk => by simp at hk
  | [_], _, k, hk => by simp at hk
  | a :: b :: l, ⟨hab, hd⟩, k, hk => by
    rcases k with _ | k
    · exact hab
    · exact down_child (b :: l) hd k (by simp at hk ⊢; omega)

variable (hnd : c.ids.Nodup)
include hnd

theorem wp_uniq {e e' : State × List State} (he : e ∈ WP c) (he' : e' ∈ WP c) (h : e.1.id = e'.1.id) :
    e = e' := by
  have : ((WP c).map (fun e => e.1.id)).Nodup := by
    have := hnd; rw [Chart.ids, ← WP_fst, List.map_map] at this; exact this
  exact CompileProof.inj_of_nodup_map _ this he he' h

theorem wp_last {e : State × List State} (he : e ∈ WP c) :
    e.2.getLast (wp_ne_nil he) = e.1 := by
  have := (wp_shape he).2.2
  rw [List.getLast?_eq_some_getLast (wp_ne_nil he)] at this
  exact Option.some.inj this

/-- **A state's identifier is below `W` exactly when `W` is on its path, before its end.** -/
theorem wp_desc {e : State × List State} (he : e ∈ WP c) {W : State} (hW : W ∈ c.allStates) :
    e.1.id ∈ W.descIds ↔ ∃ k, ∃ hk : k + 1 < e.2.length, e.2[k] = W := by
  constructor
  · intro hd
    obtain ⟨pW, hpW⟩ := wp_of_mem hW
    simp only [State.descIds, List.mem_map] at hd
    obtain ⟨T, hT, hTid⟩ := hd
    rw [← withPathList_fst pW W.children] at hT
    obtain ⟨e', he', rfl⟩ := List.mem_map.1 hT
    have he'' := withPathList_sub [] c.states (W, pW) hpW e' he'
    have := wp_uniq hnd he'' he hTid
    subst this
    obtain ⟨q, hq, _, hh, _⟩ := withPathList_shape _ _ e' he'
    have hpW' := wp_ne_nil hpW
    refine ⟨pW.length - 1, ?_, ?_⟩
    · rw [hq]; simp only [List.length_append]
      have : q.length ≠ 0 := fun h => hh (by simp [List.length_eq_zero_iff.1 h])
      have : pW.length ≠ 0 := fun h => hpW' (List.length_eq_zero_iff.1 h)
      omega
    · have hl := wp_last hnd hpW
      simp only [hq]
      rw [List.getElem_append_left (by have : pW.length ≠ 0 := fun h => hpW' (List.length_eq_zero_iff.1 h); omega)]
      dsimp only at hl
      rw [List.getLast_eq_getElem] at hl
      exact hl
  · rintro ⟨k, hk, rfl⟩
    have := down_desc e.2 (wp_shape he).1 k hk
    rw [wp_last hnd he] at this
    exact List.mem_map_of_mem this

/-- **Two paths through one state agree up to it.** -/
theorem wp_prefix {e e' : State × List State} (he : e ∈ WP c) (he' : e' ∈ WP c) {i j : Nat}
    (hi : i < e.2.length) (hj : j < e'.2.length) (h : e.2[i].id = e'.2[j].id) :
    i = j ∧ e.2.take (i + 1) = e'.2.take (j + 1) := by
  have := wp_uniq hnd (wp_closed he i hi) (wp_closed he' j hj) h
  simp only [Prod.mk.injEq] at this
  refine ⟨?_, this.2⟩
  have h2 := congrArg List.length this.2
  simp only [List.length_take] at h2
  omega

theorem wp_nodup {e : State × List State} (he : e ∈ WP c) : e.2.Nodup := by
  refine List.pairwise_iff_getElem.2 fun i j hi hj hij heq => ?_
  have := (wp_prefix hnd he he hi hj (congrArg State.id heq)).1
  omega

theorem wp_length {e : State × List State} (he : e ∈ WP c) : e.2.length ≤ c.allStates.length :=
  (wp_nodup hnd he).length_le_of_subset (wp_elems he)

theorem wp_parent_some {e : State × List State} (he : e ∈ WP c) {k : Nat} (hk : k + 2 = e.2.length) :
    c.parentOf e.1 = some (e.2[k]) := by
  have h1 := down_child e.2 (wp_shape he).1 k (by omega)
  have hl := wp_last hnd he
  rw [List.getLast_eq_getElem] at hl
  have : e.2[k + 1] = e.1 := by rw [← hl]; congr 1; omega
  rw [this] at h1
  exact CompileProof.parentOf_child hnd (wp_elems he _ (List.getElem_mem _)) h1

theorem wp_parent_none {e : State × List State} (he : e ∈ WP c) (hk : e.2.length = 1) :
    c.parentOf e.1 = none := by
  obtain ⟨_, ⟨t, ht, hh⟩, hl⟩ := wp_shape he
  match h : e.2, hk with
  | [x], _ =>
    rw [h] at hh hl
    simp only [List.head?_cons, Option.some.injEq, List.getLast?_singleton] at hh hl
    subst hh; rw [← hl]
    exact CompileProof.parentOf_top hnd ht

theorem wp_ancestorsF : ∀ (n : Nat) (e : State × List State), e ∈ WP c → e.2.length = n + 1 →
    ∀ f, n ≤ f → c.ancestorsF f e.1 = e.2.dropLast.reverse
  | 0, e, he, hl, f, _ => by
    have hp := wp_parent_none hnd he hl
    match f with
    | 0 => match h : e.2, hl with | [_], _ => simp [Chart.ancestorsF]
    | f + 1 => match h : e.2, hl with | [_], _ => simp [Chart.ancestorsF, hp]
  | n + 1, e, he, hl, f, hf => by
    obtain ⟨f, rfl⟩ : ∃ f', f = f' + 1 := ⟨f - 1, by omega⟩
    have hp := wp_parent_some hnd he (k := n) (by omega)
    have hQ := wp_closed he n (by omega)
    have ih := wp_ancestorsF n _ hQ (by simp; omega) f (by omega)
    simp only [Chart.ancestorsF, hp, ih]
    have hd : e.2.dropLast = e.2.take (n + 1) := by
      rw [List.dropLast_eq_take]; congr 1; omega
    have h3 : e.2.take (n + 1) = e.2.take n ++ [e.2[n]] :=
      List.take_succ_eq_append_getElem (show n < e.2.length by omega)
    rw [hd, h3, List.dropLast_concat, List.reverse_append]
    rfl

/-- The proper ancestors of a state, nearest first, are its path without itself, reversed. -/
theorem wp_ancestors {e : State × List State} (he : e ∈ WP c) :
    c.ancestors e.1 = e.2.dropLast.reverse := by
  have hl := wp_length hnd he
  have hne := wp_ne_nil he
  have : e.2.length ≠ 0 := fun h => hne (List.length_eq_zero_iff.1 h)
  exact wp_ancestorsF hnd (e.2.length - 1) e he (by omega) _ (by omega)

theorem chains_lookup (base : String) {e : State × List State} (he : e ∈ WP c) :
    (ctxOf c base).chains.lookup e.1.id = some (entryOf (("chart", false) :: e.2.map fr) e.1) := by
  rw [chains_eq]
  refine lookup_of_mem ?_ (List.mem_map.2 ⟨e, he, rfl⟩)
  rw [List.map_map]
  have := hnd; rw [Chart.ids, ← WP_fst, List.map_map] at this; exact this

theorem histories_lookup (base : String) (t : String) :
    (ctxOf c base).histories.lookup t = some true ↔ ∃ T ∈ c.allStates, T.id = t ∧ T.isHistory = true := by
  rw [histories_eq]
  have hn : ((c.allStates.map (fun s => (s.id, s.isHistory))).map Prod.fst).Nodup := by
    rw [List.map_map]; exact hnd
  constructor
  · intro h
    obtain ⟨T, hT, he⟩ := List.mem_map.1 (lookup_mem h)
    simp only [Prod.mk.injEq] at he
    exact ⟨T, hT, he.1, he.2⟩
  · rintro ⟨T, hT, rfl, hh⟩
    exact lookup_of_mem hn (List.mem_map.2 ⟨T, hT, by rw [hh]⟩)

end

theorem chains_lookup_none (c : Chart) (base : String) {t : String} (ht : t ∉ c.ids) :
    (ctxOf c base).chains.lookup t = none := by
  refine lookup_none fun h => ht ?_
  rw [chains_eq, List.map_map] at h
  obtain ⟨e, he, rfl⟩ := List.mem_map.1 h
  exact List.mem_map_of_mem (wp_mem he)

end Vertep.Rdf.Equiv
