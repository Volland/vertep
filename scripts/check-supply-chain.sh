#!/usr/bin/env bash
# The supply-chain gate of the Rust workspace: cargo deny with deny.toml (advisories, licences,
# bans, sources). CI runs it on every push and the release workflow on the tagged commit.
#
#   scripts/check-supply-chain.sh               check the workspace
#   scripts/check-supply-chain.sh --self-test   prove deny.toml rejects a crate with an advisory
#
# The self-test builds a throwaway crate that depends on smallvec 1.6.0 (RUSTSEC-2021-0003, a
# buffer overflow fixed in 1.6.1) and expects deny.toml to fail it and name the advisory, so an
# over-broad exception or a disabled check cannot pass silently. Needs cargo-deny and the network.
set -euo pipefail
# @tg: implements:: [[openspec:production-readiness#Supply Chain Gate]] {confidence: partial, note: "the advisory and licence gate; deny.toml holds the policy"}

root=$(cd "$(dirname "$0")/.." && pwd)

# @tg: verifies:: [[openspec:production-readiness#Supply Chain Gate#An advisory blocks a release]] {level: integration}
self_test() {
  local tmp out
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' RETURN
  mkdir -p "$tmp/vulnerable/src"
  cat >"$tmp/vulnerable/Cargo.toml" <<'EOF'
[package]
name = "vertep-supply-chain-fixture"
version = "0.0.0"
edition = "2021"
license = "MIT OR Apache-2.0"
publish = false

[dependencies]
smallvec = "=1.6.0"
EOF
  echo >"$tmp/vulnerable/src/lib.rs"
  if out=$(cd "$tmp/vulnerable" && cargo deny --manifest-path Cargo.toml --config "$root/deny.toml" check advisories 2>&1); then
    echo "self-test: deny.toml let a crate with RUSTSEC-2021-0003 through" >&2
    return 1
  fi
  case "$out" in
    *RUSTSEC-2021-0003*) echo "self-test: deny.toml fails a dependency with an open advisory and names it" >&2 ;;
    *)
      echo "self-test: cargo deny failed without naming RUSTSEC-2021-0003:" >&2
      echo "$out" >&2
      return 1
      ;;
  esac
}

case "${1:-}" in
  --self-test) self_test ;;
  "") cd "$root" && cargo deny --locked check ;;
  *)
    echo "usage: $0 [--self-test]" >&2
    exit 2
    ;;
esac
