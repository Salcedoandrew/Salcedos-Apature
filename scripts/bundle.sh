#!/usr/bin/env bash
# Build Apature.app from the release binary. No dependencies beyond Xcode's
# command-line tools (codesign). Output: target/bundle/Apature.app
#
#   scripts/bundle.sh            # build + bundle + ad-hoc sign
#   scripts/bundle.sh --install  # also copy to /Applications (replaces an old copy)
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
BUILD=$(git rev-list --count HEAD 2>/dev/null || echo 0)
# APP and APATURE_BINARY can be overridden (scripts/dmg.sh bundles a
# universal binary into target/dist without touching target/bundle).
APP="${APP:-target/bundle/Apature.app}"
BINARY="${APATURE_BINARY:-}"

if [[ -z "$BINARY" ]]; then
  APATURE_BUILD="$BUILD" cargo build --release -p apature
  BINARY=target/release/apature
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BINARY" "$APP/Contents/MacOS/Apature"
cp apps/desktop/assets/Apature.icns "$APP/Contents/Resources/Apature.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>               <string>Apature</string>
  <key>CFBundleDisplayName</key>        <string>Apature</string>
  <key>CFBundleIdentifier</key>         <string>dev.mccal.apature</string>
  <key>CFBundleVersion</key>            <string>${BUILD}</string>
  <key>CFBundleShortVersionString</key> <string>${VERSION}</string>
  <key>CFBundleExecutable</key>         <string>Apature</string>
  <key>CFBundleIconFile</key>           <string>Apature</string>
  <key>CFBundlePackageType</key>        <string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key> <string>6.0</string>
  <key>LSMinimumSystemVersion</key>     <string>12.0</string>
  <key>LSApplicationCategoryType</key>  <string>public.app-category.photography</string>
  <key>NSHighResolutionCapable</key>    <true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key> <true/>
  <key>NSHumanReadableCopyright</key>   <string>© 2026 Caleb McCartney. MIT OR Apache-2.0.</string>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key> <string>Folder</string>
      <key>CFBundleTypeRole</key> <string>Viewer</string>
      <key>LSItemContentTypes</key> <array><string>public.folder</string></array>
      <key>LSHandlerRank</key> <string>Alternate</string>
    </dict>
  </array>
</dict>
</plist>
PLIST
printf 'APPL????' > "$APP/Contents/PkgInfo"

# Signing. With a Developer ID Application certificate in the Keychain the
# bundle is signed for distribution (hardened runtime, entitlements); without
# one it is ad-hoc signed for local use. Set SIGN_IDENTITY to force one.
IDENTITY="${SIGN_IDENTITY:-}"
if [[ -z "$IDENTITY" ]]; then
  IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null | grep -m1 -o '"Developer ID Application: [^"]*"' | tr -d '"' || true)"
fi
if [[ -n "$IDENTITY" ]]; then
  codesign --force --deep --options runtime --timestamp \
    --entitlements apps/desktop/assets/entitlements.plist \
    --sign "$IDENTITY" "$APP"
  echo "signed with $IDENTITY"
else
  codesign --force --deep --sign - "$APP" >/dev/null
  echo "ad-hoc signed (no Developer ID Application certificate found)"
fi
echo "built $APP (version $VERSION, build $BUILD)"

# --dmg: a drag-to-Applications disk image; --notarize additionally submits
# it to Apple with the keychain profile "apature" (xcrun notarytool
# store-credentials apature) and staples the ticket. --install copies the
# app to /Applications (local use).
if [[ "${1:-}" == "--dmg" || "${2:-}" == "--dmg" || "${1:-}" == "--notarize" || "${2:-}" == "--notarize" ]]; then
  DMG="target/bundle/Apature-${VERSION}.dmg"
  STAGE=target/bundle/dmg-stage
  rm -rf "$STAGE" "$DMG"; mkdir -p "$STAGE"
  cp -R "$APP" "$STAGE/Apature.app"
  ln -s /Applications "$STAGE/Applications"
  hdiutil create -quiet -volname "Apature $VERSION" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
  rm -rf "$STAGE"
  if [[ -n "$IDENTITY" ]]; then codesign --force --timestamp --sign "$IDENTITY" "$DMG"; fi
  echo "disk image: $DMG ($(du -h "$DMG" | cut -f1))"
  if [[ "${1:-}" == "--notarize" || "${2:-}" == "--notarize" ]]; then
    if [[ -z "$IDENTITY" ]]; then echo "notarize needs a Developer ID signature"; exit 1; fi
    xcrun notarytool submit "$DMG" --keychain-profile apature --wait
    xcrun stapler staple "$DMG"
    xcrun stapler staple "$APP"
    echo "notarized and stapled"
  fi
fi

if [[ "${1:-}" == "--install" ]]; then
  rm -rf /Applications/Apature.app
  cp -R "$APP" /Applications/Apature.app
  echo "installed /Applications/Apature.app"
fi
