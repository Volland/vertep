#!/usr/bin/env bash
# Builds the `vertep` Python wheel from a clean checkout with the pinned maturin, puts it in
# dist/python/ and smoke-tests it in a fresh virtual environment (bindings/python/scripts/smoke.py,
# run isolated so nothing is imported from the checkout).
#
# The wheel is for this machine's platform and Python ABI (abi3, CPython >= 3.9). The wheels
# published to PyPI come from .github/workflows/python-release.yml (manylinux, macOS, Windows).
set -euo pipefail
# @tg: implements:: [[openspec:production-readiness#Reproducible Toolchain]] {confidence: partial, note: "the wheel half; scripts/build.sh is the one command"}

root=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck disable=SC1091
. "$root/scripts/toolchain.env"
dist="${VERTEP_DIST:-$root/dist}"
python="${PYTHON:-python3}"
say() { echo "build-wheel: $*" >&2; }

"$root/scripts/check-toolchain.sh"
cd "$root"
rustc_version=$(rustc --version | awk '{print $2}')
if [ "$rustc_version" != "$RUST_VERSION" ]; then
  say "rustc is $rustc_version, the pin is $RUST_VERSION"
  exit 1
fi

maturin_cmd=maturin
if [ "$(maturin --version 2>/dev/null | awk '{print $2}')" != "$MATURIN_VERSION" ]; then
  venv="$root/target/tools/maturin-$MATURIN_VERSION"
  if [ ! -x "$venv/bin/maturin" ]; then
    say "installing maturin $MATURIN_VERSION into target/tools"
    "$python" -m venv "$venv"
    "$venv/bin/pip" install --quiet "maturin==$MATURIN_VERSION"
  fi
  maturin_cmd="$venv/bin/maturin"
fi

mkdir -p "$dist/python"
rm -f "$dist"/python/vertep-*.whl
(cd bindings/python && "$maturin_cmd" build --release --interpreter "$python" --out "$dist/python")
wheel=$(ls "$dist"/python/vertep-*.whl)

smoke=$(mktemp -d)
trap 'rm -rf "$smoke"' EXIT
"$python" -m venv "$smoke/venv"
"$smoke/venv/bin/pip" install --quiet --no-index "$wheel"
"$smoke/venv/bin/python" -I bindings/python/scripts/smoke.py
say "built $(basename "$wheel") into $dist/python"
