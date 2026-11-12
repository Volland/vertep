import VertepLean.Rdf.Equiv.Focus

/-!
# The literals of the projection under the shapes' term tests

`xsd:string`, `xsd:boolean` and `xsd:integer` literals of the projection pass their datatype
tests; `vt:order` values compare as the natural numbers they write; string lengths, node kinds
and `sh:in` on the projection's literals.
-/

namespace Vertep.Rdf.ShapesL

open LeanRDF LeanRDF.Shacl

attribute [simp] P I pr cls dt lit minC maxC none0 some1 notBoth optStrP strP optLitP boolP clsP
  nodeS propS top tcls

@[simp] theorem iri_vi (l : String) : Term.iri (vi l) = vt l := rfl

@[simp] theorem rdf_type_eq : Term.iri Vocab.rdf.type = rdfType := by decide

theorem digitChar_eq : ∀ d, d < 10 → Nat.digitChar d = Xsd.digitChar d := by decide

theorem toDigits_eq (n : Nat) : Nat.toDigits 10 n = Xsd.digitsOf n := by
  induction n using Nat.strongRecOn with
  | ind n ih =>
    rw [Nat.toDigits_eq_ite (by decide), Xsd.digitsOf]
    split
    · rename_i h; rw [digitChar_eq n h]
    · rename_i h
      rw [ih (n / 10) (Nat.div_lt_self (by omega) (by decide)),
        digitChar_eq (n % 10) (Nat.mod_lt _ (by decide))]

theorem toString_eq (n : Nat) : toString n = Xsd.canonicalInteger (n : Int) := by
  simp only [Xsd.canonicalInteger, Xsd.canonicalIntegerChars]
  have : ¬ ((n : Int) < 0) := by omega
  rw [ite_eq_right_iff.2 (fun h => absurd h this), Int.natAbs_natCast, ← toDigits_eq]
  rfl

theorem localName_integer : Xsd.localName? xsdInteger = some "integer" := by decide +kernel
theorem localName_string' : Xsd.localName? xsdString = some "string" := by decide +kernel
theorem localName_boolean' : Xsd.localName? xsdBoolean = some "boolean" := by decide +kernel

theorem value_intLit (n : Nat) : (Literal.typed (toString n) xsdInteger).value? = some (.integer n) := by
  simp only [Literal.value?, localName_integer]
  rw [toString_eq, Xsd.parseInteger_canonical]
  simp [Xsd.integerTypes]

@[simp] theorem ok_dt_str (s : String) : (TermTest.datatype xsdString).ok (strLit s) = true := by
  simp [TermTest.ok, strLit, Literal.value?, Literal.datatype, localName_string']

theorem value_boolLit (b : Bool) :
    (Literal.typed (if b then "true" else "false") xsdBoolean).value? = some (.boolean b) := by
  have := Xsd.parseBoolean_canonical b
  simp only [Xsd.canonicalBoolean] at this
  simp [Literal.value?, localName_boolean', this]

@[simp] theorem ok_dt_bool (b : Bool) : (TermTest.datatype xsdBoolean).ok (boolLit b) = true := by
  simp only [TermTest.ok, boolLit, Literal.datatype, value_boolLit]
  simp

@[simp] theorem ok_dt_int (n : Nat) : (TermTest.datatype xsdInteger).ok (intLit n) = true := by
  simp only [TermTest.ok, intLit, Literal.datatype, value_intLit]
  simp

theorem compare_int (a b : Nat) :
    termCompare? (intLit a) (intLit b) = some (if a < b then .lt else if a = b then .eq else .gt) := by
  simp only [termCompare?, termValue?, intLit, value_intLit, Option.bind_eq_bind, Option.bind_some]
  simp only [Xsd.XsdValue.compare?, Xsd.XsdValue.num?, Xsd.Num.compare?]
  congr 1
  unfold Xsd.DateTime.ratCompare
  by_cases h : a < b
  · have : ((a : Int) : Rat) < ((b : Int) : Rat) := by
      rw [Rat.intCast_lt_intCast]; omega
    simp [h, this]
  · have : ¬ (((a : Int) : Rat) < ((b : Int) : Rat)) := by
      rw [Rat.intCast_lt_intCast]; omega
    by_cases e : a = b
    · subst e; simp
    · have h2 : ((a : Int) : Rat) ≠ ((b : Int) : Rat) := by
        rw [Ne, Rat.intCast_inj]; omega
      simp [this, h2, h, e]

theorem xsdInt_eq : xsdInt = intLit 0 := rfl

@[simp] theorem ok_minInclusive (n : Nat) :
    (TermTest.range .minInclusive xsdInt).ok (intLit n) = true := by
  simp only [TermTest.ok, xsdInt_eq, compare_int]
  by_cases h : n < 0 <;> by_cases e : n = 0 <;> simp [h, e] <;> omega

@[simp] theorem ok_minExclusive (n : Nat) :
    (TermTest.range .minExclusive xsdInt).ok (intLit n) = decide (0 < n) := by
  simp only [TermTest.ok, xsdInt_eq, compare_int]
  by_cases e : n = 0
  · subst e; simp
  · have : ¬ n < 0 := by omega
    simp [this, e]; omega

@[simp] theorem lessB_int (a b : Nat) : lessB true (intLit a) (intLit b) = decide (a ≤ b) := by
  simp only [lessB, compare_int]
  by_cases h : a < b
  · simp [h]; omega
  · by_cases e : a = b
    · subst e; simp
    · simp [h, e]; omega

@[simp] theorem ok_lit (l : Literal) : (TermTest.nodeKind .literal).ok (.lit l) = true := rfl
@[simp] theorem ok_lit_iri (i : IRI) : (TermTest.nodeKind .literal).ok (.iri i) = false := rfl

@[simp] theorem ok_minLength1 (s : String) :
    (TermTest.minLength 1).ok (strLit s) = decide (s ≠ "") := by
  simp only [TermTest.ok, strLit, termStr?, Literal.lexical, Option.any_some]
  by_cases h : s = ""
  · subst h; simp
  · have : 1 ≤ s.length := by
      have := mt String.length_eq_zero_iff.1 h; omega
    simp [h, this]

@[simp] theorem ok_in (vs : List Term) (v : Term) : (TermTest.in_ vs).ok v = vs.contains v := rfl

@[simp] theorem strLit_inj {a b : String} : strLit a = strLit b ↔ a = b := by simp [strLit]
@[simp] theorem intLit_inj {a b : Nat} : intLit a = intLit b ↔ a = b := by
  simp only [intLit, Term.lit.injEq, Literal.typed.injEq, and_true]
  constructor
  · intro h
    have := congrArg Xsd.parseInteger h
    rw [toString_eq, toString_eq, Xsd.parseInteger_canonical, Xsd.parseInteger_canonical] at this
    simp only [Option.some.injEq] at this
    exact Int.ofNat_inj.1 this
  · rintro rfl; rfl
@[simp] theorem boolLit_inj {a b : Bool} : boolLit a = boolLit b ↔ a = b := by
  cases a <;> cases b <;> simp [boolLit]

end Vertep.Rdf.ShapesL
