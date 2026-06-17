#!/usr/bin/env bash
# Launch Chrome with the DevTools debug port so the NyxID CDP worker can
# attach. Uses a dedicated profile dir so it doesn't disturb your main
# Chrome; log into ChatGPT once in this window and the session persists.
#
# macOS. For Linux use `google-chrome`, for Windows use chrome.exe with the
# same flags.
#
# SECURITY: --remote-debugging-port is an UNAUTHENTICATED control channel. Any
# local process that can reach this port gets full control of this Chrome
# profile (ChatGPT session + cookies). We bind it to localhost only (we do NOT
# pass --remote-debugging-address) and isolate it in a dedicated --user-data-dir.
# Do not widen the bind address, and do not reuse this profile for other
# sensitive logins. On a shared machine prefer --remote-debugging-pipe.
set -euo pipefail

PORT="${CHROME_DEBUG_PORT:-9222}"
PROFILE="${CHROME_PROFILE_DIR:-$HOME/.nyxid-chrome}"

CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
[ -x "$CHROME" ] || CHROME="/Applications/Chromium.app/Contents/MacOS/Chromium"

echo "Launching Chrome on debug port $PORT (profile: $PROFILE)"
echo "→ Log into ChatGPT in the window that opens; the login persists in this profile."
exec "$CHROME" \
  --remote-debugging-port="$PORT" \
  --user-data-dir="$PROFILE" \
  --no-first-run --no-default-browser-check \
  "https://chatgpt.com/"
