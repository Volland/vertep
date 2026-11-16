#!/usr/bin/env bash
# Verifies release files the way a consumer would: each file's signed build-provenance attestation
# must check against this repository and a workflow of it. The release workflows run it right after
# attesting, so a release never ships a file that `gh attestation verify` would reject.
#
#   scripts/verify-attestations.sh dist/npm/*.tgz
#   GH_TOKEN=... VERTEP_REPO=Volland/vertep scripts/verify-attestations.sh vertep-web-0.1.11.tgz
#
# Needs the GitHub CLI (gh >= 2.49) and a token that can read the repository.
set -euo pipefail
# @tg: implements:: [[openspec:production-readiness#Supply Chain Gate]] {confidence: partial, note: "consumer-side verification of the attestations"}
# @tg: implements:: [[decisions#Release artifacts carry signed build provenance]] {confidence: partial, note: "the check; the attestations are made in the release workflows"}

repo="${VERTEP_REPO:-${GITHUB_REPOSITORY:-Volland/vertep}}"
[ $# -gt 0 ] || {
  echo "usage: $0 FILE..." >&2
  exit 2
}

# @tg: verifies:: [[openspec:production-readiness#Supply Chain Gate#Artifacts are verifiable]] {level: e2e}
verify() {
  local file=$1
  # The attestation must belong to this repository and be signed by one of its own workflows.
  gh attestation verify "$file" --repo "$repo" --signer-repo "$repo" >/dev/null
  echo "verified: $(basename "$file") was built by a workflow of $repo" >&2
}

for file in "$@"; do verify "$file"; done
