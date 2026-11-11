#!/usr/bin/env bash
# Fail when the committed IRP suite differs from its recorded hashes.
set -euo pipefail
cd "$(dirname "$0")/../suite"
shasum -a 256 -c --quiet SHA256SUMS
echo "IRP suite matches SHA256SUMS"
