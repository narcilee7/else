#!/bin/bash

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_DIR="$ROOT/.build/Else.app"
DIST_DIR="$ROOT/.build/dist"
VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$ROOT/Resources/Info.plist")"
ARCHIVE="$DIST_DIR/Else-$VERSION.zip"

if [[ -z "${CODESIGN_IDENTITY:-}" || "${CODESIGN_IDENTITY}" == "-" ]]; then
    echo "A non-ad-hoc Developer ID identity is required for a release package." >&2
    exit 1
fi

if [[ -z "${APPLE_ID:-}" || -z "${APPLE_APP_PASSWORD:-}" || -z "${APPLE_TEAM_ID:-}" ]]; then
    echo "APPLE_ID, APPLE_APP_PASSWORD and APPLE_TEAM_ID are required for notarization." >&2
    exit 1
fi

"$ROOT/scripts/build-app.sh" release
codesign --verify --deep --strict "$APP_DIR"
mkdir -p "$DIST_DIR"
rm -f "$ARCHIVE"
ditto -c -k --sequesterRsrc --keepParent "$APP_DIR" "$ARCHIVE"

xcrun notarytool submit "$ARCHIVE" \
    --apple-id "$APPLE_ID" \
    --password "$APPLE_APP_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" \
    --wait
xcrun stapler staple "$APP_DIR"
codesign --verify --deep --strict "$APP_DIR"
rm -f "$ARCHIVE"
ditto -c -k --sequesterRsrc --keepParent "$APP_DIR" "$ARCHIVE"

archive_name="$(basename "$ARCHIVE")"
( cd "$DIST_DIR" && shasum -a 256 "$archive_name" > "$archive_name.sha256" )
echo "$ARCHIVE"
