# NyxID Mobile

NyxID mobile client (React Native + Expo). Two environments — `dev` and `prod` — driven entirely by env files. Nothing is hardcoded into scripts.

## Quick start

```bash
cd mobile
pnpm install          # also generates eas.json from your env
cp .env.example .env.dev    # or .env.prod, or both
# edit .env.* with your URLs, bundle IDs, EAS project ID, etc.
pnpm start            # Metro + dev client (defaults to APP_ENV=dev)
```

Native run:

```bash
pnpm ios              # APP_ENV=dev
pnpm android          # APP_ENV=dev
```

Android local debug (emulator/device, API URLs): see [docs/ANDROID_DEBUG.md](docs/ANDROID_DEBUG.md).

## Environment

`.env.example` is the source of truth for the env surface. Copy to `.env.dev`, `.env.prod`, or both. Everything outside one-time secrets (the `.p8` and the Play service-account JSON) flows through env.

### Per-profile env vars (DEV_* / PROD_* with bidirectional fallback)

| Variable | Required? | Purpose |
| --- | --- | --- |
| `*_API_BASE_URL` | yes (one of dev/prod) | Backend API base URL. Canonical "this profile is populated" signal. |
| `*_IOS_BUNDLE_ID` | yes (one of dev/prod) | iOS bundle identifier (reverse-DNS). |
| `*_ANDROID_PACKAGE` | yes (one of dev/prod) | Android application id (reverse-DNS). |
| `*_APPLE_ASC_APP_ID` | only for `submit:*` | Numeric ASC App ID; needed if your dev and prod ASC apps are different. |
| `*_UNIVERSAL_LINK_HOST` | optional | Host for iOS associated domains + Android intent filter. Omit to skip universal links. |
| `*_UNIVERSAL_LINK_PATH_PREFIX` | optional | Android intent-filter path prefix (e.g. `/REzJ` for AppsFlyer OneLink). |
| `*_ALLOWED_EMAILS` | optional | Comma-separated emails; empty = allow all signed-in users. |
| `*_TELEMETRY_DSN` | optional | PostHog DSN; empty = telemetry off. |
| `*_TELEMETRY_HOST` | optional | PostHog host URL. |
| `*_SHARE_ANALYTICS` | optional | `"true"` to opt-in to upstream analytics. |

**Fallback rules** (applied per-field by `app.config.ts`):

- Active profile's field empty → falls back to the other profile's value.
- Both `*_API_BASE_URL` empty → build aborts with a loud error.
- Both `*_IOS_BUNDLE_ID` empty (or both Android package empty) → build aborts.

### App-identity env vars (single value, shared across profiles)

| Variable | Required? | Default | Purpose |
| --- | --- | --- | --- |
| `EAS_PROJECT_ID` | yes | — | UUID from your EAS dashboard (`eas init`). |
| `APP_NAME` | no | `NyxID Mobile` | Display name. |
| `APP_SLUG` | no | `nyxid-mobile` | EAS slug. |
| `APP_SCHEME` | no | `nyxid` | Custom URL scheme. |

### Submit-credential env vars (account-wide, only needed for `submit:*`)

| Variable | Notes |
| --- | --- |
| `APPLE_ID` | Apple Developer account email |
| `APPLE_TEAM_ID` | 10 chars uppercase+digits |
| `ASC_API_KEY_ID` | 10 chars uppercase+digits |
| `ASC_API_KEY_ISSUER_ID` | UUID |

Local merge order: `.env.dev` → `.env.prod` → `.env.local` → `process.env` (later wins).

## Build & deploy (local EAS)

| Command | What it does |
| --- | --- |
| `pnpm build:dev` | `eas build --profile dev --platform all --local` |
| `pnpm build:prod` | `eas build --profile prod --platform all --local` |
| `pnpm submit:dev` | uploads to TestFlight (iOS) + Play **Internal testing** track (Android) |
| `pnpm submit:prod` | same: TestFlight (iOS) + Play **Internal testing** track (Android) |
| `pnpm release:dev` | build then submit (dev) |
| `pnpm release:prod` | build then submit (prod) |

Every `build:*` / `submit:*` invocation re-renders `eas.json` via `scripts/render-eas-json.js` using your `.env.*` values. The script only emits an iOS submit block when all the Apple creds for that profile are present, so `pnpm build:*` works with no submit creds set.

`eas build --local` runs the native toolchain on your machine; `eas submit` uploads the resulting artifact (no local-only flag — the upload itself necessarily talks to Apple/Google).

> **macOS required for iOS builds.** `--platform all` on Linux will fail at the Xcode step. Use `--platform android` on Linux, or split into `--platform ios` and `--platform android` invocations.

### Submission targets

| Platform | Where the build lands | How to promote |
| --- | --- | --- |
| iOS | App Store Connect → TestFlight (automatic; ASC routes all uploads here first) | Promote to App Store via ASC web UI when ready. |
| Android | Google Play Console → **Internal testing** track | Promote to Closed → Open → Production via Play Console web UI. |

Both `submit:dev` and `submit:prod` target the same internal-testing slots — this is the TestFlight-equivalent staging area. Production publication is an explicit web-UI step, not something `pnpm release:prod` can do (intentionally — no accidental shipping).

### Build versioning

`eas.json` uses `appVersionSource: "remote"` — EAS tracks `buildNumber` (iOS) and `versionCode` (Android) on its server. `autoIncrement: true` advances both on each build. The marketing version (`CFBundleShortVersionString`) still comes from `app.config.ts`.

If you're continuing from an existing TestFlight history, seed EAS once with the current native values so build numbers don't reset to 1:

```bash
pnpm exec eas build:version:set --platform ios     --profile prod --value 34
pnpm exec eas build:version:set --platform android --profile prod --value 1
```

### One-time setup for `submit:*`

1. **App Store Connect API key** — ASC → Users & Access → Integrations → App Store Connect API → `+`. Download the `.p8` (one-time only). Save as `mobile/credentials/asc-api-key.p8`. Record the Key ID and Issuer ID in `.env.dev` / `.env.prod`.
2. **Google Play service account** — Play Console → Setup → API access → create service account with at least "Release manager" role and grant Play Console access. Download JSON. Save as `mobile/credentials/play-service-account.json`.
3. Fill in the `APPLE_ID`, `APPLE_TEAM_ID`, `ASC_API_KEY_ID`, `ASC_API_KEY_ISSUER_ID` values + the per-profile `*_APPLE_ASC_APP_ID`. EAS validates these formats (email, 10-char team ID, etc.) — see `.env.example` for the expected shape.

`credentials/` is gitignored — never commit `.p8` or service-account JSON files.

`pnpm build:*` does not need any submit credentials; it only needs `*_API_BASE_URL` + `*_IOS_BUNDLE_ID` + `*_ANDROID_PACKAGE` + `EAS_PROJECT_ID`.

> **Known limitation:** if dev and prod use the same Android package, Firebase / Crashlytics events from both will share the same Analytics property. Set distinct `DEV_ANDROID_PACKAGE` / `PROD_ANDROID_PACKAGE` (and matching Firebase Android apps in `google-services.json`) to fully separate them.

## What ships in git vs. what stays local

| In git | Gitignored |
| --- | --- |
| `app.config.ts` | `eas.json` (generated each install/build) |
| `scripts/render-eas-json.js` | `.env`, `.env.dev`, `.env.prod`, `.env.local` |
| `google-services.json` | `credentials/asc-api-key.p8` |
| `.env.example` | `credentials/play-service-account.json` |
|  | `ios/.xcode.env.local` |

## Current implementation

- Login: `POST /auth/login` (email + password)
- Challenges list: `GET /approvals/requests?status=pending`
- Challenge detail: `GET /approvals/requests/{id}`
- Challenge decision: `POST /approvals/requests/{id}/decide`
- Approvals: `GET /approvals/grants`, `DELETE /approvals/grants/{id}`
- Push registration: `POST /notifications/devices` (native `apns`/`fcm` token)
- Account deletion: `DELETE /users/me`

## Deep links & push

- Deep link scheme: `{APP_SCHEME}://challenge/{challenge_id}` → `ChallengeMinimal`
- Supported payload fields: `deeplink`, `url`, `challenge_id`, `challengeId`
- Universal Links: when `*_UNIVERSAL_LINK_HOST` is set, that host is added to iOS `associatedDomains` and Android's intent filter. `*_API_BASE_URL`'s host is also auto-added to iOS `associatedDomains`.

## Session

- Access token persisted via `SecureStore`
- Cold start restores session into `Dashboard` or `Auth`

## Key files

- `app.config.ts` — env-driven Expo config; per-field DEV↔PROD fallback
- `scripts/render-eas-json.js` — generates `eas.json` from env
- `src/lib/api/mobileApi.ts`, `src/lib/api/http.ts` — HTTP client
- `src/features/auth/AuthSessionContext.tsx` — auth state + telemetry init
- `src/lib/auth/sessionStore.ts` — SecureStore wrapper
- `src/app/linking.ts` — deep link routing
