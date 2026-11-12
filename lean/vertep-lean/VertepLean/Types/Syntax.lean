import VertepLean.Types.Basic

/-!
# The textual syntax of declaration blocks

Mirrors the parser of `crates/vertep-core/src/types/syntax.rs` (`parse_declarations`) and the
reading of a chart's `vt:types` element in `crates/vertep-core/src/types/chart.rs`
(`parse_types_element`: the element's text content, entities and CDATA sections decoded), so
that the Lean side finds the same named types in a chart as Rust does. Error messages and
offsets are not mirrored; only whether a block parses and what it parses to.

```text
type      := '|' variant ('|' variant)* | variant ('|' variant)+ | tag atom | atom
variant   := tag atom?
atom      := bool | int | decimal | string | datetime | iri | value
           | list '<' type '>' | optional '<' type '>'
           | '{' (name ':' type (',' name ':' type)* ','?)? '}' | '(' type ')' | Name
decls     := (('type' Name '=' type | 'event' event-name (':' type)?) ';')*
```

The parser runs on fuel (the length of the text bounds the number of tokens, hence the nesting
depth), so it is total; it never runs out on a block of that length.
-/

namespace Vertep.Types.Syntax

/-- Rust `u8::is_ascii_whitespace`: space, tab, line feed, form feed, carriage return. -/
def asciiWs (c : Char) : Bool :=
  c == ' ' || c == '\t' || c == '\n' || c == '\x0C' || c == '\r'

/-- Rust `char::is_whitespace` (the Unicode `White_Space` property). -/
def unicodeWs (c : Char) : Bool :=
  let n := c.toNat
  (0x09 ≤ n && n ≤ 0x0D) || n == 0x20 || n == 0x85 || n == 0xA0 || n == 0x1680 ||
    (0x2000 ≤ n && n ≤ 0x200A) || n == 0x2028 || n == 0x2029 || n == 0x202F || n == 0x205F ||
    n == 0x3000

def asciiAlpha (c : Char) : Bool := ('a' ≤ c && c ≤ 'z') || ('A' ≤ c && c ≤ 'Z')
def asciiDigit (c : Char) : Bool := '0' ≤ c && c ≤ '9'
def identStart (c : Char) : Bool := asciiAlpha c || c == '_'
def identChar (c : Char) : Bool := asciiAlpha c || asciiDigit c || c == '_'

/-- Names reserved by the type language (Rust `RESERVED`). -/
def reserved (n : String) : Bool :=
  ["bool", "int", "decimal", "string", "datetime", "iri", "value", "list", "optional"].contains n

/-- Rust `Primitive::from_name`. -/
def primOfName : String → Option Prim
  | "bool" => some .bool
  | "int" => some .int
  | "decimal" => some .decimal
  | "string" => some .string
  | "datetime" => some .datetime
  | "iri" => some .iri
  | _ => none

/-- The value of a hex digit. -/
def hexVal (c : Char) : Option Nat :=
  if asciiDigit c then some (c.toNat - '0'.toNat)
  else if 'a' ≤ c && c ≤ 'f' then some (c.toNat - 'a'.toNat + 10)
  else if 'A' ≤ c && c ≤ 'F' then some (c.toNat - 'A'.toNat + 10)
  else none

/-- Digits in a radix, at least one (Rust `u32::from_str_radix` after its optional `+`). -/
def digitsVal (radix : Nat) (cs : List Char) : Option Nat :=
  if cs.isEmpty then none else
  cs.foldl (fun acc c => do
    let a ← acc
    let d ← hexVal c
    if d < radix then pure (a * radix + d) else none) (some 0)

/-- Rust `u32::from_str_radix(s, radix)`: an optional `+`, then digits; `none` past `u32`. -/
def parseU32 (radix : Nat) (cs : List Char) : Option Nat :=
  let cs := match cs with
    | '+' :: r => r
    | r => r
  match digitsVal radix cs with
  | some n => if n < 2 ^ 32 then some n else none
  | none => none

/-- Rust `char::from_u32`: a Unicode scalar value. -/
def scalar (n : Nat) : Option Char :=
  if n < 0xD800 || (0xDFFF < n && n < 0x110000) then some (Char.ofNat n) else none

/-! ## Lexer -/

inductive Tok where
  | ident (s : String)
  | str (s : String)
  | punct (c : Char)
  | eof
  deriving BEq, Repr, Inhabited

abbrev R := Except String

mutual
/-- Skip whitespace and `#` comments (Rust `skip_ws`). -/
def skipWs : List Char → List Char
  | [] => []
  | c :: cs => if c == '#' then skipComment cs else if asciiWs c then skipWs cs else c :: cs
/-- The rest of a comment, up to and including its line feed. -/
def skipComment : List Char → List Char
  | [] => []
  | c :: cs => if c == '\n' then skipWs cs else skipComment cs
end

/-- The rest of a double-quoted string after its opening quote (Rust `string`), with fuel: the
string and the text after the closing quote. Every step consumes a character, so the length of
the text is enough fuel. -/
def lexStringFuel : Nat → List Char → String → R (String × List Char)
  | 0, _, _ => throw "unterminated string"
  | _ + 1, [], _ => throw "unterminated string"
  | _ + 1, '"' :: cs, acc => pure (acc, cs)
  | _ + 1, '\\' :: [], _ => throw "unterminated escape"
  | fuel + 1, '\\' :: e :: cs, acc =>
    match e with
    | '"' => lexStringFuel fuel cs (acc.push '"')
    | '\\' => lexStringFuel fuel cs (acc.push '\\')
    | 'n' => lexStringFuel fuel cs (acc.push '\n')
    | 'r' => lexStringFuel fuel cs (acc.push '\r')
    | 't' => lexStringFuel fuel cs (acc.push '\t')
    | 'u' =>
      match cs with
      | '{' :: r =>
        let (h, rest) := r.span (· != '}')
        match rest, (parseU32 16 h).bind scalar with
        | _ :: after, some ch => lexStringFuel fuel after (acc.push ch)
        | _, _ => throw "bad \\u{...} escape"
      | _ => throw "bad \\u{...} escape"
    | _ => throw s!"unknown escape `\\{e}`"
  | fuel + 1, c :: cs, acc => lexStringFuel fuel cs (acc.push c)

/-- The rest of a double-quoted string after its opening quote (Rust `string`). -/
def lexString (cs : List Char) (acc : String) : R (String × List Char) :=
  lexStringFuel (cs.length + 1) cs acc

/-- The next token and the text after it (Rust `lex`). -/
def lex (src : List Char) : R (Tok × List Char) :=
  match skipWs src with
  | [] => pure (.eof, [])
  | c :: cs =>
    if identStart c then
      let (id, rest) := cs.span identChar
      pure (.ident (String.ofList (c :: id)), rest)
    else if c == '"' then do
      let (s, rest) ← lexString cs ""
      pure (.str s, rest)
    else if "{}<>()|,:;=".contains c then pure (.punct c, cs)
    else throw s!"unexpected character `{c}`"

/-! ## Parser -/

/-- The next token without consuming it. -/
def peek (src : List Char) : R Tok := do pure (← lex src).1

/-- Consume the punctuation `c` if it comes next. -/
def eat (c : Char) (src : List Char) : R (Bool × List Char) := do
  let (t, rest) ← lex src
  if t == .punct c then pure (true, rest) else pure (false, src)

def expect (c : Char) (src : List Char) : R (List Char) := do
  match ← eat c src with
  | (true, rest) => pure rest
  | _ => throw s!"expected `{c}`"

/-- Whether a variant payload starts here: `{`, `(` or an identifier. -/
def startsPayload (src : List Char) : R Bool := do
  match ← peek src with
  | .punct '{' | .punct '(' | .ident _ => pure true
  | _ => pure false

/-- An event name (Rust `event_name`): a quoted string or a run of characters other than
whitespace, `:`, `;`, `#` and `"`. -/
def eventName (src : List Char) : R (String × List Char) :=
  match skipWs src with
  | '"' :: cs => lexString cs ""
  | cs =>
    let (n, rest) := cs.span fun c => !(unicodeWs c || ":;#\"".contains c)
    if n.isEmpty then throw "expected an event name" else pure (String.ofList n, rest)

mutual
/-- A type (Rust `ty`). -/
def ty : Nat → List Char → R (Ty × List Char)
  | 0, _ => throw "type nested too deep"
  | fuel + 1, src => do
    let (bar, src') ← eat '|' src
    if bar then
      let (v, rest) ← variant fuel src'
      unionRest fuel [v] rest
    else
      let (t, rest) ← lex src
      match t with
      | .str tag =>
        let (v, rest) ← variantPayload fuel tag rest
        unionRest fuel [v] rest
      | .ident name =>
        if reserved name then atomNoBar fuel src
        else if (← peek rest) == .punct '<' then throw s!"named type `{name}` takes no arguments"
        else if (← peek rest) == .punct '|' || (← startsPayload rest) then
          let (v, rest) ← variantPayload fuel name rest
          unionRest fuel [v] rest
        else pure (.named name, rest)
      | _ => atomNoBar fuel src

/-- An atom that must not be followed by `|`. -/
def atomNoBar : Nat → List Char → R (Ty × List Char)
  | 0, _ => throw "type nested too deep"
  | fuel + 1, src => do
    let (t, rest) ← atom fuel src
    if (← peek rest) == .punct '|' then throw "a union variant must start with a tag"
    else pure (t, rest)

/-- More `| variant`s (Rust `union_rest`). -/
def unionRest : Nat → List (String × Option Ty) → List Char → R (Ty × List Char)
  | 0, _, _ => throw "type nested too deep"
  | fuel + 1, vs, src => do
    let (bar, src') ← eat '|' src
    if bar then
      let (v, rest) ← variant fuel src'
      unionRest fuel (vs ++ [v]) rest
    else pure (.union vs, src)

/-- A variant: a tag and an optional payload (Rust `variant`). -/
def variant : Nat → List Char → R ((String × Option Ty) × List Char)
  | 0, _ => throw "type nested too deep"
  | fuel + 1, src => do
    let (t, rest) ← lex src
    match t with
    | .str s => variantPayload fuel s rest
    | .ident n =>
      if reserved n then throw s!"`{n}` is reserved; quote the tag"
      else variantPayload fuel n rest
    | _ => throw "expected a variant tag"

/-- The payload of a variant, if one starts here (Rust `variant_payload`). -/
def variantPayload : Nat → String → List Char → R ((String × Option Ty) × List Char)
  | 0, _, _ => throw "type nested too deep"
  | fuel + 1, tag, src => do
    if ← startsPayload src then
      let (p, rest) ← atom fuel src
      pure ((tag, some p), rest)
    else pure ((tag, none), src)

/-- An atom (Rust `atom`). -/
def atom : Nat → List Char → R (Ty × List Char)
  | 0, _ => throw "type nested too deep"
  | fuel + 1, src => do
    let (t, rest) ← lex src
    match t with
    | .punct '{' => recordRest fuel [] rest
    | .punct '(' =>
      let (t, rest) ← ty fuel rest
      let rest ← expect ')' rest
      pure (t, rest)
    | .ident n =>
      match primOfName n with
      | some p => pure (.prim p, rest)
      | none =>
        if n == "value" then pure (.value, rest)
        else if n == "list" || n == "optional" then
          let rest ← expect '<' rest
          let (inner, rest) ← ty fuel rest
          let rest ← expect '>' rest
          pure (if n == "list" then .list inner else .optional inner, rest)
        else pure (.named n, rest)
    | .eof => throw "expected a type"
    | _ => throw "expected a type"

/-- The fields of a record after `{` (Rust `record_rest`). -/
def recordRest : Nat → List (String × Ty) → List Char → R (Ty × List Char)
  | 0, _, _ => throw "type nested too deep"
  | fuel + 1, fs, src => do
    let (close, src') ← eat '}' src
    if close then pure (.record fs, src') else
    let (t, rest) ← lex src
    let name ← match t with
      | .ident n | .str n => pure n
      | _ => throw "expected a field name or `}`"
    let rest ← expect ':' rest
    let (ft, rest) ← ty fuel rest
    let fs := fs ++ [(name, ft)]
    let (comma, rest') ← eat ',' rest
    if comma then recordRest fuel fs rest'
    else
      let rest ← expect '}' rest
      pure (.record fs, rest)
end

/-- A parsed declaration block: the named types and the events, in declaration order. -/
structure Block where
  types : Decls
  events : List (String × Option Ty)

/-- The declarations (Rust `parse_declarations`), with `fuel` declarations at most. -/
def declsLoop : Nat → Block → List Char → R Block
  | 0, _, _ => throw "too many declarations"
  | fuel + 1, b, src => do
    let (t, rest) ← lex src
    match t with
    | .eof => pure b
    | .ident "type" =>
      let (n, rest) ← lex rest
      let name ← match n with
        | .ident n => if reserved n then throw s!"`{n}` is reserved" else pure n
        | _ => throw "expected a type name"
      let rest ← expect '=' rest
      let (t, rest) ← ty (src.length + 1) rest
      finish fuel { b with types := b.types ++ [(name, t)] } rest
    | .ident "event" =>
      let (name, rest) ← eventName rest
      let (colon, rest') ← eat ':' rest
      if colon then
        let (t, rest) ← ty (src.length + 1) rest'
        finish fuel { b with events := b.events ++ [(name, some t)] } rest
      else finish fuel { b with events := b.events ++ [(name, none)] } rest
    | _ => throw "expected `type` or `event`"
where
  /-- After a declaration: `;`, or the end of the text. -/
  finish (fuel : Nat) (b : Block) (src : List Char) : R Block := do
    let (semi, rest) ← eat ';' src
    if semi then declsLoop fuel b rest
    else if (← peek src) == .eof then declsLoop fuel b src
    else throw "expected `;`"

/-- Parse a declaration block (Rust `parse_declarations`). -/
def parseDeclarations (src : String) : R Block :=
  declsLoop (src.length + 1) { types := [], events := [] } src.toList

/-- Parse a type expression (Rust `parse_type`). -/
def parseType (src : String) : R Ty := do
  let (t, rest) ← ty (src.length + 1) src.toList
  if (← peek rest) == .eof then pure t else throw "unexpected text after the type"

/-! ## The `vt:types` element -/

/-- Rust `str::trim`: Unicode whitespace at both ends. -/
def trim (cs : List Char) : List Char :=
  ((cs.dropWhile unicodeWs).reverse.dropWhile unicodeWs).reverse

/-- The end of the start tag: the text before the first `>` outside quoted attribute values,
and the text after it. -/
def startTag : Option Char → List Char → List Char → Option (List Char × List Char)
  | _, _, [] => none
  | some q, acc, c :: cs => startTag (if c == q then none else some q) (c :: acc) cs
  | none, acc, c :: cs =>
    if c == '"' || c == '\'' then startTag (some c) (c :: acc) cs
    else if c == '>' then some (acc.reverse, cs)
    else startTag none (c :: acc) cs

/-- The text before the last `</`, if any. -/
def beforeLastClose (cs : List Char) : Option (List Char) :=
  let rec go : List Char → List Char → Option (List Char) → Option (List Char)
    | [], _, best => best
    | c :: rest, acc, best =>
      go rest (c :: acc) (if c == '<' && rest.head? == some '/' then some acc.reverse else best)
  go cs [] none

/-- Whether `p` is a prefix of `cs`, and the rest. -/
def stripPrefix (p : List Char) (cs : List Char) : Option (List Char) :=
  if p.isPrefixOf cs then some (cs.drop p.length) else none

/-- The text before the first occurrence of `p` in `cs`, and the text after it. -/
def splitOn (p : List Char) : List Char → Option (List Char × List Char)
  | [] => if p.isEmpty then some ([], []) else none
  | c :: cs =>
    match stripPrefix p (c :: cs) with
    | some rest => some ([], rest)
    | none => (splitOn p cs).map fun (a, b) => (c :: a, b)

/-- An entity name (between `&` and `;`) as a character (Rust `parse_types_element`). -/
def entity (name : List Char) : Option Char :=
  match String.ofList name with
  | "lt" => some '<'
  | "gt" => some '>'
  | "amp" => some '&'
  | "quot" => some '"'
  | "apos" => some '\''
  | _ =>
    match name with
    | '#' :: 'x' :: h => (parseU32 16 h).bind scalar
    | '#' :: d => (parseU32 10 d).bind scalar
    | _ => none

/-- The text content of an element's inside: entities and CDATA sections decoded, no child
elements. -/
def textContent : Nat → List Char → R (List Char)
  | 0, _ => throw "vt:types: too much markup"
  | fuel + 1, cs =>
    match cs.span (fun c => c != '&' && c != '<') with
    | (text, []) => pure text
    | (text, rest) =>
      match stripPrefix "<![CDATA[".toList rest with
      | some r =>
        match splitOn "]]>".toList r with
        | some (cdata, after) => do pure (text ++ cdata ++ (← textContent fuel after))
        | none => throw "vt:types: unterminated CDATA"
      | none =>
        if rest.head? == some '<' then throw "vt:types: only text content is allowed"
        else
          match (rest.drop 1).span (· != ';') with
          | (name, _ :: after) =>
            match entity name with
            | some c => do pure (text ++ [c] ++ (← textContent fuel after))
            | none => throw "vt:types: bad entity"
          | (_, []) => throw "vt:types: bad entity"

/-- The text content of a serialized `vt:types` element (Rust `parse_types_element` before
`parse_declarations`); `none` for an empty element `<vt:types/>`. -/
def typesElementText (xml : String) : R (Option String) := do
  let xml := trim xml.toList
  let some (start, after) := startTag none [] xml | throw "vt:types: unterminated start tag"
  if start.getLast? == some '/' then return none
  let some inner := beforeLastClose after | throw "vt:types: missing end tag"
  pure (some (String.ofList (← textContent (inner.length + 1) inner)))

/-- The declaration block of a serialized `vt:types` element (Rust `parse_types_element`). -/
def parseTypesElement (xml : String) : R Block := do
  match ← typesElementText xml with
  | none => pure { types := [], events := [] }
  | some text => parseDeclarations text

end Vertep.Types.Syntax
