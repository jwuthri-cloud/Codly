#!/bin/bash
# Build Codly.app from this checkout and install it in /Applications (the copy you launch).
#
#   scripts/install-app.sh            development: the release build minus LTO (fast after the first)
#   scripts/install-app.sh --release  a real release: LTO and one codegen unit (Cargo.toml, ~3 min)
#
# Both are real release builds (bundled frontend, the app's CSP, no dev server), unlike
# `tauri build --debug`, which still loads http://localhost:1420.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ "${1:-}" != "--release" ]; then
  # Whole-program optimization is what makes a release build slow; skip it while developing.
  export CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=256 CARGO_PROFILE_RELEASE_INCREMENTAL=true
fi
pnpm tauri build --bundles app

osascript -e 'tell application "Codly" to quit' 2>/dev/null || true
for _ in $(seq 20); do pgrep -f /Applications/Codly.app/Contents/MacOS/codly >/dev/null || break; sleep 0.5; done
rm -rf /Applications/Codly.app
cp -R "${CARGO_TARGET_DIR:-target}/release/bundle/macos/Codly.app" /Applications/
open /Applications/Codly.app
ls -la /Applications/Codly.app/Contents/MacOS/codly
