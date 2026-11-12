import VertepLean.Tape

/-!
# The type language

Mirrors `crates/vertep-core/src/types/ty.rs` (`Primitive`, `Type`, `Field`, `Variant`), the
named type declarations of `decl.rs` and the lexical rules of the primitives in `prim.rs`:
signed 64-bit integers, decimals `-?[0-9]+(\.[0-9]+)?`, RFC 3339 datetimes with a required
offset and IRIs with a scheme.

Values are the model's `Value`s (`VertepLean/Tape.lean`), whose numbers are exact decimals:
`int` is decided exactly on the decimal (`mantissa × 10^-exponent` integral and in
`[-2^63, 2^63)`), where Rust decides it on the `f64` the JSON number reads as.
-/

namespace Vertep.Types

/-- The primitive types (Rust `Primitive`). -/
inductive Prim where
  | bool
  | int
  | decimal
  | string
  | datetime
  | iri
  deriving Repr, BEq, DecidableEq, Inhabited

/-- A type of the chart type language (Rust `Type`): records are lists of `(field, type)`
pairs in declaration order, unions lists of `(tag, payload)` pairs. -/
-- @tg: implements:: [[openspec:formal-model#Type Checker Correctness]]
inductive Ty where
  | prim (p : Prim)
  /-- The dynamic type `value`. -/
  | value
  | record (fields : List (String × Ty))
  | union (variants : List (String × Option Ty))
  | list (item : Ty)
  | optional (inner : Ty)
  | named (name : String)
  deriving Repr, Inhabited

/-- A block of named type declarations, in declaration order (Rust `Declarations::types`). -/
abbrev Decls := List (String × Ty)

/-- The definition of a named type: the first declaration of that name (Rust
`Declarations::lookup`). -/
def Decls.lookup (D : Decls) (n : String) : Option Ty := List.lookup n D

/-! ## Lexical rules of the primitives (Rust `prim.rs`) -/

namespace Lex

/-- `k` decimal digits at the front of `cs`, accumulated onto `acc`, and the rest. -/
def digitsAux : Nat → Nat → List Char → Option (Nat × List Char)
  | 0, acc, cs => some (acc, cs)
  | _ + 1, _, [] => none
  | k + 1, acc, c :: cs =>
    if c.isDigit then digitsAux k (acc * 10 + (c.toNat - '0'.toNat)) cs else none

/-- Exactly `k` decimal digits at the front of `cs`, as a number, and the rest. -/
def digits (k : Nat) (cs : List Char) : Option (Nat × List Char) := digitsAux k 0 cs

/-- The rest of `cs` after the character `c`. -/
def expect (c : Char) : List Char → Option (List Char)
  | d :: cs => if d == c then some cs else none
  | [] => none

def isLeap (y : Nat) : Bool := (y % 4 == 0 && y % 100 != 0) || y % 400 == 0

def daysInMonth (y m : Nat) : Nat :=
  if m == 1 || m == 3 || m == 5 || m == 7 || m == 8 || m == 10 || m == 12 then 31
  else if m == 4 || m == 6 || m == 9 || m == 11 then 30
  else if m == 2 && isLeap y then 29
  else 28

/-- The RFC 3339 offset and end of text: `Z`, `z` or `±hh:mm`, then nothing. -/
def offset : List Char → Bool
  | ['Z'] | ['z'] => true
  | s :: r =>
    (s == '+' || s == '-') &&
      match digits 2 r with
      | some (oh, r) =>
        match expect ':' r with
        | some r =>
          match digits 2 r with
          | some (om, r) => oh ≤ 23 && om ≤ 59 && r.isEmpty
          | none => false
        | none => false
      | none => false
  | [] => false

/-- Fractional seconds (`.` and at least one digit) if present, then the offset. -/
def fractionOffset : List Char → Bool
  | '.' :: r =>
    let ds := r.takeWhile Char.isDigit
    !ds.isEmpty && offset (r.drop ds.length)
  | cs => offset cs

/-- Rust `parse_datetime(s).is_ok()`: `YYYY-MM-DD(T|t)hh:mm:ss[.f+](Z|z|±hh:mm)` with month
1 to 12, the day within the month (leap years), hour to 23, minute to 59 and second to 60. -/
def datetime (cs : List Char) : Bool :=
  match digits 4 cs with
  | none => false
  | some (year, cs) =>
  match expect '-' cs with
  | none => false
  | some cs =>
  match digits 2 cs with
  | none => false
  | some (month, cs) =>
  match expect '-' cs with
  | none => false
  | some cs =>
  match digits 2 cs with
  | none => false
  | some (day, cs) =>
  1 ≤ month && month ≤ 12 && 1 ≤ day && day ≤ daysInMonth year month &&
  match cs with
  | t :: cs =>
    (t == 'T' || t == 't') &&
    match digits 2 cs with
    | none => false
    | some (hour, cs) =>
    match expect ':' cs with
    | none => false
    | some cs =>
    match digits 2 cs with
    | none => false
    | some (minute, cs) =>
    match expect ':' cs with
    | none => false
    | some cs =>
    match digits 2 cs with
    | none => false
    | some (second, cs) =>
    hour ≤ 23 && minute ≤ 59 && second ≤ 60 && fractionOffset cs
  | [] => false

def isDatetime (s : String) : Bool := datetime s.toList

def isAsciiAlpha (c : Char) : Bool := ('a' ≤ c && c ≤ 'z') || ('A' ≤ c && c ≤ 'Z')

def isHex (c : Char) : Bool := c.isDigit || ('a' ≤ c && c ≤ 'f') || ('A' ≤ c && c ≤ 'F')

/-- An IRI scheme: an ASCII letter, then letters, digits, `+`, `-` or `.`. -/
def scheme : List Char → Bool
  | c :: cs => isAsciiAlpha c && cs.all fun d => isAsciiAlpha d || d.isDigit || "+-.".contains d
  | [] => false

/-- The IRI after its scheme: no space, control character, `<>"{}|\^` or backquote, and every
`%` starts a percent-encoding. -/
def iriRest : List Char → Bool
  | [] => true
  | '%' :: a :: b :: r => isHex a && isHex b && iriRest r
  | '%' :: _ => false
  | c :: r =>
    !(c.toNat ≤ 0x20 || c.toNat == 0x7f || "<>\"{}|\\^`".contains c) && iriRest r

/-- Rust `check_iri(s).is_ok()`: an IRI with a scheme (RFC 3987). -/
def isIri (s : String) : Bool :=
  match s.toList.span (· != ':') with
  | (sch, ':' :: rest) => scheme sch && iriRest rest
  | _ => false

def allDigits (cs : List Char) : Bool := !cs.isEmpty && cs.all Char.isDigit

/-- Rust `is_decimal`: `-?[0-9]+(\.[0-9]+)?`. -/
def isDecimal (s : String) : Bool :=
  let cs := match s.toList with
    | '-' :: r => r
    | r => r
  match cs.span (· != '.') with
  | (int, []) => allDigits int
  | (int, _ :: frac) => allDigits int && allDigits frac

/-- The exact decimal `m × 10^-e` is an integer in the signed 64-bit range (Rust
`f64_to_i64(n).is_some()`). -/
def intOk (m : Int) (e : Nat) : Bool :=
  m % (10 ^ e : Int) == 0 && decide (-(2 ^ 63 : Int) * 10 ^ e ≤ m) &&
    decide (m < (2 ^ 63 : Int) * 10 ^ e)

end Lex

/-- Whether a value has a primitive type (Rust `Walker::primitive` without error). -/
def primOk : Prim → Value → Bool
  | .bool, .bool _ => true
  | .string, .str _ => true
  | .int, .num m e => Lex.intOk m e
  | .decimal, .num _ _ => true
  | .decimal, .str s => Lex.isDecimal s
  | .datetime, .str s => Lex.isDatetime s
  | .iri, .str s => Lex.isIri s
  | _, _ => false

/-- `undefined` and `null` are absent values (Rust `is_absent`). -/
def absent : Value → Bool
  | .undefined | .null => true
  | _ => false

/-- The value of field `k` of an object's entries: the first entry named `k`, unless it is
`undefined` (Rust: `find(...).filter(|v| !matches!(v, Value::Undefined))`). -/
def present (entries : List (String × Value)) (k : String) : Option Value :=
  match entries.lookup k with
  | some .undefined => none
  | r => r

/-! ## Name resolution -/

/-- The number of declarations whose names are not in `seen`: the termination measure of name
expansion (of types here, of `$ref`s in the JSON Schema validator). -/
def unexpanded {β : Type} (D : List (String × β)) (seen : List String) : Nat :=
  D.countP fun d => !seen.contains d.1

theorem lookup_mem' {β : Type} {l : List (String × β)} {n : String} {u : β}
    (h : l.lookup n = some u) : (n, u) ∈ l := by
  induction l with
  | nil => simp [List.lookup] at h
  | cons d l ih =>
    obtain ⟨k, t⟩ := d
    by_cases hk : n = k
    · subst hk; simp [List.lookup] at h; simp [h]
    · have : (n == k) = false := by simpa using hk
      simp [List.lookup, this] at h
      exact List.mem_cons_of_mem _ (ih h)

theorem lookup_mem {D : Decls} {n : String} {u : Ty} (h : D.lookup n = some u) :
    (n, u) ∈ D := lookup_mem' h

/-- Expanding a declared name not yet expanded at this position decreases `unexpanded`. -/
theorem unexpanded_lt {β : Type} {D : List (String × β)} {n : String} {seen : List String}
    {u : β} (hl : D.lookup n = some u) (hn : ¬seen.contains n = true) :
    unexpanded D (n :: seen) < unexpanded D seen := by
  unfold unexpanded
  generalize hp' : (fun d : String × β => !(n :: seen).contains d.1) = p'
  generalize hp : (fun d : String × β => !seen.contains d.1) = p
  have hle : ∀ x, p' x = true → p x = true := by
    intro x hx; subst hp hp'
    simp only [List.contains_cons, Bool.not_or, Bool.and_eq_true] at hx
    exact hx.2
  have hn' : p' (n, u) = false := by subst hp'; simp
  have hnp : p (n, u) = true := by subst hp; simpa using hn
  have heq : ∀ x : String × β, x.1 ≠ n → p' x = p x := by
    intro x hx; subst hp hp'
    simp only [List.contains_cons, Bool.not_or]
    have : (!(x.1 == n)) = true := by simpa using hx
    rw [this, Bool.true_and]
  clear hp hp'
  induction D with
  | nil => simp [List.lookup] at hl
  | cons d D ih =>
    obtain ⟨k, t⟩ := d
    rw [List.countP_cons, List.countP_cons]
    by_cases hk : n = k
    · subst hk
      have hu : t = u := by simpa [List.lookup] using hl
      subst hu
      have hmono : List.countP p' D ≤ List.countP p D :=
        List.countP_mono_left fun x _ hx => hle x hx
      simp only [hn', hnp, Bool.false_eq_true, ↓reduceIte]
      omega
    · have hb : (n == k) = false := by simpa using hk
      simp only [List.lookup, hb] at hl
      have := ih hl
      rw [heq (k, t) (Ne.symm hk)]
      split <;> omega

/-- Resolve names to a structural type (Rust `resolve_quiet`); `none` for an unknown name or a
cycle of names. -/
def resolve (D : Decls) (seen : List String) (t : Ty) : Option Ty :=
  match t with
  | .named n =>
    if _h : seen.contains n then none else
    match _hl : D.lookup n with
    | none => none
    | some u => resolve D (n :: seen) u
  | t => some t
termination_by unexpanded D seen
decreasing_by exact unexpanded_lt _hl _h

/-- A field or payload of this type may be absent: it resolves to `optional<T>` or `value`
(Rust `resolve_quiet(..)` matching `Optional | Value`, `may_be_absent`). -/
def optish (D : Decls) (t : Ty) : Bool :=
  match resolve D [] t with
  | some (.optional _) | some .value => true
  | _ => false

end Vertep.Types
