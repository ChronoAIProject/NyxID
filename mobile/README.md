# NyxID Mobile

The native workspace for the [NyxID](../) auth/SSO platform, built around persisted assistant conversations, service connections, agents, and approvals. It uses the existing NyxID backend and retains the mobile QR login, push notification, and secure session flows.

The first Web-to-RN increment and remaining migration work are documented in [RN Web parity scope](docs/RN_WEB_PARITY.md).

## What it does

- **Chats with NyxBot and specialists.** Open saved conversations, receive streaming responses, review scoped confirmations, stop work, and recover history after a connection interruption.
- **Connects services.** Browse the catalog, connect supported API-key, token-exchange, OAuth, or platform-key services, and manage existing connections under the server's permissions.
- **Manages agents.** Create personal specialists, edit authorized profiles and grants, manage memory, and open agent conversations.
- **Reviews approvals.** Receive push notifications, scan login QR codes, inspect the server's tool arguments and expiry, approve or deny requests, and revoke granted permissions.
- **Customizes chat appearance.** Settings → Display offers Gray, Lavender, and Graphite light-mode bubbles with a live preview. The choice is saved on the device and applies to individual, specialist, and group chats, including messages being sent. User bubbles share three 18pt corners and a 4pt bottom-right corner.
- **Manages account security.** Edit profile information, revoke sessions, set up TOTP, and sign in with a password and authenticator code.
- **Stays signed in securely.** Tokens persist in OS-secure storage through `expo-secure-store`; refresh and account changes fence stale responses and queued credential writes.

- **Preserves MFA assurance.**

  - Password MFA always requires a TOTP or single-use recovery code. Server social challenges last five minutes and stay in memory until verification.
  - `auth:mobile-mfa` defaults off and resolves for the person, including live org overrides, at social sign-in. While off, a social first-factor holder can sign in without an MFA code.
  - Legacy and mobile-exempt sessions keep working until expiry; enabling the flag preserves existing exemptions.
  - Device/app identity and Agent Key approvals, and login-code minting, require verified authority. Error 2003 preserves the session.
  - With the flag enabled in `/users/me.capabilities.enabled_features`, the app offers authenticator/recovery step-up and retries after verification. Otherwise it directs the user to the NyxID website to verify and approve or create a login code. Missing old-backend capabilities means off. No automatic approvals.

## Tech stack

| | |
| --- | --- |
| Framework | Expo SDK 55 + React Native 0.83 |
| Language | TypeScript (strict, `noUncheckedIndexedAccess`) |
| State | TanStack Query + React Context |
| Storage | `expo-secure-store` (tokens), AsyncStorage (preferences) |
| Push | `expo-notifications` (APNs on iOS, FCM on Android) |
| Telemetry | PostHog (opt-in, configurable per profile) |
| Build | Direct `xcodebuild` / `gradle` (no EAS) |
| Submit | `xcrun altool` / Google Play Developer API |

## Quick start

```bash
cd mobile
pnpm install
cp .env.example .env.prod         # or .env.dev, or both
# fill in the values (see "Environment" below)
pnpm start                        # Metro for local dev
```

Native run for local development:

```bash
pnpm ios                          # APP_ENV=dev
pnpm android                      # APP_ENV=dev
```

Android local debug (emulator/device, API URLs): see [docs/ANDROID_DEBUG.md](docs/ANDROID_DEBUG.md).

App Store review checklist: [docs/APP_STORE_REVIEW_CHECKLIST.md](docs/APP_STORE_REVIEW_CHECKLIST.md).

## Environment

`.env.example` is the source of truth for what config the build expects. Copy it to `.env.dev`, `.env.prod`, or both. Values in `.env.local` override either profile (for machine-specific tweaks).

### Per-profile vars (`DEV_*` / `PROD_*`)

Each field falls back to the other profile if its primary is empty. Both empty for the canonical signal (`*_API_BASE_URL`) aborts the build.

| Variable | Required | Purpose |
| --- | --- | --- |
| `*_API_BASE_URL` | yes (one of dev/prod) | Backend URL — canonical "profile populated" signal |
| `*_FRONTEND_URL` | for web QR login / waitlist | Trusted frontend origin that creates login QR codes |
| `*_IOS_BUNDLE_ID` | yes (one of dev/prod) | iOS bundle identifier (reverse-DNS) |
| `*_ANDROID_PACKAGE` | yes (one of dev/prod) | Android application id |
| `*_APPLE_ASC_APP_ID` | only for submit | Numeric ASC App ID |
| `*_IOS_BUILD_NUMBER` | for release builds | `CFBundleVersion` — bump per release |
| `*_ANDROID_VERSION_CODE` | for release builds | Android `versionCode` — bump per release |
| `*_UNIVERSAL_LINK_HOST` | optional | iOS `associatedDomains` + Android intent filter |
| `*_UNIVERSAL_LINK_PATH_PREFIX` | optional | Android intent-filter path |
| `*_LEGAL_BASE_URL` | for Privacy / Terms screens | Origin serving `/legal/{privacy,terms}.md` — same files the web dashboard renders. Empty = legal screens show an error. |
| `*_ALLOWED_EMAILS` | optional | Comma-separated allowlist; empty = allow all signed-in users |
| `*_TELEMETRY_DSN` / `*_TELEMETRY_HOST` / `*_SHARE_ANALYTICS` | optional | PostHog |

### Production domain migration

Production builds embed the existing `PROD_API_BASE_URL`, `PROD_FRONTEND_URL` and `PROD_LEGAL_BASE_URL` plus an optional second set, `PROD_NEW_API_BASE_URL`, `PROD_NEW_FRONTEND_URL` and `PROD_NEW_LEGAL_BASE_URL` (all three HTTPS URLs or none). The NyxID admin feature flag `mobile:new-domains` (global only, default off) selects the set at every launch: before the app renders, it asks both API hosts' `/public/config` and applies the first valid answer, falling back to the last saved answer, then the original domains, within 5 seconds. Switching the flag needs no rebuild; users restart or reload the app.

QR login, iOS associated domains and React Navigation accept both embedded sets. The existing `PROD_UNIVERSAL_LINK_HOST` and `PROD_UNIVERSAL_LINK_PATH_PREFIX` stay unchanged. App identifiers, signing and version numbers keep using the existing variables. See [Domain migration](docs/DOMAIN_MIGRATION.md) for the selection rules, backend requirements, rollout and rollback.

The active API URL lives in `src/lib/domains.ts`; HTTP requests, token refresh and social login use `buildApiUrl(path)` from `src/lib/api/baseUrl.ts`. The resolver trims surrounding whitespace and trailing slashes and supports the legacy `/mobile` suffix. Missing configuration raises an error before an API request.

### App identity (single value, defaults if unset)

| Variable | Default | |
| --- | --- | --- |
| `APP_NAME` | `NyxID Mobile` | Home-screen display name |
| `APP_SLUG` | `nyxid-mobile` | Expo project slug |
| `APP_SCHEME` | `nyxid` | Custom URL scheme for deep links |
| `APP_VERSION` | `1.0.4` | Marketing version (`CFBundleShortVersionString`) |

### Apple submit credentials (account-wide)

```
APPLE_ID                             your Apple Developer account email
APPLE_TEAM_ID                        10 chars uppercase+digits
ASC_API_KEY_ID                       10-char key ID from ASC Integrations
ASC_API_KEY_ISSUER_ID                UUID, same screen
mobile/credentials/asc-api-key.p8    the .p8 file Apple gave you (one-time download)
```

### Android signing + submit credentials

```
ANDROID_KEYSTORE_PATH                default: ./credentials/release.keystore
ANDROID_KEYSTORE_PASSWORD
ANDROID_KEY_ALIAS
ANDROID_KEY_PASSWORD

mobile/credentials/release.keystore           your release signing keystore (generate once with keytool)
mobile/credentials/play-service-account.json  Play Console → API access → service account → JSON download
```

## Build & deploy

| Command | What it does |
| --- | --- |
| `pnpm build:ios` / `pnpm build:android` | Build a release `.ipa` / `.aab` for the **prod** profile |
| `pnpm build:ios:dev` / `pnpm build:android:dev` | Same but for the **dev** profile |
| `pnpm build:prod` | Both platforms (iOS then Android), prod |
| `pnpm build:dev` | Both platforms, dev |
| `pnpm submit:ios` | Upload the most recent `.ipa` to App Store Connect → TestFlight |
| `pnpm submit:android` | Upload the most recent `.aab` to Play Console → Internal testing track |
| `pnpm submit:prod` | Both submits in sequence |
| `pnpm release:ios` | `build:ios && submit:ios` |
| `pnpm release:android` | `build:android && submit:android` |
| `pnpm release:prod` | Full release for both platforms |
| `pnpm bump:ios` / `pnpm bump:android` / `pnpm bump:both` | Increment `PROD_*_BUILD_NUMBER` / `_VERSION_CODE` in `.env.prod` |
| `pnpm clean:ios` / `pnpm clean:android` | Wipe build state for one platform |
| `pnpm clean:caches` | Wipe `.expo/` and this project's Xcode DerivedData |
| `pnpm clean` | All clean targets (not `node_modules`) |
| `pnpm clean:full` | All clean targets including `node_modules` |

iOS uploads land in **TestFlight** (ASC routes all uploads here first — automatic). Android uploads land in **Internal testing** (TestFlight equivalent: up to 100 testers, no Play review). Production publication for either is a manual web-UI step from there — this pipeline never auto-publishes.

> **macOS required for iOS builds.** Xcode + CocoaPods are needed locally.

### Build flow (iOS)

```
pnpm build:ios
└─ APP_ENV=prod node scripts/build-ios.js
   1. expo prebuild --platform ios      # regenerates ios/ from app.config.ts
   2. pod install                        # also runs React Native codegen → ios/build/generated/
   3. xcodebuild archive                 # automatic signing, DEVELOPMENT_TEAM=$APPLE_TEAM_ID
   4. xcodebuild -exportArchive          # produces ios/build/*.ipa
```

### Build flow (Android)

```
pnpm build:android
└─ APP_ENV=prod node scripts/build-android.js
   1. expo prebuild --platform android --clean
   2. patch-android-build-gradle.js      # force androidx.core 1.15.0
   3. ./gradlew bundleRelease            # signing via -Pandroid.injected.signing.*
                                         # → android/app/build/outputs/bundle/release/app-release.aab
```

### Submit flow (iOS)

```
pnpm submit:ios
└─ APP_ENV=prod node scripts/submit-ios.js
   1. Stage mobile/credentials/asc-api-key.p8 → ~/.appstoreconnect/private_keys/AuthKey_<KEY_ID>.p8
   2. xcrun altool --upload-app --apiKey <KEY_ID> --apiIssuer <ISSUER_ID> --file <latest .ipa>
```

### Submit flow (Android)

```
pnpm submit:android
└─ APP_ENV=prod node scripts/submit-android.js
   Play Developer API v3 (via googleapis):
   1. edits.insert
   2. edits.bundles.upload                # the .aab
   3. edits.tracks.update                  # track="internal"
   4. edits.commit
```

## One-time setup for each contributor

These are the gates between "git cloned" and "shipped a build". Plan on ~30 min total for first run.

1. **Apple Developer account** ($99/yr): https://developer.apple.com → enroll.
2. **Google Play Console account** ($25 one-time): https://play.google.com/console.
3. **App Store Connect API key.** ASC → Users and Access → Integrations → App Store Connect API → `+`. Pick role **App Manager** (or **Developer** at minimum for upload-only). Download the `.p8` **immediately** — Apple only lets you download it once. Save as `mobile/credentials/asc-api-key.p8`. Record Key ID + Issuer ID into `.env.prod`.
4. **Play service account.** Play Console → Setup → API access → "Create new service account" (Google Cloud flow). Grant **Release manager** role on Play Console, with access to the relevant app. Download the JSON key, save as `mobile/credentials/play-service-account.json`.
5. **Android release keystore.** Generate once with:
   ```bash
   keytool -genkeypair -v -storetype PKCS12 \
     -keystore mobile/credentials/release.keystore \
     -alias nyxid -keyalg RSA -keysize 2048 -validity 10000
   ```
   Set `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD` in `.env.prod`.

   > **CRITICAL — back this file + passwords up.** If you lose them, Google Play will never accept updates to your app again. You'd have to publish a brand-new listing with a new package id.

6. **Sign Xcode into your Apple Developer account.** Open Xcode → Settings → Accounts → `+` → sign in with the email matching your `APPLE_ID`. This is what lets `xcodebuild` use `CODE_SIGN_STYLE=Automatic` to auto-manage iOS certs and provisioning profiles.
7. **Fill `mobile/.env.prod`** by copying from `.env.example` and replacing every placeholder.

## Version bumping

`*_IOS_BUILD_NUMBER` and `*_ANDROID_VERSION_CODE` live in `.env.{dev,prod}`. Both Apple and Google reject builds with a version code less than or equal to the last accepted one. **Always bump before a `release:*` command.**

```bash
pnpm bump:ios        # PROD_IOS_BUILD_NUMBER → N+1
pnpm bump:android    # PROD_ANDROID_VERSION_CODE → N+1
pnpm bump:both       # both
```

The repository matches NyxID's current TestFlight build on `fun.chrono-ai.nyxid`: **1.0.4 (36)**. For a new upload, keep `APP_VERSION=1.0.4` and set a higher `PROD_IOS_BUILD_NUMBER` in your local `.env.prod`; `pnpm bump:ios` advances an existing build number of 36 to 37. Later uploads must continue to increase the build number.

## Troubleshooting / cleaning

The build scripts only remove old artifacts (`.ipa`, `.xcarchive`) per run and let `pod install` + Gradle handle incremental rebuilds. When that's not enough:

| Symptom | Run |
| --- | --- |
| Builds intermittently failing after dep updates | `pnpm clean:ios && pnpm build:ios` |
| Strange Xcode error with no obvious cause | `pnpm clean:caches && pnpm build:ios` (wipes `~/Library/Developer/Xcode/DerivedData/NyxIDMobile-*`) |
| "I don't know what state I'm in anymore" | `pnpm clean:full && pnpm install && pnpm build:ios` (full reset, ~5 min reinstall) |
| Android: `Could not find tools.jar` | `export JAVA_HOME=$(/usr/libexec/java_home -v17)` |
| iOS: signing prompt mid-build | Confirm Xcode → Settings → Accounts has your Apple ID and the team matches `APPLE_TEAM_ID` |
| TestFlight rejects with "build number not greater" | `pnpm bump:ios` then rebuild |
| Play rejects with "versionCode used" | `pnpm bump:android` then rebuild |
| `altool` complains about missing `.p8` | Confirm `mobile/credentials/asc-api-key.p8` exists, then re-run `pnpm submit:ios` |

## What ships in git vs. what stays local

| In git | Gitignored |
| --- | --- |
| `app.config.ts` (Expo config, env-driven) | `.env`, `.env.dev`, `.env.prod`, `.env.local` |
| `scripts/lib/load-env.js` | `credentials/asc-api-key.p8` |
| `scripts/build-{ios,android}.js` | `credentials/release.keystore` |
| `scripts/submit-{ios,android}.js` | `credentials/play-service-account.json` |
| `scripts/bump-version.js`, `scripts/clean.js` | `android/` (regenerated each build) |
| `scripts/patch-android-build-gradle.js` | `ios/build/`, `ios/Pods/`, `ios/.xcode.env.local` |
| `google-services.json` (Firebase) | |
| `.env.example` | |

## Legal documents (Privacy + Terms)

Privacy Policy and Terms of Service share the markdown sources in `frontend/public/legal/privacy.md` and `frontend/public/legal/terms.md`. The frontend build renders static `/privacy` and `/terms` HTML from those files; the mobile app reads the markdown directly.

Mobile fetches them at runtime from `${LEGAL_BASE_URL}/legal/{privacy,terms}.md` and renders with `react-native-markdown-display`, styled to match the mobile theme. The web build uses `react-markdown` + `remark-gfm` to compile the pages; nginx caches the HTML for 24 hours and revalidates unchanged pages by content hash afterward.

To update the text:

1. Edit `frontend/public/legal/privacy.md` or `frontend/public/legal/terms.md`.
2. Update the `effective_date` in the YAML frontmatter.
3. Redeploy the frontend.

No mobile rebuild is required. The web pages can remain in a browser cache for up to 24 hours after a frontend deploy; mobile fetches the source on the next screen open.

OSS forks: set `*_LEGAL_BASE_URL` in your `.env.*` to your own deploy origin (or a different host hosting the same `/legal/*.md` files).

## Architecture

```
mobile/
├─ app.config.ts                  # Expo config — resolves env via shared loader
├─ index.ts                       # RN entry point
├─ assets/                        # Icons, splash, notification icon
├─ google-services.json           # Firebase config (Android)
│
├─ scripts/                       # Build / submit / clean orchestrators (all in JS)
│  ├─ lib/load-env.js             # env loader + DEV↔PROD per-field fallback
│  ├─ build-ios.js                # prebuild → pod install → xcodebuild archive + export
│  ├─ build-android.js            # prebuild --clean → patch gradle → gradlew bundleRelease
│  ├─ submit-ios.js               # xcrun altool → ASC TestFlight
│  ├─ submit-android.js           # googleapis → Play Internal testing
│  ├─ bump-version.js             # increments build numbers in .env.*
│  ├─ clean.js                    # explicit cache wipes for edge cases
│  └─ patch-android-build-gradle.js  # forces androidx.core 1.15.0
│
├─ src/
│  ├─ app/                        # Navigator, root, deep-link routing
│  ├─ features/
│  │  ├─ auth/                    # Login, auth session context, MFA verify
│  │  ├─ assistant/               # Persisted chats, native SSE, scoped confirmations
│  │  ├─ services/                # Catalog and service connections
│  │  ├─ agents/                  # Specialist profiles, grants, memory, lifecycle
│  │  ├─ activity/                # Challenge list + detail
│  │  ├─ approvals/               # Granted permissions, revoke flow
│  │  ├─ account/                 # Profile, sessions, TOTP, settings, account deletion
│  │  └─ legal/                   # Privacy, terms
│  ├─ components/                 # Reusable UI primitives and workspace drawer
│  ├─ hooks/                      # useNetworkStatus, useNyxChat, etc.
│  ├─ lib/
│  │  ├─ api/                     # HTTP client (mobileApi.ts, http.ts), error mapping
│  │  ├─ auth/                    # SecureStore session persistence, JWT parsing
│  │  ├─ notifications/           # APNs + FCM registration, deep-link payloads
│  │  ├─ telemetry.ts             # PostHog init (opt-in)
│  │  └─ env.ts                   # `IS_DEV_BUILD`, `ALLOWED_EMAILS` from process.env
│  └─ theme/                      # Design tokens, mobile-specific theme
│
├─ ios/                           # Generated by expo prebuild; committed (manual customizations OK)
└─ android/                       # Generated by expo prebuild on every Android build (gitignored)
```

## Backend endpoints this app talks to

All under `/api/v1` on the configured `*_API_BASE_URL`. The workspace API families and supported operations are listed in [RN Web parity scope](docs/RN_WEB_PARITY.md); the retained mobile auth and approval paths include:

| Endpoint | Used by |
| --- | --- |
| `POST /auth/login` | Email + password login |
| `POST /auth/mfa/verify` | MFA second-factor verification |
| `POST /auth/mfa/step-up` | Upgrade the current live session before approving another sign-in |
| `POST /auth/device/preview` | Preview a login request's requester context |
| `POST /auth/device/approve` | Approve a device login with the human session |
| `POST /auth/device/deny` | Reject a device login with the human session |
| `GET /approvals/requests?status=pending` | Pending challenges list |
| `GET /approvals/requests/{id}` | Challenge detail |
| `POST /approvals/requests/{id}/decide` | Approve / deny a challenge |
| `GET /approvals/grants` | Granted permissions list |
| `DELETE /approvals/grants/{id}` | Revoke a grant |
| `POST /notifications/devices` | Register APNs / FCM push token |
| `DELETE /users/me` | Account deletion (Apple HIG requirement) |

## Deep links & push

- Custom URL scheme: `{APP_SCHEME}://challenge/{challenge_id}` → opens the challenge detail screen
- Device login scheme: `nyxid://login/device?user_code=...` -> prefills the confirmation screen without previewing or approving
- The QR scanner accepts HTTPS or `nyxid://` login URLs but extracts only `user_code`; API calls always use `*_API_BASE_URL`
- Supported push payload fields: `deeplink`, `url`, `challenge_id`, `challengeId`
- Universal Links: when `*_UNIVERSAL_LINK_HOST` is set, that host is added to iOS `associatedDomains` and Android's intent filter. The host of `*_API_BASE_URL` is also auto-added to iOS `associatedDomains` so backend-issued links open in the app.
- React Navigation accepts the configured app scheme and every embedded API/frontend origin plus the link host; the universal-link settings are the same for both domain sets (see [Domain migration](docs/DOMAIN_MIGRATION.md)).

## Session

- Access + refresh tokens persist in `SecureStore` (iOS Keychain / Android EncryptedSharedPreferences).
- Cold start restores the session and routes to `Assistant` if valid, `Auth` otherwise.
- 401 after refresh failure triggers a full sign-out (state + storage + push deregistration).

## Key source files

- `app.config.ts` — Expo config; per-field DEV↔PROD fallback resolved via `scripts/lib/load-env.js`
- `src/lib/api/baseUrl.ts` — single runtime API base resolver and endpoint URL builder; reads `EXPO_PUBLIC_API_BASE_URL` once
- `src/lib/api/http.ts` + `src/lib/api/mobileApi.ts` — HTTP client and API operations; use the shared URL builder
- `src/features/auth/AuthSessionContext.tsx` — auth state, token refresh, telemetry init
- `src/lib/auth/sessionStore.ts` — SecureStore wrapper
- `src/app/linking.ts` — deep link → screen routing
- `src/lib/notifications/pushNotifications.ts` — APNs / FCM registration + payload handling

### MFA rollout notes

No new environment variables are required. Enable `auth:mobile-mfa` globally,
for org cohorts or per person after compatible app builds are distributed and
all API replicas are upgraded. Existing legacy/exempt sessions keep their normal
expiry; step-up never extends it. Sid-less legacy refreshes acquire a fixed
30-day session on first refresh. Old-replica in-flight device/login codes and
login approvals lacking assurance fail closed; restart those requests. New
server challenges remain authoritative for sign-in UI even before capabilities
are available. Account/request changes discard pending step-up retries.

Skipped mobile MFA records metadata-only `mfa: skipped_mobile_rollout` in a
`social_login` audit for redirect sign-in or a `social_token_exchange` audit for
native exchange. Both paths emit `auth.mobile_mfa_exempted` through the existing
telemetry consent gates.

#1970 is unreleased and is not included in v0.73.0. Production replicas at
v0.73.0 or earlier never enforced session assurance. During a mixed upgrade,
sessions those replicas issue (including MFA-less web social sign-ins) lack
assurance and upgraded replicas treat them as existing devices. Rolling back
to v0.73.0 removes social MFA enforcement. Only environments that ran unreleased
#1970 builds have rejecting replicas to drain. Deploy the web step-up UI only
after all API replicas support the route.

### Native drawer and chat display

Signed-in management uses one retained pull-up drawer over the current chat.
Nested routes have Back and Close; dirty drafts and critical writes guard Close,
backdrop, drag, accessibility Escape and Android Back. The handle is separate
from page scrolling. Reduced motion avoids sliding/spring travel. The full
[route inventory](docs/NATIVE_WORKSPACE_ROUTES.md) includes Services, agent
creation/details/skills, shared legal/device/auth entries and cold-link fallbacks.

Settings → Display applies device-local chat layout across personal, specialist,
contextual and group chats. Composer maximum defaults to 120pt and adapts above
the keyboard; it does not add blank space. Wide windows expose chat column width
(default maximum 758pt). Phones omit that ineffective control. Reset affects chat
layout only. Measured transcript, composer and horizontal-strip fades indicate
remaining content without intercepting taps or replacing scroll-follow logic.

Native validation includes mobile tests, TypeScript, native dependency sync and
iOS/Android JavaScript exports. Simulator/emulator execution covers development
runtimes; physical-device and release-build verification remains separate.
RN-web, native transcript rail and machines remain outside the supported native
capabilities.

The chat toolbar offers Camera, Photos and Files under "+" through the native
pickers. Images convert to JPEG when needed; shared upload limits and lifecycle
checks apply. Eligible private/specialist/NyxBot chats use server dictation,
matching web: discover dictation options, automatically choose the available
default, stream PCM24k to the private conversation's dictation WebSocket, then
review and explicitly Insert text or Discard. Capture is ephemeral and never
sends a message. Microphone denial offers Settings. Tap starts/stops; holding
the microphone records until release. Native has no OS recognizer fallback or
source selector. Crew dictation is unsupported, matching web; the keyboard's
own microphone remains available.

A separate waveform action opens conversational voice for eligible private chats
when the server enables `assistant:voice`. It automatically selects the available
server default and uses the existing voice session/stream/control contract.
Real GPT transcription and spoken responses require an authenticated real NyxID
backend with provider access, flags and credits. Synthetic fixture transcripts
and contract tests establish lifecycle behavior only.
