import VertepLean.Rdf.Reconstruct
import Std.Data.String.ToNat

/-!
# RDF round trip: basic lemmas

Groundwork for the RDF Round Trip Theorem (formal-model, task 7.7):

* the index of a graph (`Index.of`) and the decoding primitives of `Decode` (`typed`, `string`,
  `boolean`, `integer`, `optNode`, `oneNode`, `ordered`, `hasType`) characterized by the
  objects of a subject and predicate;
* fragments: colon-free segments, the region below a fragment (`U`) and its disjointness for
  siblings and for colon-free roots;
* `percentEncode` is injective and never yields a colon.
-/

namespace Vertep.Rdf
open LeanRDF

namespace RT

theorem mem_index (ts : List Triple) (s p o : Term) :
    o ∈ ((Index.of ts).objs.getD (s, p) #[]).toList ↔ ⟨s, p, o⟩ ∈ ts := by
  suffices h : ∀ (m : Std.HashMap (Term × Term) (Array Term)) (l : List Triple),
      o ∈ ((l.foldl (fun m t => m.insert (t.s, t.p) ((m.getD (t.s, t.p) #[]).push t.o)) m).getD
        (s, p) #[]).toList ↔ o ∈ (m.getD (s, p) #[]).toList ∨ ⟨s, p, o⟩ ∈ l by
    simp [Index.of, h]
  intro m l
  induction l generalizing m with
  | nil => simp
  | cons t l ih =>
    rw [List.foldl_cons, ih, Std.HashMap.getD_insert]
    by_cases h : (t.s, t.p) = (s, p)
    · have h1 : t.s = s := (Prod.mk.inj h).1
      have h2 : t.p = p := (Prod.mk.inj h).2
      subst h1 h2
      simp only [beq_self_eq_true, ↓reduceIte, Array.toList_push, List.mem_append,
        List.mem_cons, List.not_mem_nil, or_false, List.mem_cons]
      constructor
      · rintro ((h | h) | h)
        · exact .inl h
        · exact .inr (.inl (by rw [h]))
        · exact .inr (.inr h)
      · rintro (h | h | h)
        · exact .inl (.inl h)
        · cases t; cases h; exact .inl (.inr rfl)
        · exact .inr h
    · have hb : ((t.s, t.p) == (s, p)) = false := by simpa using h
      simp only [hb, Bool.false_eq_true, ↓reduceIte, List.mem_cons]
      constructor
      · rintro (h' | h')
        · exact .inl h'
        · exact .inr (.inr h')
      · rintro (h' | h' | h')
        · exact .inl h'
        · exact absurd (by subst h'; rfl) h
        · exact .inr h'

/-! ## Decoding primitives from membership specifications -/

/-- The objects of `(s, vt p)` in `ts` are exactly `os`. -/
def Lk (ts : List Triple) (s : Term) (p : String) (os : List Term) : Prop :=
  ∀ o, ⟨s, vt p, o⟩ ∈ ts ↔ o ∈ os

variable {ts : List Triple}

theorem mem_objects (s : Term) (p : String) (o : Term) :
    o ∈ Decode.objects (Index.of ts) s p ↔ ⟨s, vt p, o⟩ ∈ ts := by
  simp only [Decode.objects, List.mem_eraseDups]; exact mem_index ..

theorem nodup_eraseDups {α : Type} [BEq α] [LawfulBEq α] : ∀ (l : List α), l.eraseDups.Nodup
  | [] => by simp
  | a :: as => by
    have : (as.filter fun b => !b == a).length < as.length + 1 :=
      Nat.lt_add_one_of_le (List.length_filter_le _ as)
    rw [List.eraseDups_cons, List.nodup_cons]
    refine ⟨?_, nodup_eraseDups _⟩
    simp
termination_by l => l.length

theorem nodup_objects (s : Term) (p : String) : (Decode.objects (Index.of ts) s p).Nodup := by
  exact nodup_eraseDups _

theorem objects_eq_nil {s : Term} {p : String} (h : Lk ts s p []) :
    Decode.objects (Index.of ts) s p = [] := by
  apply List.eq_nil_iff_forall_not_mem.2
  intro o ho
  have := (h o).1 ((mem_objects s p o).1 ho)
  simp at this

theorem objects_eq_single {s : Term} {p : String} {v : Term} (h : Lk ts s p [v]) :
    Decode.objects (Index.of ts) s p = [v] := by
  have hn := nodup_objects (ts := ts) s p
  have hm : ∀ o, o ∈ Decode.objects (Index.of ts) s p ↔ o = v := by
    intro o; rw [mem_objects, h o]; simp
  generalize Decode.objects (Index.of ts) s p = l at hn hm
  match l, hn, hm with
  | [], _, hm => exact absurd ((hm v).2 rfl) (by simp)
  | [a], _, hm => have := (hm a).1 (by simp); simp [this]
  | a :: b :: l, hn, hm =>
    have ha := (hm a).1 (by simp)
    have hb := (hm b).1 (by simp)
    simp at hn
    exact absurd (ha.trans hb.symm) hn.1.1

/-- Literal values of an optional field. -/
def litOf (dt : IRI) : Option String → List Term
  | some x => [.lit (.typed x dt)]
  | none => []

@[simp] theorem mem_litOf (dt : IRI) (v : Option String) (o : Term) :
    o ∈ litOf dt v ↔ ∃ x, v = some x ∧ o = .lit (.typed x dt) := by
  cases v <;> simp [litOf]

theorem typed_eq {s : Term} {p : String} {dt : IRI} {v : Option String}
    (h : Lk ts s p (litOf dt v)) : Decode.typed (Index.of ts) s p dt = .ok v := by
  cases v with
  | none => simp [Decode.typed, objects_eq_nil (by simpa [litOf] using h)]; rfl
  | some x => simp [Decode.typed, objects_eq_single (by simpa [litOf] using h)]; rfl

theorem optString_eq {s : Term} {p : String} {v : Option String}
    (h : Lk ts s p (litOf xsdString v)) : Decode.optString (Index.of ts) s p = .ok v :=
  typed_eq h

theorem string_eq {s : Term} {p : String} {v : String}
    (h : Lk ts s p (litOf xsdString (some v))) : Decode.string (Index.of ts) s p = .ok v := by
  simp [Decode.string, optString_eq h, bind, Except.bind, Decode.required]; rfl

theorem optExpr_eq {dt : IRI} {s : Term} {p : String} {v : Option String}
    (h : Lk ts s p (litOf dt v)) : Decode.optExpr (Index.of ts) dt s p = .ok v :=
  typed_eq h

theorem expr_eq {dt : IRI} {s : Term} {p : String} {v : String}
    (h : Lk ts s p (litOf dt (some v))) : Decode.expr (Index.of ts) dt s p = .ok v := by
  simp [Decode.expr, optExpr_eq h, bind, Except.bind, Decode.required]; rfl

theorem boolean_eq {s : Term} {p : String} {b : Bool}
    (h : Lk ts s p [boolLit b]) : Decode.boolean (Index.of ts) s p = .ok b := by
  have h' : Lk ts s p (litOf xsdBoolean (some (if b then "true" else "false"))) := by
    simpa [litOf, boolLit] using h
  cases b <;> simp [Decode.boolean, typed_eq h', bind, Except.bind, Decode.required] <;> rfl

theorem toNat?_toString (n : Nat) : (toString n).toNat? = some n := Nat.toNat?_repr n

theorem toString_not_plus (n : Nat) : ¬ ['+'] <+: Nat.toDigits 10 n := by
  rintro ⟨t, ht⟩
  have hm : '+' ∈ Nat.toDigits 10 n := by
    rw [← ht]; simp
  have := Nat.isDigit_of_mem_toDigits (by decide) (by decide) hm
  exact absurd this (by decide)

theorem integer_eq {s : Term} {p : String} {n : Nat}
    (h : Lk ts s p [intLit n]) : Decode.integer (Index.of ts) s p = .ok n := by
  have h' : Lk ts s p (litOf xsdInteger (some (toString n))) := by
    simpa [litOf, intLit] using h
  simp only [Decode.integer, typed_eq h', bind, Except.bind, Decode.required, pure, Except.pure]
  simp [Nat.toNat?_repr, toString_not_plus]

/-- `mapM` in `Except` when every call succeeds. -/
theorem mapM_ok {α β ε : Type} (f : α → Except ε β) (h : α → β) :
    ∀ (l : List α), (∀ x ∈ l, f x = .ok (h x)) → l.mapM f = .ok (l.map h)
  | [], _ => rfl
  | x :: l, hl => by
    have h1 := hl x (by simp)
    have h2 := mapM_ok f h l (fun y hy => hl y (by simp [hy]))
    simp [List.mapM_cons, h1, h2]; rfl

theorem mapM_ok_self {α ε : Type} (f : α → Except ε α) :
    ∀ (l : List α), (∀ x ∈ l, f x = .ok x) → l.mapM f = .ok l
  | [], _ => rfl
  | x :: l, hl => by
    have h1 := hl x (by simp)
    have h2 := mapM_ok_self f l (fun y hy => hl y (by simp [hy]))
    simp [List.mapM_cons, h1, h2]; rfl

def IsIri (o : Term) : Prop := ∃ i, o = .iri i

theorem nodes_eq {s : Term} {p : String} (h : ∀ o, ⟨s, vt p, o⟩ ∈ ts → IsIri o) :
    Decode.nodes (Index.of ts) s p = .ok (Decode.objects (Index.of ts) s p) := by
  unfold Decode.nodes
  apply mapM_ok_self
  intro x hx
  obtain ⟨i, rfl⟩ := h x ((mem_objects s p x).1 hx)
  rfl

theorem optNode_none {s : Term} {p : String} (h : Lk ts s p []) :
    Decode.optNode (Index.of ts) s p = .ok none := by
  have hn : Decode.nodes (Index.of ts) s p = .ok [] := by
    rw [nodes_eq (fun o ho => by simpa using (h o).1 ho), objects_eq_nil h]
  simp [Decode.optNode, hn, bind, Except.bind]; rfl

theorem optNode_some {s : Term} {p : String} {v : Term} (hv : IsIri v) (h : Lk ts s p [v]) :
    Decode.optNode (Index.of ts) s p = .ok (some v) := by
  have hn : Decode.nodes (Index.of ts) s p = .ok [v] := by
    rw [nodes_eq (fun o ho => by have := (h o).1 ho; simp at this; subst this; exact hv),
      objects_eq_single h]
  simp [Decode.optNode, hn, bind, Except.bind]; rfl

theorem oneNode_eq {s : Term} {p : String} {v : Term} (hv : IsIri v) (h : Lk ts s p [v]) :
    Decode.oneNode (Index.of ts) s p = .ok v := by
  simp [Decode.oneNode, optNode_some hv h, bind, Except.bind]; rfl

theorem hasType_eq (s : Term) (cls : String) :
    Decode.hasType (Index.of ts) s cls = decide (⟨s, rdfType, vt cls⟩ ∈ ts) := by
  unfold Decode.hasType
  rw [Bool.eq_iff_iff]
  simp only [Array.contains_iff_mem, decide_eq_true_eq]
  rw [← Array.mem_toList_iff, mem_index]

theorem forIn_ok {α ε : Type} (f : α → PUnit → Except ε (ForInStep PUnit)) :
    ∀ (l : List α), (∀ x ∈ l, f x PUnit.unit = .ok (.yield PUnit.unit)) →
      forIn l PUnit.unit f = .ok PUnit.unit
  | [], _ => rfl
  | x :: l, hl => by
    rw [List.forIn_cons, hl x (by simp)]
    exact forIn_ok f l (fun y hy => hl y (by simp [hy]))

theorem mergeSort_unique {α : Type} (le : α → α → Bool)
    (trans : ∀ a b c, le a b → le b c → le a c) (total : ∀ a b, (le a b || le b a) = true)
    (P Q : List α) (hp : P.Perm Q) (hQ : Q.Pairwise (fun a b => le a b))
    (anti : ∀ a b, a ∈ Q → b ∈ Q → le a b → le b a → a = b) : P.mergeSort le = Q := by
  apply List.Perm.eq_of_pairwise (le := fun a b => le a b)
  · intro a b ha hb h1 h2
    exact anti a b (((List.mergeSort_perm P le).trans hp).subset ha) hb h1 h2
  · exact List.pairwise_mergeSort trans total P
  · exact hQ
  · exact (List.mergeSort_perm P le).trans hp

-- @tg: implements:: [[openspec:formal-model#RDF Round Trip Theorem]] {note: "lemma: ordered collections decode in document order"}
theorem ordered_eq {s : Term} {p : String} {M : List Term}
    (hM : Lk ts s p M) (hnd : M.Nodup) (hiri : ∀ m ∈ M, IsIri m)
    (hord : ∀ j (hj : j < M.length), Lk ts M[j] "order" [intLit j]) :
    Decode.ordered (Index.of ts) s p = .ok M := by
  have hn := nodes_eq (ts := ts) (s := s) (p := p) (fun o ho => hiri o ((hM o).1 ho))
  have hperm : (Decode.objects (Index.of ts) s p).Perm M := by
    rw [List.perm_ext_iff_of_nodup (nodup_objects s p) hnd]
    intro a; rw [mem_objects, hM a]
  have hidx : ∀ x ∈ M, Decode.integer (Index.of ts) x "order" = .ok (M.idxOf x) := by
    intro x hx
    have hl : M.idxOf x < M.length := List.idxOf_lt_length_of_mem hx
    have := hord _ hl
    rw [List.getElem_idxOf hl] at this
    exact integer_eq this
  have hQ : ∀ j (hj : j < M.length), M.idxOf M[j] = j := fun j hj =>
    List.Nodup.idxOf_getElem hnd j hj
  unfold Decode.ordered
  simp only [hn, bind, Except.bind]
  rw [mapM_ok _ (fun n => (M.idxOf n, n)) _ (fun x hx => by
    rw [hidx x (hperm.subset hx)]; rfl)]
  simp only
  rw [mergeSort_unique _ ?trans ?total _ (M.map (fun n => (M.idxOf n, n))) (hperm.map _) ?sorted ?anti]
  case trans =>
    intro a b c h1 h2
    simp only [Bool.or_eq_true, decide_eq_true_eq, Bool.and_eq_true, beq_iff_eq, bne_iff_ne,
      ne_eq] at h1 h2 ⊢
    rcases h1 with h1 | ⟨h1, h1'⟩ <;> rcases h2 with h2 | ⟨h2, h2'⟩
    · exact .inl (Nat.lt_trans h1 h2)
    · exact .inl (h2 ▸ h1)
    · exact .inl (h1 ▸ h2)
    · refine .inr ⟨h1.trans h2, ?_⟩
      have := Std.TransOrd.isLE_trans (α := String) (Ordering.isLE_iff_ne_gt.2 h1')
        (Ordering.isLE_iff_ne_gt.2 h2')
      exact Ordering.isLE_iff_ne_gt.1 this
  case total =>
    intro a b
    simp only [Bool.or_eq_true, decide_eq_true_eq, Bool.and_eq_true, beq_iff_eq, bne_iff_ne,
      ne_eq]
    rcases Nat.lt_trichotomy a.1 b.1 with h | h | h
    · exact .inl (.inl h)
    · by_cases hc : compare (writeTerm a.2) (writeTerm b.2) = .gt
      · refine .inr (.inr ⟨h.symm, ?_⟩)
        rw [Std.OrientedCmp.gt_iff_lt] at hc
        rw [hc]; decide
      · exact .inl (.inr ⟨h, hc⟩)
    · exact .inr (.inl h)
  case sorted =>
    rw [List.pairwise_map, List.pairwise_iff_getElem]
    intro i j hi hj hij
    simp [hQ i hi, hQ j hj, hij]
  case anti =>
    intro a b ha hb h1 h2
    simp only [List.mem_map] at ha hb
    obtain ⟨x, hx, rfl⟩ := ha
    obtain ⟨y, hy, rfl⟩ := hb
    simp only [Bool.or_eq_true, decide_eq_true_eq, Bool.and_eq_true, beq_iff_eq] at h1 h2
    have he : M.idxOf x = M.idxOf y := by omega
    have hx' := List.getElem_idxOf (List.idxOf_lt_length_of_mem hx)
    have hy' := List.getElem_idxOf (List.idxOf_lt_length_of_mem hy)
    have : x = y := by
      rw [← hx', ← hy']; simp only [he]
    subst this; rfl
  have hfst : (M.map (fun n => (M.idxOf n, n))).map (·.1) = List.range M.length := by
    apply List.ext_getElem (by simp)
    intro i h1 h2
    simp [hQ i (by simpa using h1)]
  simp only [hfst]
  rw [forIn_ok _ _ (fun x hx => by
    obtain ⟨i, hi⟩ := x
    have : i = hi := by
      rw [List.mem_zipIdx_iff_getElem?] at hx
      obtain ⟨h, hx⟩ := List.getElem?_eq_some_iff.1 hx
      simp at hx; omega
    subst this
    simp [pure, Except.pure])]
  simp [List.map_map, Function.comp_def, pure, Except.pure]

/-! ## Fragments -/

/-- No colon. -/
def NC (l : List Char) : Prop := ':' ∉ l
/-- Empty, or starting with a colon. -/
def CS (r : List Char) : Prop := r = [] ∨ r.head? = some ':'

/-- `g` is the fragment `f` or a fragment below it. -/
def U (f g : String) : Prop := ∃ r, g.toList = f.toList ++ r ∧ CS r

/-- `c` is a child fragment of `f`: `f`, a colon and a colon-free segment. -/
def Ch (f c : String) : Prop := ∃ a, c.toList = f.toList ++ ':' :: a ∧ NC a

@[simp] theorem cs_nil : CS [] := .inl rfl
@[simp] theorem cs_cons (c : Char) (l : List Char) : CS (c :: l) ↔ c = ':' := by
  simp [CS]

theorem nc_split : ∀ {a b x y : List Char}, NC a → NC b → CS x → CS y → a ++ x = b ++ y → a = b
  | [], [], _, _, _, _, _, _, _ => rfl
  | [], b :: bs, x, y, _, hb, hx, _, h => by
    rcases hx with rfl | hx
    · simp at h
    · cases x with
      | nil => simp at hx
      | cons c x =>
        simp at hx h; subst hx
        exact absurd (h.1 ▸ List.mem_cons_self) hb
  | a :: as, [], x, y, ha, _, _, hy, h => by
    rcases hy with rfl | hy
    · simp at h
    · cases y with
      | nil => simp at hy
      | cons c y =>
        simp at hy h; subst hy
        exact absurd (h.1 ▸ List.mem_cons_self) ha
  | a :: as, b :: bs, x, y, ha, hb, hx, hy, h => by
    simp only [List.cons_append, List.cons.injEq] at h
    obtain ⟨rfl, h⟩ := h
    have := nc_split (fun hm => ha (List.mem_cons_of_mem _ hm))
      (fun hm => hb (List.mem_cons_of_mem _ hm)) hx hy h
    rw [this]

theorem U_refl (f : String) : U f f := ⟨[], by simp, .inl rfl⟩

theorem U_up {f c g : String} (hc : Ch f c) (h : U c g) : U f g := by
  obtain ⟨a, ha, _⟩ := hc
  obtain ⟨r, hr, _⟩ := h
  exact ⟨':' :: a ++ r, by rw [hr, ha]; simp, by simp⟩

theorem U_trans {a b c : String} (h1 : U a b) (h2 : U b c) : U a c := by
  obtain ⟨r1, e1, c1⟩ := h1
  obtain ⟨r2, e2, c2⟩ := h2
  refine ⟨r1 ++ r2, by rw [e2, e1]; simp, ?_⟩
  rcases c1 with rfl | c1
  · simpa using c2
  · cases r1 with
    | nil => simp at c1
    | cons x r1 => simp at c1; simp [c1]

theorem U_len {f g : String} (h : U f g) : f.toList.length ≤ g.toList.length := by
  obtain ⟨r, e, _⟩ := h
  rw [e]; simp

theorem Ch_len {f c : String} (h : Ch f c) : f.toList.length < c.toList.length := by
  obtain ⟨a, e, _⟩ := h
  rw [e]; simp

theorem not_U_of_Ch {f c : String} (hc : Ch f c) : ¬ U c f := fun h =>
  absurd (U_len h) (Nat.not_le.2 (Ch_len hc))

/-- Siblings have disjoint regions. -/
theorem U_sib {f c₁ c₂ g : String} (h₁ : Ch f c₁) (h₂ : Ch f c₂) (u₁ : U c₁ g) (u₂ : U c₂ g) :
    c₁ = c₂ := by
  obtain ⟨a, ea, na⟩ := h₁
  obtain ⟨b, eb, nb⟩ := h₂
  obtain ⟨r, er, cr⟩ := u₁
  obtain ⟨s, es, cs⟩ := u₂
  rw [er, ea] at es
  rw [eb] at es
  simp only [List.append_assoc, List.cons_append, List.append_cancel_left_eq,
    List.cons.injEq, true_and] at es
  have := nc_split na nb cr cs es
  apply String.toList_inj.1
  rw [ea, eb, this]

/-- Colon-free roots have disjoint regions. -/
theorem U_root {a b g : String} (ha : NC a.toList) (hb : NC b.toList) (u₁ : U a g) (u₂ : U b g) :
    a = b := by
  obtain ⟨r, er, cr⟩ := u₁
  obtain ⟨s, es, cs⟩ := u₂
  rw [er] at es
  exact String.toList_inj.1 (nc_split ha hb cr cs es)

/-! ## Digits -/

theorem colon_not_mem_digits (n : Nat) : ':' ∉ Nat.toDigits 10 n := fun h =>
  absurd (Nat.isDigit_of_mem_toDigits (by decide) (by decide) h) (by decide)

@[simp] theorem toList_toString (n : Nat) : (toString n).toList = Nat.toDigits 10 n := by
  simp [toString, Nat.repr]

@[simp] theorem nc_digits (n : Nat) : NC (Nat.toDigits 10 n) := colon_not_mem_digits n

@[simp] theorem nc_cons (c : Char) (l : List Char) : NC (c :: l) ↔ c ≠ ':' ∧ NC l := by
  simp [NC, eq_comm]

@[simp] theorem nc_nil : NC [] := by simp [NC]

@[simp] theorem nc_append (a b : List Char) : NC (a ++ b) ↔ NC a ∧ NC b := by
  simp [NC]

@[simp] theorem digits_ne_nil (n : Nat) : Nat.toDigits 10 n ≠ [] := by
  rw [Nat.toDigits_eq_ite (by decide)]
  split <;> simp

theorem digits_inj {m n : Nat} : Nat.toDigits 10 m = Nat.toDigits 10 n ↔ m = n := by
  constructor
  · intro h
    apply Nat.repr_injective
    simp [Nat.repr, h]
  · rintro rfl; rfl

@[simp] theorem digits_eq_cons_letter (n : Nat) (c : Char) (l : List Char) (hc : c.isDigit = false) :
    Nat.toDigits 10 n = c :: l ↔ False := by
  simp only [iff_false]
  intro h
  have := Nat.isDigit_of_mem_toDigits (b := 10) (n := n) (by decide) (by decide)
    (h ▸ List.mem_cons_self)
  rw [hc] at this; cases this

/-! ## Percent-encoding -/

def hexd : List Char := "0123456789ABCDEF".toList

/-- One byte of `percentEncode`. -/
def enc (b : UInt8) : List Char :=
  let b : Nat := b.toNat
  let c := Char.ofNat b
  if (b < 128 && (c.isAlphanum || c == '-' || c == '.' || c == '_' || c == '~')) then [c]
  else ['%', hexd[b / 16]!, hexd[b % 16]!]

theorem toList_percentEncode (s : String) : (percentEncode s).toList = s.toUTF8.toList.flatMap enc := by
  unfold percentEncode; rw [String.toList_ofList]; rfl

def hexVal (c : Char) : Nat := hexd.idxOf c

/-- The byte a code starts with. -/
def val : List Char → Nat
  | ['%', h, l] => hexVal h * 16 + hexVal l
  | [c] => c.toNat
  | _ => 0

def Good (n : Nat) : Bool :=
  let e := enc (UInt8.ofNat n)
  ((e.length == 1 && e.head? != some '%') || (e.length == 3 && e.head? == some '%')) &&
    val e == n && !e.contains ':'

set_option maxRecDepth 100000 in
theorem good_all : ∀ n, n < 256 → Good n = true := by decide

theorem good (b : UInt8) : Good b.toNat = true := by
  have := good_all b.toNat b.toNat_lt
  exact this

theorem enc_shape (b : UInt8) :
    ((enc b).length = 1 ∧ (enc b).head? ≠ some '%' ∨ (enc b).length = 3 ∧ (enc b).head? = some '%') ∧
      val (enc b) = b.toNat ∧ ':' ∉ enc b := by
  have := good b
  simp only [Good, UInt8.ofNat_toNat] at this
  simp at this
  refine ⟨?_, this.1.2, this.2⟩
  rcases this.1.1 with h | h
  · exact .inl h
  · exact .inr h

theorem enc_prefix {a b : UInt8} {x y : List Char} (h : enc a ++ x = enc b ++ y) :
    a = b ∧ x = y := by
  obtain ⟨sa, va, _⟩ := enc_shape a
  obtain ⟨sb, vb, _⟩ := enc_shape b
  have hab : enc a = enc b := by
    have hh : (enc a).head? = (enc b).head? := by
      have := congrArg List.head? h
      rcases sa with ⟨la, _⟩ | ⟨la, _⟩ <;> rcases sb with ⟨lb, _⟩ | ⟨lb, _⟩ <;>
      · rw [List.head?_append, List.head?_append] at this
        cases ea : enc a <;> cases eb : enc b <;> simp_all
    have hl : (enc a).length = (enc b).length := by
      rcases sa with ⟨la, ha⟩ | ⟨la, ha⟩ <;> rcases sb with ⟨lb, hb⟩ | ⟨lb, hb⟩
      · rw [la, lb]
      · exact absurd (hh ▸ hb) ha
      · exact absurd (hh ▸ ha) hb
      · rw [la, lb]
    have := congrArg (List.take (enc a).length) h
    rw [List.take_left, hl, List.take_left] at this
    exact this
  have hv : a.toNat = b.toNat := by rw [← va, ← vb, hab]
  exact ⟨UInt8.toNat_inj.1 hv, by rw [hab] at h; exact List.append_cancel_left h⟩

theorem flatMap_enc_inj : ∀ {l₁ l₂ : List UInt8}, l₁.flatMap enc = l₂.flatMap enc → l₁ = l₂
  | [], [], _ => rfl
  | [], b :: l, h => by
    have h1 : (enc b).length ≥ 1 := by
      rcases (enc_shape b).1 with ⟨h, _⟩ | ⟨h, _⟩ <;> omega
    have h2 := congrArg List.length h
    simp only [List.flatMap_nil, List.length_nil, List.flatMap_cons, List.length_append] at h2
    omega
  | a :: l, [], h => by
    have h1 : (enc a).length ≥ 1 := by
      rcases (enc_shape a).1 with ⟨h, _⟩ | ⟨h, _⟩ <;> omega
    have h2 := congrArg List.length h
    simp only [List.flatMap_nil, List.length_nil, List.flatMap_cons, List.length_append] at h2
    omega
  | a :: l₁, b :: l₂, h => by
    simp only [List.flatMap_cons] at h
    obtain ⟨rfl, h'⟩ := enc_prefix h
    rw [flatMap_enc_inj h']

theorem byteArray_toList (b : ByteArray) : b.toList = b.data.toList := by
  have hs : b.size = b.data.toList.length := by cases b; rfl
  suffices ∀ i r, ByteArray.toList.loop b i r = r.reverse ++ b.data.toList.drop i by
    unfold ByteArray.toList; simpa using this 0 []
  intro i r
  induction h : b.size - i generalizing i r with
  | zero =>
    unfold ByteArray.toList.loop
    have : ¬ i < b.size := by omega
    simp only [this, ↓reduceIte]
    rw [List.drop_eq_nil_of_le (by omega)]; simp
  | succ n ih =>
    unfold ByteArray.toList.loop
    have hi : i < b.size := by omega
    simp only [hi, ↓reduceIte]
    rw [ih (i + 1) _ (by omega)]
    have hi' : i < b.data.toList.length := by omega
    rw [List.drop_eq_getElem_cons hi']
    have : b.get! i = b.data.toList[i] := by
      cases b with
      | mk d =>
        simp only [ByteArray.get!, Array.getElem_toList]
        exact getElem!_pos d i (by simpa using hi')
    rw [this]; simp

theorem toUTF8_toList_inj {s t : String} (h : s.toUTF8.toList = t.toUTF8.toList) : s = t := by
  rw [byteArray_toList, byteArray_toList] at h
  apply String.toByteArray_inj.1
  apply ByteArray.ext
  exact Array.toList_inj.1 h

/-- `percentEncode` is injective, so distinct state identifiers get distinct node IRIs. -/
-- @tg: implements:: [[openspec:formal-model#RDF Round Trip Theorem]] {note: "lemma: distinct identifiers get distinct node IRIs"}
theorem percentEncode_inj {s t : String} (h : percentEncode s = percentEncode t) : s = t := by
  have := congrArg String.toList h
  rw [toList_percentEncode, toList_percentEncode] at this
  exact toUTF8_toList_inj (flatMap_enc_inj this)

/-- `percentEncode` never yields a colon. -/
theorem colon_not_mem_percentEncode (s : String) : ':' ∉ (percentEncode s).toList := by
  rw [toList_percentEncode, List.mem_flatMap]
  rintro ⟨b, _, hb⟩
  exact (enc_shape b).2.2 hb

end RT
end Vertep.Rdf
