#!/usr/bin/env bash
# Copy the research skills into Claude Code's and Codex's user skill folders (an existing skill of the
# same name is replaced, so edits here reach the agents on their next run).
set -euo pipefail
cd "$(dirname "$0")"
for dest in "$HOME/.claude/skills" "$HOME/.codex/skills"; do
  mkdir -p "$dest"
  for s in */; do
    s="${s%/}"
    rm -rf "${dest:?}/$s"
    cp -R "$s" "$dest/$s"
  done
  echo "installed into $dest: $(ls -d */ | tr -d / | tr '\n' ' ')"
done
