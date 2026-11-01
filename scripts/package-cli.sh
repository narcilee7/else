#!/usr/bin/env bash
# Build and package the CLI for exactly one target.
#
# This script is intentionally target-scoped. Cross-platform releases are built
# by the CI matrix; one host never relabels its binary as another platform.

set -euo pipefail

VERSION="${1:-}"
TARGET="${2:-}"
BUILDER="${BUILDER:-cargo}"

if [[ -z "$VERSION" || -z "$TARGET" ]]; then
    echo "usage: scripts/package-cli.sh <version> <target>" >&2
    exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="${DIST_DIR:-$ROOT/dist}"
ARCHIVE="$DIST_DIR/else-$TARGET.tar.gz"

cd "$ROOT"

package_version="$(
    cargo metadata --no-deps --format-version 1 |
        python3 -c 'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "else-bot"))'
)"
if [[ "$VERSION" != "$package_version" ]]; then
    echo "release version $VERSION does not match else-bot Cargo version $package_version" >&2
    exit 1
fi

if ! command -v "$BUILDER" >/dev/null 2>&1; then
    echo "required build command is unavailable: $BUILDER" >&2
    exit 1
fi

echo "==> build else-bot $VERSION for $TARGET with $BUILDER"
"$BUILDER" build --locked --release --target "$TARGET" -p else-bot

BINARY="$ROOT/target/$TARGET/release/else-bot"
if [[ ! -x "$BINARY" ]]; then
    echo "expected binary was not produced: $BINARY" >&2
    exit 1
fi

mkdir -p "$DIST_DIR"
rm -f "$ARCHIVE" "$ARCHIVE.sha256"

if [[ "$TARGET" == *apple-darwin ]]; then
    codesign --force --sign - "$BINARY"
fi

tar -czf "$ARCHIVE" -C "$(dirname "$BINARY")" else-bot
archive_name="$(basename "$ARCHIVE")"
if command -v sha256sum >/dev/null 2>&1; then
    ( cd "$DIST_DIR" && sha256sum "$archive_name" > "$archive_name.sha256" )
else
    ( cd "$DIST_DIR" && shasum -a 256 "$archive_name" > "$archive_name.sha256" )
fi

echo "$ARCHIVE"
