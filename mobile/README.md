# NyxID Mobile

NyxID mobile client (React Native + Expo). Env-driven, no EAS dependency. You bring your own Apple Developer account, Google Play Console account, and an Android keystore — that's it.

## Quick start

```bash
cd mobile
pnpm install
cp .env.example .env.prod                # or .env.dev, or both
# fill in values
pnpm start                               # Metro for local dev (APP_ENV=dev)
```

Native run for local development:

```bash
pnpm ios                                 # APP_ENV=dev
pnpm android                             # APP_ENV=dev
```

Android local debug (emulator/device, API URLs): see [docs/ANDROID_DEBUG.md](docs/ANDROID_DEBUG.md).

## Environment

`.env.example` is the source of truth. Copy to `.env.dev`, `.env.prod`, or both. Every per-env value is resolved per-field with bidirectional fallback (active profile's value first, the other profile's value as fallback).

### Per-profile env vars (`DEV_*` / `PROD_*`)

| Variable | Required | Purpose |
| --- | --- | --- |
| `*_API_BASE_URL` | yes (one of dev/prod) | Backend URL; canonical "profile populated" signal |
| `*_IOS_BUNDLE_ID` | yes (one of dev/prod) | iOS bundle identifier (reverse-DNS) |
| `*_ANDROID_PACKAGE` | yes (one of dev/prod) | Android application id |
| `*_APPLE_ASC_APP_ID` | only for submit | Numeric ASC App ID |
| `*_IOS_BUILD_NUMBER` | for release builds | `CFBundleVersion`; bump per release |
| `*_ANDROID_VERSION_CODE` | for release builds | Android `versionCode`; bump per release |
| `*_UNIVERSAL_LINK_HOST` | optional | iOS associatedDomains + Android intent filter |
| `*_UNIVERSAL_LINK_PATH_PREFIX` | optional | Android intent-filter path |
| `*_ALLOWED_EMAILS` | optional | Comma-separated; empty = allow all |
| `*_TELEMETRY_DSN/HOST/SHARE_ANALYTICS` | optional | PostHog |

### App identity (single value)

`APP_NAME`, `APP_SLUG`, `APP_SCHEME`, `APP_VERSION` — all default to NyxID values if unset.

### iOS credentials

```
APPLE_ID, APPLE_TEAM_ID, ASC_API_KEY_ID, ASC_API_KEY_ISSUER_ID
mobile/credentials/asc-api-key.p8
```

### Android credentials

```
ANDROID_KEYSTORE_PATH, ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS, ANDROID_KEY_PASSWORD
mobile/credentials/release.keystore
mobile/credentials/play-service-account.json
```

## Build & deploy

| Command | What it does |
| --- | --- |
| `pnpm build:ios` / `pnpm build:android` | Build a release `.ipa` / `.aab` for the **prod** profile |
| `pnpm build:ios:dev` / `pnpm build:android:dev` | Same but for the **dev** profile |
| `pnpm build:prod` | Both platforms (iOS then Android), prod |
| `pnpm build:dev` | Both platforms, dev |
| `pnpm submit:ios` | Upload most recent `.ipa` to App Store Connect → TestFlight |
| `pnpm submit:android` | Upload most recent `.aab` to Play Console → Internal testing |
| `pnpm submit:prod` | Both submits in sequence |
| `pnpm release:ios` | `build:ios && submit:ios` |
| `pnpm release:android` | `build:android && submit:android` |
| `pnpm release:prod` | Full release for both platforms |
| `pnpm bump:ios` | Increment `PROD_IOS_BUILD_NUMBER` in `.env.prod` |
| `pnpm bump:android` | Increment `PROD_ANDROID_VERSION_CODE` in `.env.prod` |
| `pnpm bump:both` | Both |

iOS uploads land in **TestFlight** (automatic via ASC). Android uploads land in **Internal testing** (TestFlight equivalent — up to 100 testers, no Play review). Production publication for either is a manual web-UI step from there.

### Build flow (iOS)

```
pnpm build:ios
└─ APP_ENV=prod node scripts/build-ios.js
   1. expo prebuild --platform ios     # regenerates ios/ from app.config.ts
   2. pod install
   3. xcodebuild archive               # automatic signing, DEVELOPMENT_TEAM=$APPLE_TEAM_ID
   4. xcodebuild -exportArchive        # produces ios/build/*.ipa
```

### Build flow (Android)

```
pnpm build:android
└─ APP_ENV=prod node scripts/build-android.js
   1. expo prebuild --platform android --clean
   2. patch-android-build-gradle.js     # forces androidx.core 1.15.0
   3. ./gradlew bundleRelease           # signs via -Pandroid.injected.signing.*
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
   Uses googleapis (Play Developer API v3):
   1. edits.insert
   2. edits.bundles.upload     # the .aab
   3. edits.tracks.update      # track="internal"
   4. edits.commit
```

> **macOS required for iOS builds.** Xcode + CocoaPods are needed locally.

## One-time setup for each contributor

1. **Apple Developer account** ($99/yr): https://developer.apple.com → enroll.
2. **Google Play Console account** ($25 one-time): https://play.google.com/console.
3. **App Store Connect API key** — ASC → Users and Access → Integrations → App Store Connect API → `+`. Download the `.p8` (one-time only). Save as `mobile/credentials/asc-api-key.p8`. Record the Key ID + Issuer ID into `.env.prod`.
4. **Play service account** — Play Console → Setup → API access → create service account, grant "Release manager" role. Download JSON. Save as `mobile/credentials/play-service-account.json`.
5. **Android keystore** — generate once:
   ```bash
   keytool -genkeypair -v -storetype PKCS12 \
     -keystore mobile/credentials/release.keystore \
     -alias nyxid -keyalg RSA -keysize 2048 -validity 10000
   ```
   Set `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD` in `.env.prod`.

   > **CRITICAL: back this file + password up.** If you lose them, Google Play will never accept updates to your app again — you'd have to publish a brand-new listing.

6. **Xcode signed in with your Apple Developer account.** Open Xcode → Settings → Accounts → `+` → sign in. This lets `xcodebuild` auto-manage iOS provisioning profiles for `CODE_SIGN_STYLE=Automatic`.

7. Fill in `mobile/.env.prod` (copy from `.env.example`).

## Version bumping

`*_IOS_BUILD_NUMBER` and `*_ANDROID_VERSION_CODE` are in `.env.{dev,prod}`. Bump per release using `pnpm bump:ios` / `pnpm bump:android`, or edit the file directly. Both Apple and Google reject builds with a versionCode ≤ the last accepted, so always bump before a `release:*` command.

For the existing TestFlight history on `fun.chrono-ai.nyxid`, the last accepted iOS build was 34. Start `PROD_IOS_BUILD_NUMBER=35` for the first build under this pipeline.

## What ships in git vs. what stays local

| In git | Gitignored |
| --- | --- |
| `app.config.ts` | `.env`, `.env.dev`, `.env.prod`, `.env.local` |
| `scripts/lib/load-env.js` | `credentials/asc-api-key.p8` |
| `scripts/build-ios.js`, `scripts/build-android.js` | `credentials/release.keystore` |
| `scripts/submit-ios.js`, `scripts/submit-android.js` | `credentials/play-service-account.json` |
| `scripts/bump-version.js` | `android/` (regenerated each build) |
| `scripts/patch-android-build-gradle.js` | `ios/build/`, `ios/Pods/`, `ios/.xcode.env.local` |
| `google-services.json` (Firebase) | |
| `.env.example` | |

## Current implementation

- Login: `POST /auth/login` (email + password)
- Challenges: `GET /approvals/requests?status=pending`, decision via `POST /approvals/requests/{id}/decide`
- Approvals: `GET/DELETE /approvals/grants`
- Push: `POST /notifications/devices` (apns/fcm tokens)
- Account deletion: `DELETE /users/me`

## Deep links & push

- Deep link scheme: `{APP_SCHEME}://challenge/{challenge_id}` → `ChallengeMinimal`
- Universal links: when `*_UNIVERSAL_LINK_HOST` is set, that host is added to iOS `associatedDomains` and Android's intent filter. The host of `*_API_BASE_URL` is also auto-added to iOS `associatedDomains`.

## Session

- Access token persisted via `SecureStore`. Cold start restores session into `Dashboard` or `Auth`.

## Key files

- `app.config.ts` — Expo config, reads env via shared loader
- `scripts/lib/load-env.js` — env loader + DEV↔PROD per-field fallback
- `scripts/build-ios.js`, `scripts/build-android.js` — native build orchestrators
- `scripts/submit-ios.js`, `scripts/submit-android.js` — upload orchestrators
- `scripts/bump-version.js` — increments build numbers in `.env.*`
- `src/lib/api/mobileApi.ts`, `src/lib/api/http.ts` — HTTP client
- `src/features/auth/AuthSessionContext.tsx` — auth state + telemetry init
- `src/app/linking.ts` — deep link routing
