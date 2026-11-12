#!/bin/sh
# Builds the WebAssembly modules and the TypeScript of the npm workspace.
#
#   core     packages/web/wasm/core      core, shell, null/cel/rust datamodels, in-memory store
#   journal  packages/web/wasm/journal   + tiramemsu on SQLite in WebAssembly (lazy-loaded)
#   ecma     packages/dm-ecma/wasm       + the boa ECMAScript datamodel (separate package)
#   test     .native/wasm-test           + the test-only exports (traces, differential records,
#                                          the W3C IRP suite runner); never published
#
# and the native helper `vertep-wasm-native` the native-versus-WebAssembly tests compare with
# (it also hosts the WebSocket server of the end-to-end tests).
# Needs the wasm32-unknown-unknown target and the wasm-bindgen CLI at the version pinned in
# crates/vertep-wasm/Cargo.toml. Set VERTEP_PROFILE=dev for a fast unoptimised build.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$here/../.." && pwd)
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target/m6}"
export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS='--cfg getrandom_backend="wasm_js"'
profile="${VERTEP_PROFILE:-release}"
if [ "$profile" = release ]; then
  export CARGO_PROFILE_RELEASE_OPT_LEVEL=z
  export CARGO_PROFILE_RELEASE_LTO=fat
  export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
  export CARGO_PROFILE_RELEASE_DEBUG=false
  export CARGO_PROFILE_RELEASE_STRIP=true
  flag=--release
  dir=release
else
  flag=
  dir=debug
fi

need=$(sed -n 's/^wasm-bindgen = "=\(.*\)"/\1/p' "$root/crates/vertep-wasm/Cargo.toml")
have=$(wasm-bindgen --version | awk '{print $2}')
if [ "$need" != "$have" ]; then
  echo "wasm-bindgen CLI $have does not match the crate's $need" >&2
  exit 2
fi

build() { # features out-dir
  echo "building vertep-wasm [$1] -> $2" >&2
  if [ -n "$1" ]; then
    cargo build -p vertep-wasm --lib --target wasm32-unknown-unknown $flag --features "$1"
  else
    cargo build -p vertep-wasm --lib --target wasm32-unknown-unknown $flag
  fi
  rm -rf "$2"
  wasm-bindgen --target web --out-dir "$2" --out-name vertep_wasm \
    "$CARGO_TARGET_DIR/wasm32-unknown-unknown/$dir/vertep_wasm.wasm"
}

build "" "$here/packages/web/wasm/core"
build journal "$here/packages/web/wasm/journal"
if [ "${VERTEP_SKIP_ECMA:-0}" != 1 ]; then
  build "journal,ecmascript" "$here/packages/dm-ecma/wasm"
  build "journal,ecmascript,conformance" "$here/.native/wasm-test"
fi

echo "building the native helper and vt" >&2
cargo build -p vertep-wasm --features journal,ecmascript,conformance --bin vertep-wasm-native
cargo build -p vertep-cli --bin vt
mkdir -p "$here/.native"
cp "$CARGO_TARGET_DIR/debug/vertep-wasm-native" "$here/.native/vertep-wasm-native"
cp "$CARGO_TARGET_DIR/debug/vt" "$here/.native/vt"

cd "$here"
npx tsc -p packages/web
npx tsc -p packages/dm-ecma
echo "built" >&2
