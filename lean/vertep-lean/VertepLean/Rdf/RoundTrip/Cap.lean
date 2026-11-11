import VertepLean.Rdf.RoundTrip.Basic

/-!
# RDF round trip: capture of a node's triples

`Cap ts R E`: the list `E` (the emission of a node) is a sublist of the graph's triples `ts` and
holds every relevant triple of `ts` whose subject is a node IRI with a fragment in the region `R`.
`Cap.kid` passes capture from a node to its children, given the decomposition of its emission
into local triples and children with pairwise distinct child fragments.
-/

namespace Vertep.Rdf
open LeanRDF

namespace RT

variable (k : Ctx)

@[simp] theorem iri_inj {f g : String} : k.iri f = k.iri g ↔ f = g := by
  simp [Ctx.iri]

@[simp] theorem vt_inj {p q : String} : vt p = vt q ↔ p = q := by
  simp [vt]

@[simp] theorem vt_ne_rdfType (p : String) : vt p ≠ rdfType := by
  simp [vt, rdfType, vtNs, ← String.toList_inj, String.toList_append]

@[simp] theorem rdfType_ne_vt (p : String) : rdfType ≠ vt p := fun h => vt_ne_rdfType p h.symm

@[simp] theorem U_refl' (f : String) : U f f := U_refl f

/-- The triples the decoder may read: all but the derived `vt:historyDefault` and `vt:choice`
triples, whose subjects are not tied to the node emitting them. -/
def Rel (t : Triple) : Prop := t.p ≠ vt "historyDefault" ∧ t.p ≠ vt "choice"

@[simp] theorem rel_mk (s o : Term) (p : String) :
    Rel ⟨s, vt p, o⟩ ↔ p ≠ "historyDefault" ∧ p ≠ "choice" := by
  simp [Rel]

@[simp] theorem rel_type (s o : Term) : Rel ⟨s, rdfType, o⟩ := by
  simp [Rel]

/-- Every relevant triple is the link from `owner` or has a subject at or below `f`. -/
def LocL (owner : Term) (prop f : String) (E : List Triple) : Prop :=
  ∀ t ∈ E, Rel t → t = ⟨owner, vt prop, k.iri f⟩ ∨ ∃ g, t.s = k.iri g ∧ U f g

/-- Every relevant triple has subject `owner` or a subject at or below `f`. -/
def LocU (owner : Term) (f : String) (E : List Triple) : Prop :=
  ∀ t ∈ E, Rel t → t.s = owner ∨ ∃ g, t.s = k.iri g ∧ U f g

theorem LocL.locU {owner : Term} {prop f : String} {E : List Triple} (h : LocL k owner prop f E) :
    LocU k owner f E := fun t ht hr => by
  rcases h t ht hr with rfl | h
  · exact .inl rfl
  · exact .inr h

/-- `E` (a node's emission) is a sublist of `ts` and holds every relevant triple of `ts` whose
subject has a fragment in `R`. -/
structure Cap (ts : List Triple) (R : String → Prop) (E : List Triple) : Prop where
  sub : E.Sublist ts
  cap : ∀ t ∈ ts, Rel t → ∀ g, t.s = k.iri g → R g → t ∈ E

variable {k}

theorem Cap.lookup {ts E : List Triple} {R : String → Prop} (h : Cap k ts R E) {f : String}
    (hf : R f) {p o : Term} (hr : Rel ⟨k.iri f, p, o⟩) :
    ⟨k.iri f, p, o⟩ ∈ ts ↔ ⟨k.iri f, p, o⟩ ∈ E :=
  ⟨fun ht => h.cap _ ht hr f rfl hf, fun ht => h.sub.subset ht⟩

theorem Cap.mono {ts E : List Triple} {R R' : String → Prop} (h : Cap k ts R E)
    (hR : ∀ g, R' g → R g) : Cap k ts R' E :=
  ⟨h.sub, fun t ht hr g hg hR' => h.cap t ht hr g hg (hR g hR')⟩

/-- The owner of a node lies strictly above it. -/
def Above (owner : Term) (f : String) : Prop := ∀ g, U f g → owner ≠ k.iri g

theorem above_of_ch {o f : String} (h : Ch o f) : Above (k := k) (k.iri o) f := by
  intro g hg he
  rw [iri_inj] at he; subst he
  exact absurd (U_len hg) (Nat.not_le.2 (Ch_len h))

/-- Capture passes from a node at `f` to each child of a decomposition of its emission. -/
theorem Cap.kid {ts E : List Triple} {f : String} (h : Cap k ts (U f) E) {owner : Term}
    (D : List (String × List Triple))
    (hcov : ∀ t ∈ E, Rel t → t.s = owner ∨ t.s = k.iri f ∨ ∃ d ∈ D, t ∈ d.2)
    (hch : ∀ d ∈ D, Ch f d.1)
    (hloc : ∀ d ∈ D, LocU k (k.iri f) d.1 d.2)
    (hnd : (D.map Prod.fst).Nodup)
    (hown : Above (k := k) owner f)
    {d : String × List Triple} (hd : d ∈ D) (hsub : d.2.Sublist E) : Cap k ts (U d.1) d.2 := by
  refine ⟨hsub.trans h.sub, fun t ht hr g hg hU => ?_⟩
  have hUf : U f g := U_up (hch d hd) hU
  have hE := h.cap t ht hr g hg hUf
  rcases hcov t hE hr with ho | hs | ⟨d', hd', ht'⟩
  · exact absurd (ho.symm.trans hg) (hown g hUf)
  · rw [hg, iri_inj] at hs; subst hs
    exact absurd hU (not_U_of_Ch (hch d hd))
  · rcases hloc d' hd' t ht' hr with hs | ⟨g', hg', hU'⟩
    · rw [hg, iri_inj] at hs; subst hs
      exact absurd hU (not_U_of_Ch (hch d hd))
    · rw [hg, iri_inj] at hg'; subst hg'
      have he : d'.1 = d.1 := U_sib (hch d' hd') (hch d hd) hU' hU
      have : d' = d := by
        rw [List.nodup_iff_pairwise_ne, List.pairwise_map] at hnd
        refine Classical.byContradiction fun hne => ?_
        obtain ⟨i, hi, rfl⟩ := List.getElem_of_mem hd
        obtain ⟨j, hj, rfl⟩ := List.getElem_of_mem hd'
        rcases Nat.lt_trichotomy i j with hij | rfl | hij
        · exact List.pairwise_iff_getElem.1 hnd i j hi hj hij he.symm
        · exact hne rfl
        · exact List.pairwise_iff_getElem.1 hnd j i hj hi hij he
      rw [← this]; exact ht'

end RT
end Vertep.Rdf
