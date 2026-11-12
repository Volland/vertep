import VertepLean.Rdf.Equiv.ShapesTerm

/-!
# SHACL Core semantics of the chart shapes' building blocks

Rewrite rules that turn `Shacl.Conforms` of the shapes of `ShapesTerm` into plain statements
about the data graph: value nodes of the paths the shapes use, the counting components with the
counts they use (0 and 1), qualified counts, logical components, and `sh:class` on a graph
without `rdfs:subClassOf` triples.
-/

namespace Vertep.Rdf.ShapesL

open LeanRDF LeanRDF.Shacl

variable (G : Graph)

/-! ## Paths -/

@[simp] theorem vn_none (x y : Term) : VN G none x y ↔ y = x := Iff.rfl
@[simp] theorem vn_some (p : Path) (x y : Term) : VN G (some p) x y ↔ ValueNode G p x y := Iff.rfl

@[simp] theorem valueNode_pred (p : IRI) (x y : Term) :
    ValueNode G (.pred p) x y ↔ (⟨x, .iri p, y⟩ : Triple) ∈ G :=
  ⟨fun h => by cases h; assumption, .pred⟩

@[simp] theorem valueNode_inv (p : Path) (x y : Term) :
    ValueNode G (.inv p) x y ↔ ValueNode G p y x :=
  ⟨fun h => by cases h; assumption, .inv⟩

@[simp] theorem valueNode_seq_nil (x y : Term) : ValueNode G (.seq []) x y ↔ y = x :=
  ⟨fun h => by cases h; rfl, fun h => h ▸ .seqNil⟩

@[simp] theorem valueNode_seq_cons (p : Path) (ps : List Path) (x y : Term) :
    ValueNode G (.seq (p :: ps)) x y ↔ ∃ z, ValueNode G p x z ∧ ValueNode G (.seq ps) z y :=
  ⟨fun h => by cases h with | seqCons h1 h2 => exact ⟨_, h1, h2⟩,
   fun ⟨_, h1, h2⟩ => .seqCons h1 h2⟩

@[simp] theorem valueNode_alt (ps : List Path) (x y : Term) :
    ValueNode G (.alt ps) x y ↔ ∃ p ∈ ps, ValueNode G p x y :=
  ⟨fun h => by cases h with | alt hm h => exact ⟨_, hm, h⟩, fun ⟨_, hm, h⟩ => .alt hm h⟩

/-- `sh:oneOrMorePath` as the transitive closure of the step relation. -/
inductive Plus (R : Term → Term → Prop) : Term → Term → Prop
  | one {x y : Term} : R x y → Plus R x y
  | tail {x y z : Term} : Plus R x y → R y z → Plus R x z

theorem star_iff_plus {p : Path} {x y : Term} :
    ValueNode G (.zeroOrMore p) x y ↔ y = x ∨ Plus (ValueNode G p) x y := by
  constructor
  · intro h
    generalize hq : Path.zeroOrMore p = q at h
    induction h with
    | starRefl => exact .inl rfl
    | starTail _ h2 ih =>
      cases hq
      rcases ih rfl with rfl | ih
      · exact .inr (.one h2)
      · exact .inr (.tail ih h2)
    | _ => cases hq
  · rintro (rfl | h)
    · exact .starRefl
    · induction h with
      | one h => exact .starTail .starRefl h
      | tail _ h2 ih => exact .starTail ih h2

@[simp] theorem valueNode_oneOrMore (p : Path) (x y : Term) :
    ValueNode G (.oneOrMore p) x y ↔ Plus (ValueNode G p) x y := by
  constructor
  · intro h
    cases h with
    | plus h1 h2 =>
      rcases (star_iff_plus G).1 h1 with rfl | h1
      · exact .one h2
      · exact .tail h1 h2
  · intro h
    cases h with
    | one h => exact .plus .starRefl h
    | tail h1 h2 => exact .plus ((star_iff_plus G).2 (.inr h1)) h2

theorem Plus.mono {R S : Term → Term → Prop} (hRS : ∀ x y, R x y → S x y) {x y : Term}
    (h : Plus R x y) : Plus S x y := by
  induction h with
  | one h => exact .one (hRS _ _ h)
  | tail _ h2 ih => exact .tail ih (hRS _ _ h2)

theorem Plus.congr {R S : Term → Term → Prop} (hRS : ∀ x y, R x y ↔ S x y) {x y : Term} :
    Plus R x y ↔ Plus S x y :=
  ⟨Plus.mono (fun x y => (hRS x y).1), Plus.mono (fun x y => (hRS x y).2)⟩

theorem Plus.head {R : Term → Term → Prop} {x y z : Term} (h1 : R x y) (h2 : Plus R y z) :
    Plus R x z := by
  induction h2 with
  | one h => exact .tail (.one h1) h
  | tail _ h ih => exact .tail ih h

theorem Plus.trans {R : Term → Term → Prop} {x y z : Term} (h1 : Plus R x y) (h2 : Plus R y z) :
    Plus R x z := by
  induction h2 with
  | one h => exact .tail h1 h
  | tail _ h ih => exact .tail ih h

theorem Plus.cases_head {R : Term → Term → Prop} {x z : Term} (h : Plus R x z) :
    R x z ∨ ∃ y, R x y ∧ Plus R y z := by
  induction h with
  | one h => exact .inl h
  | tail h1 h2 ih =>
    rcases ih with h | ⟨y, hy, hp⟩
    · exact .inr ⟨_, h, .one h2⟩
    · exact .inr ⟨y, hy, .tail hp h2⟩

/-! ## Shapes and component lists -/

@[simp] theorem conforms_mk (path : Option Path) (targets : List Target) (cs : List Component)
    (x : Term) : Conforms G (mk path targets cs) x ↔ CompsHold G path cs x := by
  simp [mk, Conforms]

@[simp] theorem conforms_nodeS (cs : List Component) (x : Term) :
    Conforms G (nodeS cs) x ↔ CompsHold G none cs x := conforms_mk G _ _ cs x

@[simp] theorem conforms_propS (p : Path) (cs : List Component) (x : Term) :
    Conforms G (propS p cs) x ↔ CompsHold G (some p) cs x := conforms_mk G _ _ cs x

@[simp] theorem conforms_top (ts : List Target) (cs : List Component) (x : Term) :
    Conforms G (top ts cs) x ↔ CompsHold G none cs x := conforms_mk G _ _ cs x

@[simp] theorem compsHold_nil (path : Option Path) (x : Term) : CompsHold G path [] x ↔ True := by
  simp [CompsHold]

@[simp] theorem compsHold_cons (path : Option Path) (c : Component) (cs : List Component) (x : Term) :
    CompsHold G path (c :: cs) x ↔ CompHolds G path c x ∧ CompsHold G path cs x := by
  simp [CompsHold]

@[simp] theorem allConform_nil (x : Term) : AllConform G [] x ↔ True := by simp [AllConform]
@[simp] theorem allConform_cons (s : Shape) (ss : List Shape) (x : Term) :
    AllConform G (s :: ss) x ↔ Conforms G s x ∧ AllConform G ss x := by simp [AllConform]
@[simp] theorem someConforms_nil (x : Term) : SomeConforms G [] x ↔ False := by simp [SomeConforms]
@[simp] theorem someConforms_cons (s : Shape) (ss : List Shape) (x : Term) :
    SomeConforms G (s :: ss) x ↔ Conforms G s x ∨ SomeConforms G ss x := by simp [SomeConforms]
@[simp] theorem noneConform_nil (x : Term) : NoneConform G [] x ↔ True := by simp [NoneConform]
@[simp] theorem noneConform_cons (s : Shape) (ss : List Shape) (x : Term) :
    NoneConform G (s :: ss) x ↔ ¬ Conforms G s x ∧ NoneConform G ss x := by simp [NoneConform]
@[simp] theorem oneConforms_nil (x : Term) : OneConforms G [] x ↔ False := by simp [OneConforms]
@[simp] theorem oneConforms_cons (s : Shape) (ss : List Shape) (x : Term) :
    OneConforms G (s :: ss) x ↔
      (Conforms G s x ∧ NoneConform G ss x) ∨ (¬ Conforms G s x ∧ OneConforms G ss x) := by
  simp [OneConforms]

/-! ## Components -/

variable (path : Option Path) (x : Term)

@[simp] theorem holds_test (t : TermTest) :
    CompHolds G path (.test t) x ↔ ∀ v, VN G path x v → t.ok v = true := Iff.rfl
@[simp] theorem holds_class (c : Term) :
    CompHolds G path (.class_ c) x ↔ ∀ v, VN G path x v → InstanceOf G v c := Iff.rfl
@[simp] theorem holds_equals (p : IRI) :
    CompHolds G path (.equals p) x ↔ ∀ v, VN G path x v ↔ (⟨x, .iri p, v⟩ : Triple) ∈ G := Iff.rfl
@[simp] theorem holds_disjoint (p : IRI) :
    CompHolds G path (.disjoint p) x ↔ ∀ v, VN G path x v → (⟨x, .iri p, v⟩ : Triple) ∉ G := Iff.rfl
@[simp] theorem holds_lessThanOrEquals (p : IRI) :
    CompHolds G path (.lessThanOrEquals p) x ↔
      ∀ v w, VN G path x v → (⟨x, .iri p, w⟩ : Triple) ∈ G → lessB true v w = true := Iff.rfl
@[simp] theorem holds_not (s : Shape) :
    CompHolds G path (.not s) x ↔ ∀ v, VN G path x v → ¬ Conforms G s v := Iff.rfl
@[simp] theorem holds_and (ss : List Shape) :
    CompHolds G path (.and ss) x ↔ ∀ v, VN G path x v → AllConform G ss v := Iff.rfl
@[simp] theorem holds_or (ss : List Shape) :
    CompHolds G path (.or ss) x ↔ ∀ v, VN G path x v → SomeConforms G ss v := Iff.rfl
@[simp] theorem holds_xone (ss : List Shape) :
    CompHolds G path (.xone ss) x ↔ ∀ v, VN G path x v → OneConforms G ss v := Iff.rfl
@[simp] theorem holds_node (s : Shape) :
    CompHolds G path (.node s) x ↔ ∀ v, VN G path x v → Conforms G s v := Iff.rfl
@[simp] theorem holds_property (s : Shape) :
    CompHolds G path (.property s) x ↔ ∀ v, VN G path x v → Conforms G s v := Iff.rfl

@[simp] theorem holds_minCount1 :
    CompHolds G path (.minCount 1) x ↔ ∃ y, VN G path x y := by
  simp only [CompHolds]
  constructor
  · rintro ⟨l, _, hl, hn⟩
    cases l with
    | nil => simp at hn
    | cons y l => exact ⟨y, hl y List.mem_cons_self⟩
  · rintro ⟨y, hy⟩
    exact ⟨[y], List.nodup_cons.2 ⟨List.not_mem_nil, List.nodup_nil⟩,
      fun z hz => by simp at hz; subst hz; exact hy, by simp⟩

@[simp] theorem holds_maxCount1 :
    CompHolds G path (.maxCount 1) x ↔ ∀ y z, VN G path x y → VN G path x z → y = z := by
  simp only [CompHolds]
  constructor
  · intro h y z hy hz
    by_cases hne : y = z
    · exact hne
    · have := h [y, z] (by simp [hne]) (by
        intro w hw; simp at hw; rcases hw with rfl | rfl <;> assumption)
      simp at this
  · intro h l hl hm
    match l, hl, hm with
    | [], _, _ => simp
    | [_], _, _ => simp
    | a :: b :: _, hl, hm =>
      have := h a b (hm a (by simp)) (hm b (by simp))
      subst this; simp at hl

@[simp] theorem holds_maxCount0 :
    CompHolds G path (.maxCount 0) x ↔ ∀ y, ¬ VN G path x y := by
  simp only [CompHolds]
  constructor
  · intro h y hy
    have := h [y] (by simp) (by intro w hw; simp at hw; subst hw; exact hy)
    simp at this
  · intro h l _ hm
    match l, hm with
    | [], _ => simp
    | a :: _, hm => exact absurd (hm a (by simp)) (h a)

@[simp] theorem holds_qualifiedMin1 (s : Shape) :
    CompHolds G path (.qualifiedMin s 1 []) x ↔ ∃ y, VN G path x y ∧ Conforms G s y := by
  simp only [CompHolds, noneConform_nil, and_true]
  constructor
  · rintro ⟨l, _, hl, hn⟩
    cases l with
    | nil => simp at hn
    | cons y l => exact ⟨y, hl y List.mem_cons_self⟩
  · rintro ⟨y, hy⟩
    exact ⟨[y], List.nodup_cons.2 ⟨List.not_mem_nil, List.nodup_nil⟩,
      fun z hz => by simp at hz; subst hz; exact hy, by simp⟩

@[simp] theorem holds_qualifiedMax0 (s : Shape) :
    CompHolds G path (.qualifiedMax s 0 []) x ↔ ∀ y, VN G path x y → ¬ Conforms G s y := by
  simp only [CompHolds, noneConform_nil, and_true]
  constructor
  · intro h y hy hs
    have := h [y] (by simp) (by intro w hw; simp at hw; subst hw; exact ⟨hy, hs⟩)
    simp at this
  · intro h l _ hm
    match l, hm with
    | [], _ => simp
    | a :: _, hm => exact absurd (hm a (by simp)).2 (h a (hm a (by simp)).1)

/-! ## Classes -/

/-- The graph has no `rdfs:subClassOf` triples. -/
def NoSub : Prop := ∀ s o, (⟨s, .iri Vocab.rdfs.subClassOf, o⟩ : Triple) ∉ G

theorem instanceOf_iff (h : NoSub G) (v c : Term) :
    InstanceOf G v c ↔ (⟨v, .iri Vocab.rdf.type, c⟩ : Triple) ∈ G := by
  constructor
  · rintro ⟨t, ht, hc⟩
    rcases (star_iff_plus G).1 hc with rfl | hp
    · exact ht
    · rcases hp.cases_head with h' | ⟨_, h', _⟩ <;>
        exact absurd ((valueNode_pred G _ _ _).1 h') (h _ _)
  · intro h
    exact ⟨c, h, .starRefl⟩

end Vertep.Rdf.ShapesL
