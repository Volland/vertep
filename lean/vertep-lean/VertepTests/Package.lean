import Lean.Data.Json
import VertepLean

/-!
# The Lean package

Building this module (`lake build VertepTests`) elaborates the whole library, so every theorem
is checked; the pin check below reads the manifest at build time.
-/

namespace Vertep.Test

/-- Every theorem of the library is in scope and checked. -/
-- @tg: verifies:: [[openspec:formal-model#Lean Package#Clean build]] {level: unit}
theorem clean_build_checks_theorems :
    (∀ c, checkWF c = true ↔ WellFormed c) ∧ (∀ cc, TypedStepSoundness cc dynamicTypes) :=
  ⟨checkWF_iff, dynamic_typed_soundness⟩

/-- A 40-character lowercase hexadecimal git revision. -/
def isFullRev (s : String) : Bool :=
  s.length == 40 && s.toList.all (fun c => c.isDigit || ('a' ≤ c && c ≤ 'f'))

/-- The toolchain is an exact release and every dependency, LeanRDF included, is pinned to a
full git revision that is also the revision the lakefile asks for. -/
def pinnedDependencies : IO Bool := do
  let tc := (← IO.FS.readFile "lean-toolchain").trimAscii.toString
  let man ← IO.FS.readFile "lake-manifest.json"
  let some j := (Lean.Json.parse man).toOption | return false
  let some pkgs := (j.getObjValAs? (Array Lean.Json) "packages").toOption | return false
  let field := fun (p : Lean.Json) (k : String) =>
    ((p.getObjValAs? String k).toOption).getD ""
  let names : List String := pkgs.toList.map (fun p => field p "name")
  let pinned : List Bool := pkgs.toList.map (fun p =>
    isFullRev (field p "rev") && field p "rev" == field p "inputRev")
  pure (tc == "leanprover/lean4:v4.35.0" && names.contains "leanrdf" && pinned.all id)

-- @tg: verifies:: [[openspec:formal-model#Lean Package#Pinned dependency]] {level: unit}
#eval show IO Unit from do
  unless (← pinnedDependencies) do
    throw (IO.userError "the Lean toolchain or a dependency is not pinned to an exact revision")

end Vertep.Test
