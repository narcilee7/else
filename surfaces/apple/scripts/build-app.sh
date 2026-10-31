#!/bin/bash

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO_ROOT="$(cd "$ROOT/../.." && pwd)"
CONFIGURATION="${1:-debug}"
APP_DIR="$ROOT/.build/Else.app"
HELPERS_DIR="$APP_DIR/Contents/Helpers"
LAUNCH_AGENTS_DIR="$APP_DIR/Contents/Library/LaunchAgents"
SIGN_IDENTITY="${CODESIGN_IDENTITY:--}"

cd "$ROOT"
swift build -c "$CONFIGURATION"
PRODUCT_DIR="$(swift build -c "$CONFIGURATION" --show-bin-path)"

if [[ "$CONFIGURATION" == "release" ]]; then
    cargo build \
        --release \
        -p else-daemon \
        --manifest-path "$REPO_ROOT/Cargo.toml"
    DAEMON_BINARY="$REPO_ROOT/target/release/else-daemon"
else
    cargo build \
        -p else-daemon \
        --manifest-path "$REPO_ROOT/Cargo.toml"
    DAEMON_BINARY="$REPO_ROOT/target/debug/else-daemon"
fi

rm -rf "$APP_DIR"
mkdir -p \
    "$APP_DIR/Contents/MacOS" \
    "$APP_DIR/Contents/Resources" \
    "$HELPERS_DIR" \
    "$LAUNCH_AGENTS_DIR"
cp "$ROOT/Resources/Info.plist" "$APP_DIR/Contents/Info.plist"
cp "$PRODUCT_DIR/ElseMac" "$APP_DIR/Contents/MacOS/ElseMac"
install -m 755 "$DAEMON_BINARY" "$HELPERS_DIR/else-daemon"
install -m 644 \
    "$ROOT/Resources/LaunchAgents/com.else.desktop.runtime.plist" \
    "$LAUNCH_AGENTS_DIR/com.else.desktop.runtime.plist"
install -m 644 \
    "$ROOT/Resources/LaunchAgents/com.else.desktop.notifier.plist" \
    "$LAUNCH_AGENTS_DIR/com.else.desktop.notifier.plist"
install -m 755 "$PRODUCT_DIR/ElseNotifier" "$APP_DIR/Contents/MacOS/ElseNotifier"

if [[ "$SIGN_IDENTITY" == "-" ]]; then
    codesign --force --sign - "$HELPERS_DIR/else-daemon"
    codesign --force --sign - "$APP_DIR/Contents/MacOS/ElseNotifier"
    codesign --force --sign - "$APP_DIR"
else
    codesign \
        --force \
        --options runtime \
        --timestamp \
        --sign "$SIGN_IDENTITY" \
        "$HELPERS_DIR/else-daemon"
    codesign \
        --force \
        --options runtime \
        --timestamp \
        --sign "$SIGN_IDENTITY" \
        "$APP_DIR/Contents/MacOS/ElseNotifier"
    codesign \
        --force \
        --options runtime \
        --timestamp \
        --sign "$SIGN_IDENTITY" \
        "$APP_DIR"
fi

codesign --verify --deep --strict "$APP_DIR"
echo "$APP_DIR"
