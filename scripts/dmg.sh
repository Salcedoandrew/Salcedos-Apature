#!/usr/bin/env bash
# Build a universal (Apple silicon + Intel) Apature.app and a drag-to-
# Applications disk image with create-dmg (brew install create-dmg).
# Output: target/dist/Apature.app and
#         target/dist/Apature-<version>-test-<build>-universal.dmg
# target/bundle (the local app from scripts/bundle.sh) is left alone.
#
#   scripts/dmg.sh
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
BUILD=$(git rev-list --count HEAD 2>/dev/null || echo 0)
DIST=target/dist
APP="$DIST/Apature.app"
DMG="$DIST/Apature-${VERSION}-test-${BUILD}-universal.dmg"

command -v create-dmg >/dev/null || { echo "create-dmg missing: brew install create-dmg"; exit 1; }
rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null

for t in aarch64-apple-darwin x86_64-apple-darwin; do
  APATURE_BUILD="$BUILD" cargo build --release -p apature --target "$t"
done

mkdir -p "$DIST"
lipo -create \
  target/aarch64-apple-darwin/release/apature \
  target/x86_64-apple-darwin/release/apature \
  -output "$DIST/apature-universal"
lipo -info "$DIST/apature-universal"

rm -rf "$APP"
APP="$APP" APATURE_BINARY="$DIST/apature-universal" scripts/bundle.sh
codesign -v --strict "$APP"

STAGE="$DIST/dmg-stage"
rm -rf "$STAGE" "$DMG"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/Apature.app"
create-dmg \
  --volname "Apature $VERSION ($BUILD)" \
  --volicon apps/desktop/assets/Apature.icns \
  --window-pos 200 120 --window-size 560 340 --icon-size 100 \
  --icon "Apature.app" 140 160 --hide-extension "Apature.app" \
  --app-drop-link 420 160 \
  --no-internet-enable \
  "$DMG" "$STAGE"
rm -rf "$STAGE"
echo "disk image: $DMG ($(du -h "$DMG" | cut -f1))"
