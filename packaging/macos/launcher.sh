#!/bin/bash
# DeepScan.app's CFBundleExecutable. Starts the local engine + daemon
# (bundled under Contents/Resources by build-dmg.sh) and opens the UI in
# the user's default browser — see docs/ARCHITECTURE.md.
set -euo pipefail

RESOURCES="$(cd "$(dirname "${BASH_SOURCE[0]}")/../Resources" && pwd)"
export DEEPSCAN_ENV=production
export DEEPSCAN_MODE=local

# The engine writes ~/.deepscan/engine.lock once it's up; the daemon and
# this script both read it rather than assuming a fixed port.
"$RESOURCES/bin/deepscan-engine" &
ENGINE_PID=$!

# Give the engine a moment to bind and publish engine.lock before the
# daemon and parser bridge try to connect to it.
for _ in $(seq 1 30); do
  [ -f "$HOME/.deepscan/engine.lock" ] && break
  sleep 0.5
done

"$RESOURCES/bin/deepscan-daemon" &
DAEMON_PID=$!

# Optional: only start the Tika document-parsing bridge if a JRE is
# actually bundled/available. Document search still works without it —
# ViaTikaThenMiniLm extraction calls will just fail for that file category.
if [ -x "$RESOURCES/jre/bin/java" ] && [ -f "$RESOURCES/lib/deepscan-parser.jar" ]; then
  "$RESOURCES/jre/bin/java" -jar "$RESOURCES/lib/deepscan-parser.jar" &
  PARSER_PID=$!
fi

HTTP_PORT="$(python3 -c "import json;print(json.load(open('$HOME/.deepscan/engine.lock'))['port'])" 2>/dev/null || echo 51424)"
open "http://127.0.0.1:${HTTP_PORT}"

cleanup() {
  kill "$ENGINE_PID" "$DAEMON_PID" "${PARSER_PID:-}" 2>/dev/null || true
}
trap cleanup EXIT

wait "$ENGINE_PID"
