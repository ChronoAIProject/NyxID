# NyxID Mobile

The mobile authenticator companion for the [NyxID](../) auth/SSO platform. Receives push notifications when a sign-in or sensitive action needs your approval, lets you approve or deny it with one tap, and persists the granted permissions for any policies you've opted into.

Think Duo Mobile, Microsoft Authenticator, or Authy — but bound to your own self-hosted NyxID backend.

## What it does

- **Approves auth challenges.** When a service requests access (login, OAuth grant, MFA step-up, admin action), the backend pushes a notification to this app. You see the request details — who, what, when, where — and tap approve or deny.
- **Manages granted permissions.** View every grant you've issued (per-service, per-scope), and revoke any of them.
- **Acts as an MFA factor.** When MFA is enabled on a NyxID account, this app's approval is the second factor.
- **Stays signed in securely.** Tokens persist in the OS secure enclave (iOS Keychain via `expo-secure-store`, Android EncryptedSharedPreferences).

For end users this is a companion app to whatever website / service uses NyxID as its identity provider. For NyxID operators this is what your tenants install.

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

### App identity (single value, defaults if unset)

| Variable | Default | |
| --- | --- | --- |
| `APP_NAME` | `NyxID Mobile` | Home-screen display name |
| `APP_SLUG` | `nyxid-mobile` | Expo project slug |
| `APP_SCHEME` | `nyxid` | Custom URL scheme for deep links |
| `APP_VERSION` | `1.0.1` | Marketing version (`CFBundleShortVersionString`) |

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

For NyxID's existing TestFlight history on `fun.chrono-ai.nyxid`, the last accepted iOS build was **34**. The first build under this pipeline should be `PROD_IOS_BUILD_NUMBER=35`.

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

Privacy Policy and Terms of Service are **markdown files served by the frontend** (`frontend/public/legal/privacy.md`, `frontend/public/legal/terms.md`). Both the web dashboard and the mobile app render the same source — edit once, both surfaces update on next visit (web) / next launch (mobile).

Mobile fetches them at runtime from `${LEGAL_BASE_URL}/legal/{privacy,terms}.md` and renders with `react-native-markdown-display`, styled to match the mobile theme. Web fetches from the same `/legal/*.md` path and renders with `react-markdown` + `remark-gfm`.

To update the text:

1. Edit `frontend/public/legal/privacy.md` or `frontend/public/legal/terms.md`.
2. Update the `effective_date` in the YAML frontmatter.
3. Redeploy the frontend.

No mobile rebuild required — the next time someone opens the screen, they get the new text.

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
│  │  ├─ activity/                # Challenge list + detail (the main "what to approve" screen)
│  │  ├─ approvals/               # Granted permissions, revoke flow
│  │  ├─ account/                 # Profile, settings, account deletion
│  │  └─ legal/                   # Privacy, terms
│  ├─ components/                 # Reusable UI primitives
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

All under `/api/v1` on the configured `*_API_BASE_URL`:

| Endpoint | Used by |
| --- | --- |
| `POST /auth/login` | Email + password login |
| `POST /auth/mfa/verify` | MFA second-factor verification |
| `GET /approvals/requests?status=pending` | Pending challenges list |
| `GET /approvals/requests/{id}` | Challenge detail |
| `POST /approvals/requests/{id}/decide` | Approve / deny a challenge |
| `GET /approvals/grants` | Granted permissions list |
| `DELETE /approvals/grants/{id}` | Revoke a grant |
| `POST /notifications/devices` | Register APNs / FCM push token |
| `DELETE /users/me` | Account deletion (Apple HIG requirement) |

## Deep links & push

- Custom URL scheme: `{APP_SCHEME}://challenge/{challenge_id}` → opens the challenge detail screen
- Supported push payload fields: `deeplink`, `url`, `challenge_id`, `challengeId`
- Universal Links: when `*_UNIVERSAL_LINK_HOST` is set, that host is added to iOS `associatedDomains` and Android's intent filter. The host of `*_API_BASE_URL` is also auto-added to iOS `associatedDomains` so backend-issued links open in the app.

## Session

- Access + refresh tokens persist in `SecureStore` (iOS Keychain / Android EncryptedSharedPreferences).
- Cold start restores the session and routes to `Dashboard` if valid, `Auth` otherwise.
- 401 after refresh failure triggers a full sign-out (state + storage + push deregistration).

## Key source files

- `app.config.ts` — Expo config; per-field DEV↔PROD fallback resolved via `scripts/lib/load-env.js`
- `src/lib/api/http.ts` + `src/lib/api/mobileApi.ts` — HTTP client; reads `EXPO_PUBLIC_API_BASE_URL`
- `src/features/auth/AuthSessionContext.tsx` — auth state, token refresh, telemetry init
- `src/lib/auth/sessionStore.ts` — SecureStore wrapper
- `src/app/linking.ts` — deep link → screen routing
- `src/lib/notifications/pushNotifications.ts` — APNs / FCM registration + payload handling
