#!/usr/bin/env bash
# CI gate for vertep-lean (formal-model: Lean Package, Axiom Hygiene).
#
#  1. The library builds from the pinned toolchain; it is compiled with `warningAsError`, so a
#     `sorry` (a warning in Lean) fails this step.
#  2. The library sources contain no `sorry`, `admit`, `axiom`, `unsafe`, `implemented_by` or
#     `extern` (the `Pending/` statements are outside the library and outside this scan).
#  3. The tests build: `VertepTests.Axioms` collects the axioms of every main theorem, and the
#     build fails if any is outside `propext`, `Classical.choice`, `Quot.sound`.
#  4. The `#print axioms` report of every main theorem is printed.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== lake build VertepLean (warnings are errors)"
lake build VertepLean

echo "== source scan"
if grep -rnwE 'sorry|admit' VertepLean VertepLean.lean; then
  echo "error: sorry or admit in the library" >&2
  exit 1
fi
if grep -rnE '^\s*(private |protected |noncomputable )*(axiom|unsafe)\b|@\[(implemented_by|extern)' VertepLean VertepLean.lean; then
  echo "error: axiom, unsafe or foreign declaration in the library" >&2
  exit 1
fi

echo "== lake build vertep-oracle vt-certify vertep-shapes vertep-journal vertep-lease vertep-custody vertep-decide vertep-types"
lake build vertep-oracle vt-certify vertep-shapes vertep-journal vertep-lease vertep-custody vertep-decide vertep-types

echo "== lake build VertepTests (axioms pinned)"
lake build VertepTests

echo "== #print axioms of the main theorems"
lake env lean VertepTests/Axioms.lean 2>&1 | grep -E "depends on axioms|error" | tee /dev/stderr | \
  { ! grep -q "error"; }
echo "OK: no sorry, only propext, Classical.choice and Quot.sound"
