#!/usr/bin/env bash
# Launch Chrome with the DevTools debug port so the NyxID CDP worker can
# attach. Uses a dedicated profile dir so it doesn't disturb your main
# Chrome; log into ChatGPT once in this window and the session persists.
#
# macOS. For Linux use `google-chrome`, for Windows use chrome.exe with the
# same flags.
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
