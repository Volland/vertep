#!/usr/bin/env bash
# The one-command build: @vertep/web, @vertep/dm-ecma and the `vertep` Python wheel from a clean
# checkout, each smoke-tested, into dist/ (dist/npm/*.tgz, dist/python/*.whl).
#
#   scripts/build.sh               on this machine, with the pins of scripts/toolchain.env
#   scripts/build.sh --container   inside the pinned build image (.devcontainer/Dockerfile);
#                                  needs only Docker (or Podman: CONTAINER=podman)
#   scripts/build.sh [--container] --only npm|wheel    one of the two
#
# CI runs `scripts/build.sh --container` on a clean runner (.github/workflows/ci.yml, job
# `reproducible-build`). The container runs as the calling user, so dist/ and target/ stay yours.
set -euo pipefail
# @tg: implements:: [[openspec:production-readiness#Reproducible Toolchain]]

root=$(cd "$(dirname "$0")/.." && pwd)

if [ "${1:-}" = --container ]; then
  shift
  engine="${CONTAINER:-docker}"
  image="${VERTEP_BUILD_IMAGE:-vertep-build:local}"
  if [ "${VERTEP_SKIP_IMAGE_BUILD:-0}" != 1 ]; then
    "$engine" build -f "$root/.devcontainer/Dockerfile" -t "$image" "$root/scripts"
  fi
  # A cargo registry that outlives the container, so a second run does not download again; and a
  # target directory of its own, so host and container builds never share build-script outputs.
  registry="${VERTEP_CARGO_REGISTRY:-$root/target/container-cargo-registry}"
  mkdir -p "$registry" "$root/target"
  exec "$engine" run --rm \
    --user "$(id -u):$(id -g)" \
    -e HOME=/tmp/home -e VERTEP_PROFILE -e CARGO_TERM_COLOR \
    -e CARGO_TARGET_DIR=/work/target/container \
    -v "$root:/work" -v "$registry:/usr/local/cargo/registry" \
    -w /work "$image" scripts/build.sh "$@"
fi

only=all
if [ "${1:-}" = --only ]; then
  only="${2:-}"
fi
case "$only" in
  all | npm | wheel) ;;
  *)
    echo "usage: $0 [--container] [--only npm|wheel]" >&2
    exit 2
    ;;
esac

mkdir -p "${HOME:-/tmp/home}"
# Each half ends with its smoke test (the packed tarballs in a fresh Node.js project, the wheel in
# a fresh virtual environment); CI runs this in the pinned image from a clean checkout.
# @tg: verifies:: [[openspec:production-readiness#Reproducible Toolchain#Clean build]] {level: e2e}
if [ "$only" != wheel ]; then "$root/scripts/build-wasm.sh"; fi
if [ "$only" != npm ]; then "$root/scripts/build-wheel.sh"; fi
echo "build: done" >&2
ls -l "${VERTEP_DIST:-$root/dist}"/* >&2
