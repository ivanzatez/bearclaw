#!/usr/bin/env bash
# Run bearclaw-web in the foreground for development.
# Usage: bin/dev.sh [port]
#   Uses the shared bearclaw config ($BEARCLAW_CONFIG or ~/.config/bearclaw/config.json);
#   without one it runs as a single local host on 127.0.0.1.
#   Env: BEARCLAW_CONFIG, BEARCLAW_WEB_BIND (default here: 127.0.0.1), BEARCLAW_WEB_UI, BEARCLAW_BIN.
set -euo pipefail

WEB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="${1:-${BEARCLAW_WEB_PORT:-7799}}"
BIND="${BEARCLAW_WEB_BIND:-127.0.0.1}"

echo "bearclaw-web dev  config=${BEARCLAW_CONFIG:-${XDG_CONFIG_HOME:-$HOME/.config}/bearclaw/config.json}  port=${PORT}"
echo "  http://${BIND}:${PORT}/"
exec env BEARCLAW_WEB_PORT="$PORT" BEARCLAW_WEB_BIND="$BIND" node "$WEB_DIR/server.mjs"
