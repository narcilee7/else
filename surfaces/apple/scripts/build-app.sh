#!/bin/bash

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIGURATION="${1:-debug}"
APP_DIR="$ROOT/.build/Else.app"

cd "$ROOT"
swift build -c "$CONFIGURATION"
PRODUCT_DIR="$(swift build -c "$CONFIGURATION" --show-bin-path)"
SIGN_IDENTITY="${CODESIGN_IDENTITY:--}"

rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
cp "$ROOT/Resources/Info.plist" "$APP_DIR/Contents/Info.plist"
cp "$PRODUCT_DIR/ElseMac" "$APP_DIR/Contents/MacOS/ElseMac"
if [[ "$SIGN_IDENTITY" == "-" ]]; then
    codesign --force --sign - "$APP_DIR"
else
    codesign \
        --force \
        --options runtime \
        --timestamp \
        --sign "$SIGN_IDENTITY" \
        "$APP_DIR"
fi

echo "$APP_DIR"
