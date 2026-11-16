#!/usr/bin/env bash
# Writes the CycloneDX SBOMs of a release into dist/sbom/:
#   - one per workspace crate (cargo-cyclonedx, every target platform, all features): the crates on
#     crates.io, the `vt` binary (vertep-cli) and the Rust inside the wheel (vertep-py);
#   - one per npm package (npm sbom): @vertep/web and @vertep/dm-ecma.
# The release workflow (.github/workflows/release.yml) attests them and attaches them to the
# GitHub release. Needs cargo-cyclonedx (version pinned in that workflow) and npm >= 10.
set -euo pipefail
# @tg: implements:: [[openspec:production-readiness#Supply Chain Gate]] {confidence: partial, note: "the SBOM per release"}

root=$(cd "$(dirname "$0")/.." && pwd)
out="${VERTEP_DIST:-$root/dist}/sbom"
mkdir -p "$out"
rm -f "$out"/*.cdx.json

cd "$root"
# cargo-cyclonedx writes <crate>.cdx.json next to each Cargo.toml; move them out of the tree.
cargo cyclonedx --format json --spec-version 1.5 --all-features --target all -q
find crates examples -name '*.cdx.json' -not -path '*/target/*' -exec mv {} "$out"/ \;

cd "$root/bindings/js"
[ -d node_modules ] || npm ci --no-audit --no-fund >/dev/null
npm sbom --sbom-format cyclonedx --omit dev -w @vertep/web >"$out/vertep-web.npm.cdx.json"
npm sbom --sbom-format cyclonedx --omit dev -w @vertep/dm-ecma >"$out/vertep-dm-ecma.npm.cdx.json"

echo "sbom: $(ls "$out" | wc -l | tr -d ' ') SBOMs in $out" >&2
