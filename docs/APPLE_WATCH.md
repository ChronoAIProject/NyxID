# NyxID for Apple Watch

A native SwiftUI watchOS app shipped inside the NyxID iPhone app, built around
three things: notifications, agents and chats.

## Experience

Three vertical pages (Digital Crown or swipe). Each page header has the gear
(Settings) on the left and a white mic on the right that starts a new chat.

| Page | Shows | Tap |
| --- | --- | --- |
| **Inbox** | One amber card for the approvals waiting ("2 approvals · Gmail · Calendar"), then the NyxBot feed (agent avatar, title, "agent · age", unread dot) | The card opens the approval queue; a feed entry opens a short preview and "Open chat" |
| **Agents** | NyxBot and specialists (avatar, name, one-line tagline) | New chat with that agent |
| **Chats** | Recent threads (avatar, title, age) | Continue that thread |

- **New chat**: an agent pill (tap to switch agents), a big mic ("Tap to
  speak"), one suggestion chip ("Plan my day" for NyxBot, "What's new?" for a
  specialist) and a keyboard button.
- **Working**: the question, three dots and one status line taken from the
  turn's tool activity ("Checking your calendar…"), then "You can lower your
  wrist. We'll tap you when it's ready." and Stop. If the agent is waiting on an
  approval, the line becomes "Needs your approval" with "Review ›"; anything
  else that needs a decision says "Decide on iPhone".
- **Reply**: the first sentence bold, the rest of the lead smaller, then up to
  six list rows with times in their own column ("10:00 · Design review"). The
  rest is behind "More ›". The bottom bar has speaker (read aloud), mic (reply)
  and phone (continue in the iPhone or web app through Handoff).
- **Approvals**: one card at a time ("1 OF 2"): what is asked, who
  asks, "Read-only" / "Makes changes" and how long it lasts. Swipe right or tap
  Allow, swipe left or tap Deny; the result screen offers "Next approval ›".
- **Settings**: "Read replies aloud" (only with headphones), the account name
  and Sign out.
- Detail screens put back and the screen's name (agent avatar and name, the
  agent pill, "1 OF 2") in the top bar beside the clock. watchOS drops the
  clock when that content is too wide, so it is capped at the screen width
  minus about 117pt and long names truncate (`TopBar` in `App/Theme.swift`).
- **Setup**: while signed out the watch shows "Connect to iPhone" and a
  six-digit code; see [Sign-in](#sign-in).
- Agents look the same as on the iPhone and web: NyxBot wears the NyxID mark,
  other agents get initials in a muted colour derived from their id
  (`Core/AgentAvatarModel.swift` mirrors `mobile/src/components/agentAvatarModel.ts`).
- Colour is earned (`DESIGN.md`): white for the main action, amber only for
  approvals, purple only for unread dots and the working dots.
- Screen and spoken text stay short; server error prose never reaches the
  watch ("Offline", "Already handled", "Busy. Try again soon").
- When a reply arrives the watch taps; if the app isn't on screen it posts a
  local notification with the brief reply. Replies are read aloud automatically
  only when headphones are connected.
- The feed needs `assistant:notifications`. Without it the Inbox shows
  approvals only.

Phase 1 (this document) uses only plain HTTPS. A live voice call is a possible
later phase; see [Not in phase 1](#not-in-phase-1).

## Code layout

| Path | What |
| --- | --- |
| `mobile/ios/NyxIDWatch/Core/` | Foundation-only core: session storage, API client, wire models, reply text. Compiled into the watch target and testable with `swift test` (macOS or Linux). |
| `mobile/ios/NyxIDWatch/App/` | SwiftUI screens, WatchConnectivity receiver, notification actions, speech. |
| `mobile/ios/NyxIDWatch/CoreTests/` | Unit tests against a scripted HTTP transport. |
| `mobile/modules/nyx-watch/` | Local Expo module: the iPhone side of WatchConnectivity. |
| `mobile/src/features/account/AppleWatchSettingsScreen.tsx`, `watchPairing.ts` | Settings › Apple Watch on the iPhone. |
| `mobile/plugins/withWatchCompanion.js` | Prebuild keeps the watch target's bundle ID and version in step with the iPhone app. |

The watch target (`NyxIDWatch`, watchOS 10+) lives in the committed
`NyxIDMobile.xcodeproj` and is embedded in the iPhone app ("Embed Watch
Content"). Its bundle ID is the iPhone app's plus `.watchkitapp`. The iPhone
target must stay the project's first target: Expo prebuild rewrites only that
one, and the plugin then rewrites the watch target. Never regenerate `ios/` with
`expo prebuild --clean`; the plugin fails if the watch target is missing.

## Sign-in

The watch never receives or shares the iPhone's session. A refresh token
rotates on every use and replaying a rotated one revokes the whole session, so
two devices can't share one.

1. While signed out, the watch shows a random six-digit code and shares it with
   the iPhone as its application context (`{v, kind:"hello", code}`).
2. Settings › Apple Watch on the iPhone shows that code ("Code on your watch").
   The person checks it matches the watch and taps Connect, which mints a
   one-time account login code (`POST /auth/login-code` with
   `{"auth_kind":"account_session"}`; first-party human only; MFA step-up
   applies as for any sign-in on another device).
3. The iPhone sends only `{v, kind:"pair", code, api_base_url, match,
   frontend_url, expires_at}` over WatchConnectivity, where `match` is the
   watch's code: `sendMessage` when the watch app is reachable, otherwise it
   leaves the message as the application context, which the watch app receives
   when it next runs. Only the newest context is kept, and the iPhone overwrites
   it with a blank context once setup ends. (`transferUserInfo` isn't used: it
   isn't delivered between simulators, and a queue of single-use codes has no
   value.) The watch accepts a pair message only while signed out and only when
   `match` equals the code on its screen, so a stale or stray message never
   signs it in or disturbs a working sign-in. `api_base_url` must be HTTPS
   (HTTP only for loopback).
4. The watch redeems the code (`POST /auth/login-code/redeem`) for its own
   access and refresh tokens, labelled "Apple Watch", and stores them in the
   watch keychain (`WhenUnlockedThisDeviceOnly`).
5. The iPhone follows the code's status until it is `redeemed`. If the person
   leaves the screen first, the unused code is cancelled.

The watch refreshes one request at a time with `POST /auth/refresh` and signs
out when the server refuses the refresh token. Signing out (on the watch, or
"Sign out of Apple Watch" on the iPhone) calls `POST /auth/logout` with the
watch's own token, which ends only the watch session. The session is also listed
under Account & security › Sessions and can be revoked there.

The agent chat routes accept only first-party human account sessions and refuse
API keys, so the watch session is a full account session, like the iPhone's.

## Approvals and notification actions

- `GET /approvals/requests?status=pending&include_admin_orgs=true`, then
  `POST /approvals/requests/{id}/decide` with an `Idempotency-Key`. A
  grant-mode approval lasts the person's configured grant period
  (`grant_expiry_days`), as on the iPhone.
- Execution-permission proposals (`execution_proposal` present) need the full
  review page and are never decided from the watch.
- iPhone alerts appear on the watch when the iPhone is locked. The backend tags
  **only decidable approval requests** with the `APPROVAL_REQUEST` category
  (`push_service::apns_alert_category`); the watch registers Approve and Deny
  for it. Both actions open the watch app, which makes the decision with its own
  session and shows the result. Approve requires an unlocked watch.
- After signing in, the watch asks once for notification permission (when not
  already decided) so NyxID alerts also show while the watch app is open.

## Talking to agents

- `GET /assistant/nyxagent/agents` (NyxBot first; only `can_use` agents that
  aren't destroyed).
- A new chat goes to the agent's home thread, else its newest thread, else a
  new thread started by the first message. Chats and feed entries continue
  their own thread.
- Input is the system dictation sheet (`TextFieldLink`); the speech-recognition
  API isn't available on watchOS.
- `POST /assistant/nyxagent/turns` returns a server-sent event stream. The
  watch reads only the response headers (`x-nyxid-conversation-id`) and drops
  the connection; the turn keeps running on the server.
- It then polls `GET /assistant/nyxagent/conversations/{id}` every two seconds
  for up to ten minutes. The status line comes from `active_turn.activities`
  (`Core/ToolActivityText.swift` mirrors the iPhone's `describeToolActivity`).
  The reply is the newest assistant message after the person's message once
  `active_turn` is null. Stop is `POST /assistant/nyxagent/conversations/{id}/stop`.
- `ReplyLayout` (`Core/WatchPresentation.swift`) shapes the reply for the
  screen; `SpokenText.brief` gives the spoken form: the first paragraph, cut at
  a sentence or word past 240 characters, with code and links named, not read
  out.
- The Chats page uses `GET /assistant/nyxagent/conversations` (newest first,
  threads of destroyed agents hidden).

## Not in phase 1

- **Live voice calls.** watchOS allows WebSockets only while streaming audio or
  during a CallKit call (Apple TN3135), and there is no WebRTC for watchOS. The
  Grok voice relay (hold-to-talk PCM over NyxID's WebSocket) is the path a watch
  could use, presented as a CallKit call.
- **Watch-registered push.** The watch relies on iPhone alerts appearing on it;
  it doesn't register its own APNs token.
- **Translations.** The watch app is English only; the iPhone's Apple Watch
  screen is translated.
- **Complications.**
- **Wear OS.**

## Testing

- `cd mobile/ios/NyxIDWatch && swift test` (also runs in the Mobile CI job).
- `pnpm test` covers the iPhone pairing flow, the settings row and the plugin.
- Build the watch target alone:
  `xcodebuild -project mobile/ios/NyxIDMobile.xcodeproj -target NyxIDWatch -sdk watchsimulator`.
- Test networking on a real watch: the simulator always allows WebSockets and
  other low-level networking, which watchOS restricts on device.

## Release notes

- Register the `<bundle id>.watchkitapp` App ID; automatic signing with
  `-allowProvisioningUpdates` creates it during archive.
- App Store Connect needs watch screenshots and the watch app icon (the iPhone
  icon is reused).
