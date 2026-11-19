#!/bin/sh
# Measures the size and cold start of @vertep/web and of the Python wheel from artifacts that are
# already built, and prints one JSON object:
#
#   {"web_wasm_bytes": .., "web_wasm_gzip_bytes": .., "web_journal_wasm_bytes": ..,
#    "web_journal_wasm_gzip_bytes": .., "wheel_bytes": .., "web_cold_start_ms": ..,
#    "wheel_import_ms": ..}
#
# It never builds anything: a figure whose artifact is missing (or whose runtime, node or
# python3, is not installed) is null. Build first with scripts/build-wasm.sh (WebAssembly in
# bindings/js/packages/web/wasm/{core,journal}/, the package in bindings/js/packages/web/dist/)
# and scripts/build-wheel.sh (dist/python/vertep-*.whl; VERTEP_DIST moves dist/).
#
# Cold start: one `node` process imports @vertep/web from the built package, creates an actor
# system on memory storage and terminates it. Wheel import: `import vertep` in a fresh virtual
# environment with only the wheel installed. Both are wall-clock milliseconds of one run.
#
# @tg: implements:: [[openspec:production-readiness#Benchmark Suite]]
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
dist="${VERTEP_DIST:-$root/dist}"
web="$root/bindings/js/packages/web"

json() { if [ -n "${1:-}" ]; then printf '%s' "$1"; else printf 'null'; fi; }
bytes() { [ -f "$1" ] && wc -c <"$1" | tr -d ' ' || true; }
gzbytes() { [ -f "$1" ] && gzip -9 -c "$1" | wc -c | tr -d ' ' || true; }

core_wasm="$web/wasm/core/vertep_wasm_bg.wasm"
journal_wasm="$web/wasm/journal/vertep_wasm_bg.wasm"
web_wasm=$(bytes "$core_wasm")
web_wasm_gz=$(gzbytes "$core_wasm")
journal=$(bytes "$journal_wasm")
journal_gz=$(gzbytes "$journal_wasm")

wheel_file=$(ls "$dist"/python/vertep-*.whl 2>/dev/null | head -n 1 || true)
wheel=""
[ -n "$wheel_file" ] && wheel=$(bytes "$wheel_file")

cold=""
if [ -f "$web/dist/index.js" ] && [ -n "$web_wasm" ] && command -v node >/dev/null 2>&1; then
  cold=$(node --input-type=module -e "
    const t0 = performance.now();
    const { createActorSystem } = await import('$web/dist/index.js');
    const system = await createActorSystem({ storage: 'memory' });
    await system.terminate();
    console.log((performance.now() - t0).toFixed(1));
  " 2>/dev/null || true)
fi

import_ms=""
if [ -n "$wheel_file" ] && command -v python3 >/dev/null 2>&1; then
  venv=$(mktemp -d)
  trap 'rm -rf "$venv"' EXIT
  if python3 -m venv "$venv/v" >/dev/null 2>&1 &&
    "$venv/v/bin/pip" install --quiet --no-index "$wheel_file" >/dev/null 2>&1; then
    import_ms=$(cd "$venv" && "$venv/v/bin/python" -I -c "
import time
t0 = time.perf_counter()
import vertep
print(f'{(time.perf_counter() - t0) * 1000:.1f}')
" 2>/dev/null || true)
  fi
fi

printf '{"web_wasm_bytes": %s, "web_wasm_gzip_bytes": %s, "web_journal_wasm_bytes": %s, "web_journal_wasm_gzip_bytes": %s, "wheel_bytes": %s, "web_cold_start_ms": %s, "wheel_import_ms": %s}\n' \
  "$(json "$web_wasm")" "$(json "$web_wasm_gz")" "$(json "$journal")" "$(json "$journal_gz")" \
  "$(json "$wheel")" "$(json "$cold")" "$(json "$import_ms")"
