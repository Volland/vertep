#!/usr/bin/env bash
# Import the W3C SCXML 1.0 Implementation Report Plan (IRP) tests.
#
# Source: the verbatim copy of https://www.w3.org/Voice/2013/scxml-irp/ kept in the uscxml
# repository (test/w3c/{manifest.xml,confEcma.xsl,txml/}), pinned by commit below. The W3C
# server serves a bot challenge to scripted clients, so it cannot be fetched directly in CI.
#
# Each .txml template becomes an ecmascript variant through the pinned confEcma.xsl, which is
# XSLT 2.0 and therefore runs on SaxonJS (xslt3), not xsltproc. Tests written for the null
# datamodel are kept as null variants. suite/SHA256SUMS lets CI detect drift.
#
# Templated tests that do not depend on ECMAScript also get a cel variant (M1 task 6.10)
# through confCel.xsl, which is Vertep's own stylesheet and is not taken from the mirror.
# suite/cel/EXCLUDED.tsv records why every other test has no cel variant.
# Re-run only to re-import; the generated files are committed.
#
# @tg: implements:: [[openspec:scxml-conformance#IRP Suite Import]]
set -euo pipefail

MIRROR_REPO="tklab-tud/uscxml"
MIRROR_REV="11013b76a02d1534c7375a70f77efd2463798ddd"
XSLT3="xslt3@2.6.0"

HERE="$(cd "$(dirname "$0")/.." && pwd)"
SUITE="$HERE/suite"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

command -v npx >/dev/null || { echo "npx (Node.js) is required for $XSLT3" >&2; exit 1; }

curl -fsSL "https://codeload.github.com/$MIRROR_REPO/tar.gz/$MIRROR_REV" | tar -xz -C "$WORK"
SRC="$WORK/uscxml-$MIRROR_REV/test/w3c"

rm -rf "$SUITE/raw" "$SUITE/ecmascript" "$SUITE/null" "$SUITE/cel"
mkdir -p "$SUITE/raw" "$SUITE/ecmascript" "$SUITE/null" "$SUITE/cel"
cp "$SRC/manifest.xml" "$SRC/confEcma.xsl" "$SUITE/"
cat > "$SUITE/SOURCE" <<EOF
W3C SCXML 1.0 IRP (https://www.w3.org/Voice/2013/scxml-irp/), Copyright W3C (MIT, ERCIM, Keio,
Beihang), W3C Software and Document License. Copied from $MIRROR_REPO@$MIRROR_REV test/w3c,
transformed with confEcma.xsl on $XSLT3.
EOF

# Every start and dep file named in the manifest, laid out as <id>/<file>.
grep -oE '<(start|dep) uri="[^"]+"' "$SUITE/manifest.xml" | sed -E 's/.*uri="([^"]+)"/\1/' | sort -u |
while read -r uri; do
  name="$(basename "$uri")"
  if [ ! -f "$SRC/txml/$name" ]; then echo "missing in mirror: $uri" >&2; exit 1; fi
  mkdir -p "$SUITE/raw/$(dirname "$uri")"
  cp "$SRC/txml/$name" "$SUITE/raw/$uri"
done

# Generate variants.
find "$SUITE/raw" -type f | LC_ALL=C sort | while read -r f; do
  rel="${f#"$SUITE/raw"/}"
  dir="$(dirname "$rel")"
  case "$f" in
    *.txml)
      variant=ecmascript
      grep -q 'datamodel="null"' "$f" && variant=null
      out="$SUITE/$variant/$dir/$(basename "${rel%.txml}").scxml"
      mkdir -p "$(dirname "$out")"
      npx -y "$XSLT3" -xsl:"$SUITE/confEcma.xsl" -s:"$f" -o:"$out" >/dev/null
      ;;
    *)
      mkdir -p "$SUITE/ecmascript/$dir"
      cp "$f" "$SUITE/ecmascript/$dir/"
      ;;
  esac
done

# CEL variants: only templates (tests that leave the datamodel to conf:datamodel), and only
# those that need nothing ECMAScript-specific. Every test without a cel variant gets a line
# in cel/EXCLUDED.tsv.
CEL_RAW_ONLY='conf:(eventRaw|methodIsPost|eventNamedParamHasValue|eventIdParamHasValue|messageBodyEquals|getNamedParamVal|getIDParamVal|getNthParamName|getNthParamVal|msgIsBody|scxmlEventExpr)\b'
printf 'test\treason\n' > "$SUITE/cel/EXCLUDED.tsv"
for dir in $(ls "$SUITE/raw" | LC_ALL=C sort -n); do
  txml=$(find "$SUITE/raw/$dir" -name '*.txml' | LC_ALL=C sort)
  reason=""
  if [ -z "$txml" ]; then
    reason="no SCXML document: the test is a description for a manual check"
  elif ! grep -q 'conf:datamodel' $txml; then
    dm=$( (grep -ho 'datamodel="[a-z]*"' $txml || true) | head -1 | sed -E 's/datamodel="([a-z]*)"/\1/')
    reason="not a template: written for the ${dm:-default (null)} datamodel"
  elif grep -qE '<script|conf:script' $txml; then
    reason="uses <script>, which the expression-only CEL datamodel does not have"
  elif grep -qE "$CEL_RAW_ONLY" $txml; then
    reason="matches ECMAScript regular expressions against _event.raw of the Basic HTTP processor, which the CEL event does not have"
  fi
  if [ -n "$reason" ]; then
    printf '%s\t%s\n' "$dir" "$reason" >> "$SUITE/cel/EXCLUDED.tsv"
    continue
  fi
  mkdir -p "$SUITE/cel/$dir"
  for f in $(find "$SUITE/raw/$dir" -type f | LC_ALL=C sort); do
    case "$f" in
      *.txml) npx -y "$XSLT3" -xsl:"$SUITE/confCel.xsl" -s:"$f" -o:"$SUITE/cel/$dir/$(basename "${f%.txml}").scxml" >/dev/null ;;
      *) cp "$f" "$SUITE/cel/$dir/" ;;
    esac
  done
done

(cd "$SUITE" && find manifest.xml confEcma.xsl confCel.xsl SOURCE raw ecmascript null cel -type f | LC_ALL=C sort | xargs shasum -a 256) > "$SUITE/SHA256SUMS"
echo "imported $(find "$SUITE/ecmascript" -name '*.scxml' | wc -l | tr -d ' ') ecmascript, $(find "$SUITE/null" -name '*.scxml' | wc -l | tr -d ' ') null and $(find "$SUITE/cel" -name '*.scxml' | wc -l | tr -d ' ') cel charts; $(($(wc -l < "$SUITE/cel/EXCLUDED.tsv") - 1)) tests have no cel variant"
