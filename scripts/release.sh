#!/usr/bin/env bash
# Package the CLI for the current host only.
#
# Public releases are assembled from the four target-specific CI artifacts.
# This helper is for local dogfood builds and never relabels a foreign binary.

set -euo pipefail

VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
    echo "usage: scripts/release.sh <version>" >&2
    exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${2:-$(rustc -vV | sed -n 's/^host: //p')}"

"$ROOT/scripts/package-cli.sh" "$VERSION" "$TARGET"
echo "Local artifact only. Cross-platform artifacts are produced by .github/workflows/internal-release.yml."
