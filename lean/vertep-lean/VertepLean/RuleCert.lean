/-!
# Rule certificates

The ground-instance checker behind `vt rules certify` (`rule-verification`, *Derivation
Certificate*). A rule decision's derivation is a clause of the rules profile, the answers its
`tm.triple` calls took (the *tape*), and a binding of the clause's variables. The checker accepts
the instance when every body atom holds under the binding against the tape, the head instantiates
to the recorded conclusion, and the support variables are bound to the recorded statement ids.

What this proves, and what it does not: the kernel checks that the recorded conclusion follows
from the tape under this clause (sufficiency of the stated support, `decisions#Support is checked
for necessity and certificates check sufficiency`). It does not check that the clause is the
program's clause (the Rust exporter parses the program text, so that parser is trusted), nor that
each tape fact is in the store (the eid check of the host).
-/

namespace Vertep.RuleCert

/-- A ground term: an IRI, a plain string or an integer. -/
inductive Term where
  | iri (s : String)
  | str (s : String)
  | int (n : Int)
  deriving DecidableEq, Repr

/-- A term or a variable of a clause. -/
inductive Pat where
  | var (name : String)
  | num (name : String)
  | term (t : Term)
  deriving DecidableEq, Repr

/-- One answer of a `tm.triple` call: a statement of the store with its id. -/
structure Fact where
  s : Term
  p : Term
  o : Term
  eid : String
  deriving DecidableEq, Repr

/-- The comparisons of the profile. -/
inductive Cmp where
  | le | lt | ge | gt | eq
  deriving DecidableEq, Repr

/-- A body atom: a `tm.triple` call (the last field is the variable bound to the statement id) or
an integer comparison. -/
inductive Atom where
  | triple (s p o : Pat) (e : String)
  | cmp (op : Cmp) (l r : Pat)
  deriving Repr

/-- A clause `tm.conclude (triple S P O) [support…] :- body`. -/
structure Clause where
  headS : Pat
  headP : Pat
  headO : Pat
  support : List String
  body : List Atom
  deriving Repr

/-- Values of the clause's variables. Statement ids are bound as `Term.str`. -/
abbrev Binding := List (String × Term)

def Binding.lookup (b : Binding) (v : String) : Option Term :=
  (b.find? (fun x => x.1 == v)).map (·.2)

def Pat.inst (b : Binding) : Pat → Option Term
  | .var v => b.lookup v
  | .num v => match b.lookup v with
    | some (.int n) => some (.int n)
    | _ => none
  | .term t => some t

def Cmp.holds : Cmp → Int → Int → Bool
  | .le, x, y => decide (x ≤ y)
  | .lt, x, y => decide (x < y)
  | .ge, x, y => decide (x ≥ y)
  | .gt, x, y => decide (x > y)
  | .eq, x, y => decide (x = y)

/-- An atom holds under a binding against the tape. -/
def atomOk (tape : List Fact) (b : Binding) : Atom → Bool
  | .triple s p o e =>
    match s.inst b, p.inst b, o.inst b, b.lookup e with
    | some s', some p', some o', some (.str id) =>
      tape.any (fun f => decide (f.s = s') && decide (f.p = p') && decide (f.o = o') && f.eid == id)
    | _, _, _, _ => false
  | .cmp op l r =>
    match l.inst b, r.inst b with
    | some (.int x), some (.int y) => op.holds x y
    | _, _ => false

/-- The statement ids the support variables stand for. -/
def supportIds (c : Clause) (b : Binding) : List String :=
  c.support.map (fun v => match b.lookup v with | some (.str id) => id | _ => "")

/-- The checker. -/
def check (tape : List Fact) (c : Clause) (b : Binding)
    (s p o : Term) (support : List String) : Bool :=
  c.body.all (atomOk tape b) &&
  decide (c.headS.inst b = some s) && decide (c.headP.inst b = some p) &&
  decide (c.headO.inst b = some o) && decide (supportIds c b = support)

/-- The statement the certificate establishes: the instantiated clause is a derivation. -/
def Derives (tape : List Fact) (c : Clause) (b : Binding)
    (s p o : Term) (support : List String) : Prop :=
  (∀ a ∈ c.body, atomOk tape b a = true) ∧
  c.headS.inst b = some s ∧ c.headP.inst b = some p ∧ c.headO.inst b = some o ∧
  supportIds c b = support

/-- **Soundness.** An accepted instance is a derivation. -/
theorem check_sound {tape : List Fact} {c : Clause} {b : Binding} {s p o : Term}
    {support : List String} (h : check tape c b s p o support = true) :
    Derives tape c b s p o support := by
  simp only [check, Bool.and_eq_true, List.all_eq_true, decide_eq_true_eq] at h
  obtain ⟨⟨⟨⟨hb, hs⟩, hp⟩, ho⟩, hsup⟩ := h
  exact ⟨hb, hs, hp, ho, hsup⟩

/-- **Completeness.** A derivation is accepted. -/
theorem check_complete {tape : List Fact} {c : Clause} {b : Binding} {s p o : Term}
    {support : List String} (h : Derives tape c b s p o support) :
    check tape c b s p o support = true := by
  obtain ⟨hb, hs, hp, ho, hsup⟩ := h
  simp only [check, Bool.and_eq_true, List.all_eq_true, decide_eq_true_eq]
  exact ⟨⟨⟨⟨hb, hs⟩, hp⟩, ho⟩, hsup⟩

/-- A statement id bound through a `tm.triple` atom that holds is on the tape: the stated support
is not invented. -/
theorem triple_in_tape {tape : List Fact} {b : Binding} {s p o : Pat} {e : String}
    (h : atomOk tape b (.triple s p o e) = true) :
    ∃ f ∈ tape, b.lookup e = some (Term.str f.eid) := by
  unfold atomOk at h
  rcases hs : s.inst b with _ | s' <;> rcases hp : p.inst b with _ | p' <;>
    rcases ho : o.inst b with _ | o' <;> rcases he : b.lookup e with _ | t <;>
    simp only [hs, hp, ho, he] at h <;> try contradiction
  cases t with
  | str id =>
    simp only [List.any_eq_true, Bool.and_eq_true, decide_eq_true_eq, beq_iff_eq] at h
    obtain ⟨f, hf, _, hid⟩ := h
    exact ⟨f, hf, by rw [hid]⟩
  | iri _ => simp at h
  | int _ => simp at h

end Vertep.RuleCert
