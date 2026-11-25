#!/usr/bin/env bash
# Build the Rust staticlib, compile the SwiftUI host, and package a signed .app.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
DIST="${1:-$ROOT/dist}"
APP="$DIST/Azeroth Survivor Reborn Trainer.app"
BIN_NAME="Azeroth Survivor Reborn Trainer"
SDK="$(xcrun --sdk macosx --show-sdk-path)"
MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
export MACOSX_DEPLOYMENT_TARGET

cd "$ROOT"
cargo build --release --locked --target-dir "$TARGET_DIR"
cargo build --release --locked --bin backend-smoke --target-dir "$TARGET_DIR"

VERSION="$(cargo metadata --locked --no-deps --format-version 1 | python3 -c 'import json,sys; print([p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="reborn-trainer"][0])')"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$ROOT/macos/.build"

SOURCES=(
  "$ROOT/macos/Trainer/Sources/TrainerApp.swift"
  "$ROOT/macos/Trainer/Sources/ContentViews.swift"
  "$ROOT/macos/Trainer/Sources/TrainerStore.swift"
  "$ROOT/macos/Trainer/Sources/RustBridge.swift"
  "$ROOT/macos/Trainer/Sources/Models.swift"
)

swiftc "${SOURCES[@]}" \
  -parse-as-library \
  -O \
  -target "arm64-apple-macosx${MACOSX_DEPLOYMENT_TARGET}" \
  -sdk "$SDK" \
  -import-objc-header "$ROOT/macos/Trainer/Sources/BridgingHeader.h" \
  -I "$ROOT/include" \
  -L "$TARGET_DIR/release" \
  -lreborn_trainer \
  -framework SwiftUI \
  -framework AppKit \
  -framework Foundation \
  -framework Combine \
  -o "$APP/Contents/MacOS/$BIN_NAME"

cp "$ROOT/macos/Trainer/Resources/Info.plist" "$APP/Contents/Info.plist"
plutil -replace CFBundleShortVersionString -string "$VERSION" "$APP/Contents/Info.plist"
plutil -replace CFBundleVersion -string "$VERSION" "$APP/Contents/Info.plist"

ICONSET="$ROOT/macos/.build/Trainer.iconset"
rm -rf "$ICONSET"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$ROOT/assets/trainer-logo.png" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$ROOT/assets/trainer-logo.png" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Trainer.icns"

codesign --force --sign - --entitlements "$ROOT/assets/macos/debugger.entitlements" "$APP"
codesign --verify --strict "$APP"

ZIP="$DIST/Azeroth.Survivor.Reborn.Trainer.macos-arm64.zip"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
echo "Built $APP"
echo "Zip $ZIP"
echo "Smoke binary: $TARGET_DIR/release/backend-smoke"
