#!/bin/bash
# DeepScan.app's CFBundleExecutable. Starts the local engine + daemon
# (bundled under Contents/Resources by build-dmg.sh) and opens the UI in a
# native window (Contents/Resources/bin/DeepScanWindow, a small Cocoa +
# WKWebView app compiled at build time) — see docs/ARCHITECTURE.md.
set -euo pipefail

RESOURCES="$(cd "$(dirname "${BASH_SOURCE[0]}")/../Resources" && pwd)"
export DEEPSCAN_ENV=production
export DEEPSCAN_MODE=local
# Point directly at the bundled, read-only models instead of copying
# ~275MB into ~/.deepscan on first launch.
export DEEPSCAN_MODEL_DIR="$RESOURCES/models"

fail() {
  osascript -e "display alert \"DeepScan couldn't start\" message \"$1\" as critical"
  exit 1
}

"$RESOURCES/bin/deepscan-engine" > "$HOME/.deepscan-launch.log" 2>&1 &
ENGINE_PID=$!

# The engine writes ~/.deepscan/engine.lock once it's up; the daemon and
# this script both read it rather than assuming a fixed port.
for _ in $(seq 1 30); do
  [ -f "$HOME/.deepscan/engine.lock" ] && break
  # If the engine already exited, waiting for its lockfile is pointless —
  # bail with a real error instead of silently opening a dead browser tab.
  if ! kill -0 "$ENGINE_PID" 2>/dev/null; then
    fail "The DeepScan engine exited immediately on startup. Log: ~/.deepscan-launch.log"
  fi
  sleep 0.5
done

if ! kill -0 "$ENGINE_PID" 2>/dev/null; then
  fail "The DeepScan engine exited on startup. Log: ~/.deepscan-launch.log"
fi

"$RESOURCES/bin/deepscan-daemon" &
DAEMON_PID=$!

# Optional: only start the Tika document-parsing bridge if a JRE is
# actually bundled/available. Document search still works without it —
# ViaTikaThenMiniLm extraction calls will just fail for that file category.
if [ -x "$RESOURCES/jre/bin/java" ] && [ -f "$RESOURCES/lib/deepscan-parser.jar" ]; then
  "$RESOURCES/jre/bin/java" -jar "$RESOURCES/lib/deepscan-parser.jar" &
  PARSER_PID=$!
fi

# DEEPSCAN_ENGINE_HTTP_PORT isn't set here, so this matches config.rs's own
# default (51424) exactly — engine.lock only records the gRPC port, not
# this one, so reading HTTP_PORT from it would grab the wrong value.
export DEEPSCAN_URL="http://127.0.0.1:51424"

cleanup() {
  kill "$ENGINE_PID" "$DAEMON_PID" "${PARSER_PID:-}" 2>/dev/null || true
}
trap cleanup EXIT

# A real native window (Cocoa + WKWebView, compiled by build-dmg.sh) — not
# a browser with its UI stripped down. Closing this window is what the
# user experiences as "quitting DeepScan", so it runs in the foreground;
# quitting it triggers the cleanup trap above, stopping the engine/daemon
# with it rather than leaving them running invisibly.
"$RESOURCES/bin/DeepScanWindow"
