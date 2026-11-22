#!/usr/bin/env bash
# (Re)start the voice relay in the background, next to `codly serve` (which proxies /api/voice/ws to it).
#   voice/start.sh                 # port 8766
#   CODLY_VOICE_PORT=9000 voice/start.sh   # then start codly serve with the same CODLY_VOICE_PORT
# The relay writes a token to ~/.config/codly/voice-<port>.token at each start; `codly serve`, run by
# the same user, reads it for each call. Nothing else can open a voice session.
set -euo pipefail
cd "$(dirname "$0")"
. ~/.config/wif/openai.env   # OPENAI_IDENTITY_PROVIDER_ID, OPENAI_SERVICE_ACCOUNT_ID (identifiers, not secrets)

if [[ -f relay.pid ]] && kill -0 "$(cat relay.pid)" 2>/dev/null; then
  kill "$(cat relay.pid)"
  sleep 1
fi

uv sync --frozen -q
CODLY_VOICE_PORT="${CODLY_VOICE_PORT:-8766}" nohup uv run --frozen python relay.py >> relay.log 2>&1 &
echo $! > relay.pid
echo "voice relay pid $(cat relay.pid), port ${CODLY_VOICE_PORT:-8766}, log $(pwd)/relay.log"
