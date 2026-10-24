#!/usr/bin/env bash
# Builds, signs and (with --upload) uploads the Mac App Store build of Dispatched.
#
#   scripts/appstore.sh <build-number> [--upload]
#
# Needs, on this Mac:
#   - the "Apple Distribution" and "3rd Party Mac Developer Installer" (Mac Installer Distribution)
#     certificates of team N9H2M3339M in the login keychain;
#   - the Mac App Store provisioning profile at appstore/Dispatched_Mac_App_Store.provisionprofile;
#   - for --upload: APPSTORE_API_KEY_ID and APPSTORE_API_ISSUER, with the key at
#     ~/.appstoreconnect/private_keys/AuthKey_<id>.p8 (where altool looks for it).
# The build number must grow with every upload of the same version (App Store Connect refuses a repeat).
set -euo pipefail

BUILD="${1:?usage: scripts/appstore.sh <build-number> [--upload]}"
UPLOAD="${2:-}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_DIR="$ROOT/app"
OUT="$ROOT/appstore"
TEAM="N9H2M3339M"
APP_IDENTITY="${APPSTORE_APP_IDENTITY:-Apple Distribution: \"Horizon Tech\" d.o.o. Sarajevo ($TEAM)}"
INSTALLER_IDENTITY="${APPSTORE_INSTALLER_IDENTITY:-3rd Party Mac Developer Installer: \"Horizon Tech\" d.o.o. Sarajevo ($TEAM)}"

[ -f "$OUT/Dispatched_Mac_App_Store.provisionprofile" ] \
  || { echo "missing $OUT/Dispatched_Mac_App_Store.provisionprofile" >&2; exit 1; }

VERSION="$(node -p "require('$APP_DIR/src-tauri/tauri.conf.json').version")"
# The store overlay plus this build's number; `tauri build --config` takes a JSON merge patch.
CONFIG="$(node -e '
  const c = require(process.argv[1]);
  c.bundle.macOS.bundleVersion = process.argv[2];
  console.log(JSON.stringify(c));
' "$APP_DIR/src-tauri/tauri.appstore.conf.json" "$BUILD")"

cd "$APP_DIR"
APPLE_SIGNING_IDENTITY="$APP_IDENTITY" npx tauri build \
  --bundles app \
  --target universal-apple-darwin \
  --no-default-features --features custom-protocol \
  --config "$CONFIG"

APP="$ROOT/target/universal-apple-darwin/release/bundle/macos/Dispatched.app"
PKG="$OUT/Dispatched_${VERSION}_${BUILD}.pkg"

# Tauri signs the app with the store entitlements; refuse to package an app without the sandbox.
codesign -d --entitlements - --xml "$APP" 2>/dev/null | grep -q "com.apple.security.app-sandbox" \
  || { echo "the app is not sandboxed; refusing to package it" >&2; exit 1; }

xcrun productbuild --sign "$INSTALLER_IDENTITY" --component "$APP" /Applications "$PKG"
echo "built $PKG"

if [ "$UPLOAD" = "--upload" ]; then
  : "${APPSTORE_API_KEY_ID:?set APPSTORE_API_KEY_ID}" "${APPSTORE_API_ISSUER:?set APPSTORE_API_ISSUER}"
  xcrun altool --validate-app --type macos --file "$PKG" --apiKey "$APPSTORE_API_KEY_ID" --apiIssuer "$APPSTORE_API_ISSUER"
  xcrun altool --upload-app --type macos --file "$PKG" --apiKey "$APPSTORE_API_KEY_ID" --apiIssuer "$APPSTORE_API_ISSUER"
fi
