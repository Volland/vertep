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

rm -rf "$SUITE/raw" "$SUITE/ecmascript" "$SUITE/null"
mkdir -p "$SUITE/raw" "$SUITE/ecmascript" "$SUITE/null"
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

(cd "$SUITE" && find manifest.xml confEcma.xsl SOURCE raw ecmascript null -type f | LC_ALL=C sort | xargs shasum -a 256) > "$SUITE/SHA256SUMS"
echo "imported $(find "$SUITE/ecmascript" -name '*.scxml' | wc -l | tr -d ' ') ecmascript and $(find "$SUITE/null" -name '*.scxml' | wc -l | tr -d ' ') null charts"
