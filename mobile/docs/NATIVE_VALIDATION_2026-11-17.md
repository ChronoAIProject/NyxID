# Native integration verification — 2026-11-17 (#1868, Workstream 6)

This record continues [NATIVE_VALIDATION_2026-11-14.md](NATIVE_VALIDATION_2026-11-14.md). It covers the Android paths that were previously only unit-tested, the iOS gaps left open there, the defects found and fixed on this branch, and what still needs physical devices or production credentials. Configuration and ownership remain in [NATIVE_INTEGRATION.md](NATIVE_INTEGRATION.md).

## Environment and revisions

- Code under test: branch `feat/mobile-1868-completion`, rebased onto `main` `0aa248d7c`. Device runs used per-fix builds before the rebase; the defect table lists the rebased commit of each fix. The final Android pass ran on the pre-rebase head (`9eae166b0`: the same branch patches on the original base `2b1162ba7`). After the rebase, Android was smoke-tested on `89b946b15` and re-checked on the final head `c604a1ce8`: service OAuth with the explicit Chrome package, force-stop recovery (no stale task after cold start, then a fresh authorization completes), the App Link fallback in a Custom Tab, and terminal connect.
- Android:
  - emulator `ws6-pixel35`: Pixel 7 profile, Android 15 (API 35), `google_apis` arm64 image with Chrome, Google Play services and Play Store, headless;
  - emulator `ws6-pixel30`: Android 11 (API 30), `google_apis` arm64, used for pre-Android-12 App Links;
  - release APKs built by `scripts/build-android.js` (`expo prebuild --clean` + Gradle `bundleRelease`), plus `assembleRelease` for installation;
  - package `fun.chronoai.nyxid`, minSdk 24, targetSdk 36;
  - signed with a throwaway local key (SHA-256 `42:8D:B6:99:6B:97:88:80:B0:2E:E2:B5:7A:EE:5D:38:1F:A8:79:A2:44:56:67:24:BB:AB:E0:D7:3F:19:25:B1`).
- iOS:
  - Xcode 27.2, iOS 27.2 simulator (iPhone 18 Pro, a fresh device for this run);
  - Release configuration with the JavaScript bundle embedded, ad-hoc signing;
  - the local-only UIScene startup patch (not part of the branch).
- Backend:
  - local `nyxid-server` 0.78.0 built from this branch, on MongoDB 8.0;
  - `BASE_URL=https://localhost`, reached through a loopback-only TLS front with a test CA. The CA is trusted as a system CA on the emulator (in memory) and in the simulator keychain. The release app therefore uses HTTPS without any cleartext exception.
  - AASA identity `56DKXVFJS2.fun.chrono-ai.nyxid`; Asset Links identity `fun.chronoai.nyxid` with the throwaway key.
  - The "frontend" host (`https://10.0.2.2` on Android, `https://127.0.0.1` on iOS) is a labelled stand-in page. It proxies `/.well-known` to the backend, as the production frontend nginx does.
- Service OAuth:
  - a local mock OAuth 2.0 provider: an HTTPS consent page with Approve / Deny / Provider error / Approve-but-exchange-fails, and an HTTP token endpoint;
  - registered with `nyxid admin catalog apply` (manifest with `credential_mode: admin`);
  - Chrome on the emulator trusts only the mock's leaf key, via `--ignore-certificate-errors-spki-list`.
- Push:
  - the Android build used a FAKE `google-services.json` (non-existent project, same package), pending the owner's go-ahead to request a real FCM token from `nyxid-fbc03`;
  - Android delivery is SIMULATED: a root broadcast delivers the backend's exact FCM message shape (notification + data, channel `approvals`) into the app's own `FirebaseInstanceIdReceiver`. This exercises the real Firebase and expo-notifications handling, but not FCM transport.
- SSH:
  - an OpenSSH target (Alpine 3.20, `TrustedUserCAKeys` + `AuthorizedPrincipalsFile`, principal `nyx`) reached through a branch-built `nyxid node`;
  - certificate-mode catalog service `ws6-ssh`.

## Results

### Row 1 — Service OAuth (Android 15)

| Case | Result |
| --- | --- |
| New connection → Continue to authorization | The Custom Tab opens the provider page. Approve → NyxID callback → `nyxid://services/oauth-return` → the app comes back by itself, and the connection shows Connected with a fresh "Last authorized". |
| Deny / Provider error / exchange failure | Each returns to the app with "Authorization failed" and Reconnect. The server status is `failed`. |
| Cancel (×) | "Authorization incomplete" with Continue authentication. Never reported as success. |
| Process killed on the provider page (SIGKILL, as the low-memory killer does) → relaunch | "Authorization isn't finished" for the exact connection, with Check again and Continue authorization. Continue opens a fresh authorization in a new task, and it completes (requires fix D2). |
| Force stop on the provider page → relaunch | The stale Custom Tab task is removed on cold start, and Continue opens a fresh authorization that completes (requires fix D2b). |
| Account switch | B sees none of A's pending attempt or connections. Back on A, the connection still shows Pending auth with Continue. |
| Real providers | Not run: no provider credentials (owner question Q-C). |

### Row 2 — Push (Android 15; transport simulated)

| Case | Result |
| --- | --- |
| Sign-in | Android 13+ POST_NOTIFICATIONS prompt → Allow (granted). |
| Foreground | In-app "Approval required · Review" toast → the exact request's Approval detail. |
| Background | System notification on channel `approvals` (importance MAX), with the payload's title and body → tap → exact request. |
| Cold start (process killed, notification tapped) | Opens the exact request. |
| Decide in app | Approve unblocks the waiting `nyxid ssh exec`; Deny returns `approval_failed` to it. |
| Sign-out | Clears the app's notifications from the tray (1 → 0). |
| Token registration, rotation, logout unbind, rebind | Not run: needs a real FCM token (owner question Q-F). With the fake Firebase project, no token exists and the server lists no device (`devices: []`), as expected. |
| Real FCM / APNs delivery | Not run: needs `FCM_SERVICE_ACCOUNT_PATH` / an APNs key (owner question Q-E). |

### Row 3 — Universal Links / App Links

| Case | Result |
| --- | --- |
| Host-set build env | `DEV_/PROD_NATIVE_LINK_HOSTS` (`api` default, or `api+frontend`). With production-like URLs and the opt-in: `api` → iOS `applinks:nyx-api.chrono-ai.fun`, `applinks:api.nyxid.ai`, `applinks:nyxid.onelink.me` (identical to the committed entitlements), and on Android OneLink plus one 21-path autoVerify filter per API host. `api+frontend` → additionally `nyx.chrono-ai.fun` and `nyxid.ai`. The build banner prints the set and the resulting hosts. |
| Opt-in off (default) | The APK declares only the custom-scheme filters; HTTPS links open in the browser. |
| Android 15, host set A + opt-in, `pm set-app-links` | Unverified → browser. Approved → a claimed path opens the app cold (`/activity`) and warm (`/services`). Unclaimed (`/dashboard`) → browser. Denied → browser. |
| In-app web fallback | A claimed path without a native page (`/account/<unknown>`) opens the frontend page in a Custom Tab, path and query preserved, even when the frontend domain is verified for the app (requires fix D6). API-origin URLs map onto the frontend. |
| Android 11 (pre-12) | Unverified → "Open with NyxID Mobile / Chrome" chooser. `always` → app. `never` and unclaimed paths → browser. |
| Real verification | Not possible locally. On Google images (both API levels) Google Play services verifies through Google's Digital Asset Links service, which cannot reach a private host. The log shows the verifier failing both hosts in about 260 ms with no request reaching the local server. Real verification needs `assetlinks.json` published on the production hosts (owner questions Q-A, Q-B). |
| Production state (read-only status check, 2026-11-17) | AASA 200 and assetlinks 404 on `api.nyxid.ai`, `nyxid.ai`, `nyx-api.chrono-ai.fun` and `nyx.chrono-ai.fun`. The frontend nginx proxies `/.well-known` to the backend, so host set A serves Asset Links once `ANDROID_APP_*` is set. |
| iOS OS-level association | See the iOS section. |

### Row 7 — SSH terminal (Android 15)

| Case | Result |
| --- | --- |
| Open terminal | Before fix D1 the app crashed (SIGSEGV); after it, the terminal opens. |
| Approval loop | "Waiting for approval or the node…" → Review approvals → stacked Activity → Approve → Back → the same terminal connects. Denial shows the non-asserting refusal text. |
| PTY and input | `tty` → `/dev/pts/0`. UTF-8 output renders. Typing works. With the soft keyboard up: Tab completion, sticky Ctrl-C (interrupts `sleep 100`), arrows in `less`, Esc in `vim` (requires fix D3). Paste from the system clipboard works. |
| Resize | The PTY is 36×45 with the keyboard hidden and 19×45 with it shown, across repeated toggles. |
| `exit` / server-side close | Disconnected, "Connection closed." and Reconnect, with no network-error banner (requires fix D5). |
| Node offline / host-key mismatch / unknown service | The server message in full; "SSH host key mismatch"; "not connected… Open Services". |
| Deep link | Pre-connect "Connect to ws6-ssh as nyx?" with no auto-connect. |
| Background | Within 15 s the same session is kept. On Android the remote session stays open while the app is backgrounded (JS timers paused; measured 75 s), and returning after 15 s or more closes it immediately (Disconnected). See the D4 decision. |
| Stalled lookup | With the API frozen, "Connecting…" becomes "This SSH connection could not be loaded." with Retry after 20 s, and Retry succeeds once the API is reachable (requires fix D7). |
| 4 MB output flood | The session ends with Disconnected and "Connection closed." The server logs "Dropping web terminal due to full receive buffer". The same flood completes on the iOS simulator through the same TLS front. This is a known server-side limit, documented and tracked separately. |

### iOS delta (iOS 27.2 simulator)

| Case | Result |
| --- | --- |
| Host-key mismatch (wrong pin via `nyxid node ssh cert-host-key pin`) | "SSH host key mismatch" in the status line and the terminal, with Reconnect; audit `ssh_host_key_mismatch`. `forget` → re-TOFU → connects. |
| `exit` | The shell output, then "Connection closed." with Disconnected and Reconnect. |
| 4 MB flood | Completes (`FLOOD-DONE`); the session stays connected. |
| Key bar with the software keyboard | Stays directly above the iOS keyboard (unchanged by D3). |
| "Connecting…" observation | Seen once: on the first terminal deep link after a fresh install and sign-in, the screen stayed on "Connecting…" for over 2 minutes, with no Connect button. Not reproduced in 4 further attempts: Back and re-open; cold start via the link; reinstall with the notification prompt pending; an erased simulator with fresh sign-in, about 3 minutes on Home, then the link. Code inspection showed the lookup had no bound or Retry. D7 bounds it to 20 s with Retry. |
| Final build | PENDING. Since about 03:17 on 2026-11-17, app launch on every iOS 27.2 simulator on the verification machine hangs (`simctl launch` of the app or of Settings, `openurl`, `install`), and the CoS reproduced it on a separate device. The final-head iOS build compiles; the remaining iOS checks are listed in the WS6 handoff. |

## Defects found and fixed on this branch

| ID | Platform | Defect | Fix |
| --- | --- | --- | --- |
| D1 | Android | Opening the SSH terminal crashed the app: the iOS-only string shorthand `dataDetectorTypes="none"` segfaults the Android Fabric props parser. | `f856fd60b`: array form `["none"]`, with a test for both platforms. Also present on `main` since #2498. |
| D2, D2b | Android | After process death or a force stop during service OAuth, expo-web-browser's proxy task survived, and every later auth session resurfaced the stale tab instead of loading the new URL. | `e1cba17c7`, `b652d2570`, hardened in `263fe7657`: a config plugin removes leftover proxy tasks on the first `MainActivity` creation in a new process, matching the task's base intent or base activity. Includes a fixture test. |
| D3 | Android | The terminal key bar was hidden behind the soft keyboard: a second, conflicting keyboard adjustment. | `6cadb6ebc`: Android relies on the shared page-level keyboard viewport; iOS unchanged. |
| D4 | Android | The documented 15 s background close holds only on return on Android. | `f663ac2e1`: documented precisely, with an on-return regression test (decision: keep the behaviour). |
| D5 | Android | A normal `exit` or a server-side close was reported as "Couldn't open the terminal connection". | `9d06f23c4`: after `connected`, errors and closes end the session normally. |
| D6 | Android | In-app browser opens (web fallback, web handoffs) were captured by the app's own verified App Links. A provider domain verified for another app could capture service OAuth. | `bb28dc527` (+ test harness `0d281b5de`, `89b946b15`; fail-safe retry `263fe7657`): an explicit Custom Tabs browser package for every in-app browser open and for service OAuth. |
| D7 | Both | The terminal connection lookup had no bound or retry. | `56a223deb`: a 20 s bound with Retry. |

Also on the branch: `875312e19`, the host-set build env for row 3 (an explicit frontend URL is required for `api+frontend` since `c604a1ce8`).

## Not verified here (needs production credentials, publication or physical devices)

- Real OAuth providers on both platforms: provider application credentials with the NyxID callback registered (Q-C).
- Real FCM token registration, rotation, logout unbind and rebind (Q-F), and real FCM delivery (`FCM_SERVICE_ACCOUNT_PATH`, Q-E).
- Real APNs delivery: an APNs key plus a team-signed build with `aps-environment` (Q-E).
- OS-level App Link verification against production (Asset Links published with the Play App Signing SHA-256, Q-A/Q-B), and iOS Universal Link association on a team-signed build or device.
- Physical iPhone and Android devices: everything above was run on simulators and emulators. Real-device runs also depend on the iOS 27 UIScene launch issue on `main`.
