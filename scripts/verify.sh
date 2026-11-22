#!/bin/bash
# Every check CI runs, in the same order (.github/workflows/ci.yml runs this script):
#
#   scripts/verify.sh
#
# Needs Rust, Node, pnpm and git; on Linux also the system packages Tauri builds with (the
# workflow lists them). The first run builds the whole Rust workspace and takes a while.
set -euo pipefail
cd "$(dirname "$0")/.."

step() {
  printf '\n== %s\n' "$*"
  "$@"
}

# The state of the TypeScript bindings (src/lib/gen), which `cargo test` writes (ts-rs).
bindings() {
  git status --porcelain -- src/lib/gen
  git diff -- src/lib/gen
}

step pnpm install --frozen-lockfile
step pnpm check
step pnpm test
step pnpm build
before=$(bindings)
step cargo test --workspace --locked
if [ "$(bindings)" != "$before" ]; then
  git status --short -- src/lib/gen >&2
  echo "verify: the tests rewrote src/lib/gen, so the committed bindings were out of date. Commit the new files." >&2
  exit 1
fi
printf '\nverify: all checks passed\n'
