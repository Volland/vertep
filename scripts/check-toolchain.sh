#!/usr/bin/env bash
# Checks that every place naming a pinned tool version agrees with scripts/toolchain.env, and
# names both sides of any disagreement. scripts/build-wasm.sh runs it first; CI runs it with
# --self-test, which feeds it a drifted pin and expects the failure.
#
#   scripts/check-toolchain.sh               check the checkout
#   scripts/check-toolchain.sh --cli         also check the wasm-bindgen CLI on PATH
#   scripts/check-toolchain.sh --self-test   prove a drifted pin fails with both versions named
#
# VERTEP_TOOLCHAIN_ENV overrides the pins file (the self-test uses it).
set -euo pipefail
# @tg: implements:: [[openspec:production-readiness#Reproducible Toolchain]] {confidence: partial, note: "the pin drift check"}

root=$(cd "$(dirname "$0")/.." && pwd)

check() {
  local pins="${VERTEP_TOOLCHAIN_ENV:-$root/scripts/toolchain.env}" fail=0
  # shellcheck disable=SC1090
  . "$pins"

  local toolchain lib lock from
  toolchain=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$root/rust-toolchain.toml")
  if [ "$toolchain" != "$RUST_VERSION" ]; then
    echo "toolchain drift: rust-toolchain.toml channel is $toolchain, scripts/toolchain.env RUST_VERSION is $RUST_VERSION" >&2
    fail=1
  fi
  from=$(sed -n 's/^FROM rust:\([0-9.]*\)-.*/\1/p' "$root/.devcontainer/Dockerfile")
  if [ "$from" != "$RUST_VERSION" ]; then
    echo "toolchain drift: .devcontainer/Dockerfile is FROM rust:$from, scripts/toolchain.env RUST_VERSION is $RUST_VERSION" >&2
    fail=1
  fi

  lib=$(sed -n 's/^wasm-bindgen = "=\(.*\)"/\1/p' "$root/crates/vertep-wasm/Cargo.toml")
  if [ "$lib" != "$WASM_BINDGEN_VERSION" ]; then
    echo "wasm-bindgen drift: the library pin in crates/vertep-wasm/Cargo.toml is $lib, the pinned CLI (scripts/toolchain.env WASM_BINDGEN_VERSION) is $WASM_BINDGEN_VERSION" >&2
    fail=1
  fi
  lock=$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' "$root/Cargo.lock")
  if [ "$lock" != "$WASM_BINDGEN_VERSION" ]; then
    echo "wasm-bindgen drift: Cargo.lock resolves the library to $lock, the pinned CLI (scripts/toolchain.env WASM_BINDGEN_VERSION) is $WASM_BINDGEN_VERSION" >&2
    fail=1
  fi

  if [ "${1:-}" = --cli ]; then
    local cli
    cli=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}') || cli=missing
    if [ "$cli" != "$WASM_BINDGEN_VERSION" ]; then
      echo "wasm-bindgen drift: the CLI on PATH is ${cli:-missing}, the library and the pin are $WASM_BINDGEN_VERSION" >&2
      fail=1
    fi
  fi
  return $fail
}

# @tg: verifies:: [[openspec:production-readiness#Reproducible Toolchain#A version drift is caught]] {level: integration}
self_test() {
  local tmp out
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' RETURN
  sed 's/^WASM_BINDGEN_VERSION=.*/WASM_BINDGEN_VERSION=0.0.1/' "$root/scripts/toolchain.env" >"$tmp/toolchain.env"
  if out=$(VERTEP_TOOLCHAIN_ENV="$tmp/toolchain.env" check 2>&1); then
    echo "self-test: a drifted wasm-bindgen pin passed the check" >&2
    return 1
  fi
  local lib
  lib=$(sed -n 's/^wasm-bindgen = "=\(.*\)"/\1/p' "$root/crates/vertep-wasm/Cargo.toml")
  case "$out" in
    *"is $lib"*"is 0.0.1"*) ;;
    *)
      echo "self-test: the failure does not name both versions ($lib and 0.0.1):" >&2
      echo "$out" >&2
      return 1
      ;;
  esac
  check
  echo "self-test: a drifted pin fails and names both versions; the checkout passes" >&2
}

case "${1:-}" in
  --self-test) self_test ;;
  --cli | "") check "${1:-}" && echo "toolchain pins agree" >&2 ;;
  *)
    echo "usage: $0 [--cli | --self-test]" >&2
    exit 2
    ;;
esac
