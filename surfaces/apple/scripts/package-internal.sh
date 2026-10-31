#!/bin/bash

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_DIR="$ROOT/.build/Else.app"
DIST_DIR="$ROOT/.build/dist"
VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$ROOT/Resources/Info.plist")"
ARCH="$(uname -m)"

case "$ARCH" in
    arm64 | aarch64) ARCH="arm64" ;;
    x86_64) ARCH="x86_64" ;;
    *)
        echo "unsupported macOS architecture: $ARCH" >&2
        exit 1
        ;;
esac

ARCHIVE="$DIST_DIR/Else-$VERSION-$ARCH-adhoc.zip"

CODESIGN_IDENTITY=- "$ROOT/scripts/build-app.sh" release
codesign --verify --deep --strict "$APP_DIR"

mkdir -p "$DIST_DIR"
rm -f "$ARCHIVE" "$ARCHIVE.sha256"
ditto -c -k --sequesterRsrc --keepParent "$APP_DIR" "$ARCHIVE"
archive_name="$(basename "$ARCHIVE")"
( cd "$DIST_DIR" && shasum -a 256 "$archive_name" > "$archive_name.sha256" )

echo "$ARCHIVE"
echo "Internal artifact only: ad-hoc signed and not notarized."
