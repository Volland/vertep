import Lean.Data.Json
import VertepLean.Json
import VertepLean.Types.Check

/-!
# The type checker as a differential oracle (verification backlog 4.5)

`vertep-types` reads cases of the Rust value checker and requires the model's `check` to give
the same verdict on each:

```
vertep-types --cases CASES.jsonl
```

One line per case: `{"decls": Declarations, "ty": Type, "value": Value, "ok": bool}`, with
`Declarations`, `Type` and `Value` in serde's default representation of the Rust types of
`vertep_core::types` and `vertep_core::datamodel` (`{"Primitive": "Int"}`, `"Value"`,
`{"Record": [{"name", "ty"}]}`, `{"Union": [{"tag", "payload"}]}`, `{"List": …}`,
`{"Optional": …}`, `{"Named": "A"}`), and `ok` whether `Declarations::check` accepted the
value. Spans and event declarations are ignored. Numbers are exact decimals here and `f64` in
Rust, so cases near the edges of the signed 64-bit range can differ for that reason alone.
Exit codes: `0` all agree, `1` the first disagreement on standard error, `3` an input error.
-/

namespace Vertep.Types.Oracle

open Lean (Json)

abbrev R := Except String

def primFromJson : Json → R Prim
  | .str "Bool" => pure .bool
  | .str "Int" => pure .int
  | .str "Decimal" => pure .decimal
  | .str "String" => pure .string
  | .str "Datetime" => pure .datetime
  | .str "Iri" => pure .iri
  | j => throw s!"unknown primitive {j.compress}"

/-- A serde `Type`, nested at most `fuel` deep. -/
def tyFromJson : Nat → Json → R Ty
  | 0, _ => throw "type nested too deep"
  | fuel + 1, j => do
    match ← Vertep.Json.tagged j with
    | ("Value", _) => pure .value
    | ("Primitive", p) => pure (.prim (← primFromJson p))
    | ("Named", n) => pure (.named (← n.getStr?))
    | ("List", t) => pure (.list (← tyFromJson fuel t))
    | ("Optional", t) => pure (.optional (← tyFromJson fuel t))
    | ("Record", fs) => do
      let fs ← (← fs.getArr?).toList.mapM fun f => do
        let name ← (← f.getObjVal? "name").getStr?
        let t ← tyFromJson fuel (← f.getObjVal? "ty")
        pure (name, t)
      pure (.record fs)
    | ("Union", vs) => do
      let vs ← (← vs.getArr?).toList.mapM fun v => do
        let tag ← (← v.getObjVal? "tag").getStr?
        let p := (v.getObjVal? "payload").toOption.filter (· != .null)
        let pt ← p.mapM (tyFromJson fuel)
        pure (tag, pt)
      pure (.union vs)
    | (t, _) => throw s!"unknown type variant {t}"

/-- The named types of a serde `Declarations`. -/
def declsFromJson (j : Json) : R Decls := do
  (← (← j.getObjVal? "types").getArr?).toList.mapM fun d => do
    pure ((← (← d.getObjVal? "name").getStr?), (← tyFromJson 256 (← d.getObjVal? "ty")))

/-- One case: the block, the type, the value and the Rust verdict. -/
def caseFromJson (j : Json) : R (Decls × Ty × Value × Bool) := do
  pure (← declsFromJson (← j.getObjVal? "decls"), ← tyFromJson 256 (← j.getObjVal? "ty"),
    ← Vertep.Json.valueFromJson (← j.getObjVal? "value"), ← (← j.getObjVal? "ok").getBool?)

/-- The command line. -/
-- @tg: implements:: [[openspec:formal-model#Type Checker Correctness]] {note: "differential oracle"}
def run (args : List String) : IO UInt32 := do
  let (some path) := (match args with
    | ["--cases", p] => some p
    | _ => none)
    | IO.eprintln "usage: vertep-types --cases CASES.jsonl"; return 3
  let lines := (← IO.FS.lines path).toList.filter (fun l => !l.all Char.isWhitespace)
  let mut n := 0
  for line in lines do
    n := n + 1
    match Json.parse line >>= caseFromJson with
    | .error e => IO.eprintln s!"case {n}: {e}"; return 3
    | .ok (D, t, v, ok) =>
      let lean := check D [] t v
      if lean != ok then
        IO.eprintln s!"case {n}: Rust {if ok then "accepts" else "rejects"}, Lean {if lean then "accepts" else "rejects"}: {line}"
        return 1
  IO.println s!"{n} cases agree"
  return 0

end Vertep.Types.Oracle
