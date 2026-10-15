#!/bin/sh
# `npm run tauri:dev`: `tauri dev`, with the dev binary signed on macOS
# (see dev-sign-run.sh). Elsewhere it is plain `tauri dev`.
here=$(cd "$(dirname "$0")" && pwd)
if [ "$(uname)" = Darwin ]; then
  export CARGO_TARGET_AARCH64_APPLE_DARWIN_RUNNER="$here/dev-sign-run.sh"
  export CARGO_TARGET_X86_64_APPLE_DARWIN_RUNNER="$here/dev-sign-run.sh"
fi
exec npx tauri dev "$@"
