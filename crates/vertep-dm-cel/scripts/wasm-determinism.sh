#!/usr/bin/env bash
# Run the CEL determinism corpus (tests/determinism.rs) on wasm32-wasip1 under Node's WASI,
# against the same golden file the native test uses
# ([[openspec:cel-datamodel#Deterministic Evaluation#Same result on two targets]]).
# Needs: rustup target add wasm32-wasip1; Node 20 or later.
set -euo pipefail
cd "$(dirname "$0")/../../.."
target=wasm32-wasip1
wasm="$(cargo test -p vertep-dm-cel --target "$target" --test determinism --no-run --message-format=json \
  | node -e '
      let wasm = "";
      require("readline").createInterface({ input: process.stdin }).on("line", (l) => {
        try {
          const m = JSON.parse(l);
          if (m.reason === "compiler-artifact" && m.target.name === "determinism" && m.executable) wasm = m.executable;
        } catch {}
      }).on("close", () => { if (!wasm) process.exit(1); console.log(wasm); });')"
echo "running $wasm"
node --no-warnings crates/vertep-dm-cel/scripts/wasi-run.mjs "$wasm" --test-threads=1
