#!/bin/sh
# Cargo runner used by `npm run tauri:dev` on macOS. It signs the dev binary with
# a stable identity before running it, so the Keychain's "Always Allow" for
# Dispatched's stored sign-ins and connection strings survives rebuilds (an
# unsigned binary is a new app to macOS after every build). Without a signing
# identity the binary runs unsigned. Override the identity with
# DISPATCHED_DEV_SIGN_IDENTITY (a name or SHA-1 from `security find-identity`).
bin="$1"
shift
id="${DISPATCHED_DEV_SIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null | awk '/Developer ID Application|Apple Development/ { print $2; exit }')}"
if [ -n "$id" ]; then
  codesign --force --sign "$id" --identifier io.horizon-tech.dispatched "$bin" 2>/dev/null ||
    echo "dev-sign: codesign failed, running unsigned" >&2
fi
exec "$bin" "$@"
