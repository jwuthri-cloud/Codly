#!/bin/bash
# Build this checkout and restart `codly serve` on the Linux devbox (run it there, from the repo).
#
#   scripts/deploy-devbox.sh               fast build, then restart codly serve in tmux session `codly`
#   scripts/deploy-devbox.sh --no-restart  build only
#   scripts/deploy-devbox.sh --yes         restart even when agent processes are alive
#   scripts/deploy-devbox.sh --release     the real release profile (LTO, slow), then restart
#
# The fast build is the release build without LTO, with incremental compilation, and with Codly's
# own two crates at opt-level 1. Dependencies keep their full optimization and their cache, so a
# change in the core recompiles two crates lightly instead of optimizing them for minutes.
# The package overrides go through `--config`: Cargo has no environment variable for them.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="/usr/lib/rust-1.97/bin:$PATH"

restart=1 force=0 fast=1
for a in "$@"; do
  case "$a" in
    --no-restart) restart=0 ;;
    --yes) force=1 ;;
    --release) fast=0 ;;
    *) echo "unknown option $a" >&2; exit 2 ;;
  esac
done

cargo_args=()
if [ "$fast" = 1 ]; then
  export CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=256 CARGO_PROFILE_RELEASE_INCREMENTAL=true
  cargo_args=(-- --config 'profile.release.package.codly-core.opt-level=1' --config 'profile.release.package.codly.opt-level=1')
fi
start=$(date +%s)
pnpm tauri build --no-bundle "${cargo_args[@]}"
echo "built in $(( $(date +%s) - start )) s"
[ "$restart" = 1 ] || exit 0

# A restart ends every agent process of the running server, mid-turn or not. They cannot be told
# apart from idle ones here, so list them and let the person decide.
serve=$(pgrep -f 'target/release/codly serve' | while read -r p; do [ "$(ps -o comm= -p "$p")" = codly ] && echo "$p"; done | head -1)
if [ -n "$serve" ] && [ "$force" = 0 ]; then
  agents=$(ps -o pid=,etime=,args= --ppid "$serve" | grep -E 'claude|codex|opencode|gemini' | cut -c1-110 || true)
  if [ -n "$agents" ]; then
    echo "codly serve has agent processes; a restart ends them (a running turn is lost):"
    echo "$agents"
    echo "Run again with --yes to restart anyway. The new binary is built and waits."
    exit 3
  fi
fi

log="$HOME/.codly-serve.log"
tmux respawn-pane -k -t codly -c "$PWD" "xvfb-run -a dbus-run-session -- target/release/codly serve --host 127.0.0.1 --port 4280 2>&1 | tee -a $log"
sleep 6
grep 'Codly is running at' "$log" | tail -1
