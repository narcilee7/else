#!/bin/bash

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIGURATION="${1:-debug}"
APP_DIR="$ROOT/.build/Else.app"

cd "$ROOT"
swift build -c "$CONFIGURATION"
PRODUCT_DIR="$(swift build -c "$CONFIGURATION" --show-bin-path)"

rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
cp "$ROOT/Resources/Info.plist" "$APP_DIR/Contents/Info.plist"
cp "$PRODUCT_DIR/ElseMac" "$APP_DIR/Contents/MacOS/ElseMac"
codesign --force --sign - "$APP_DIR"

echo "$APP_DIR"
