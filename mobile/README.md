# NyxID Mobile

NyxID mobile client (React Native + Expo). Two environments — `dev` and `prod` — driven entirely by env files. No values are baked into scripts.

## Quick start

```bash
cd mobile
pnpm install          # also renders eas.json from eas.json.template
cp .env.example .env.dev    # or .env.prod, or both
# edit .env.* with your backend URL etc.
pnpm start            # Metro + dev client (defaults to APP_ENV=dev)
```

Native run:

```bash
pnpm ios              # APP_ENV=dev
pnpm android          # APP_ENV=dev
```

Android local debug (emulator/device, API URLs): see [docs/ANDROID_DEBUG.md](docs/ANDROID_DEBUG.md).

## Environment

A single `.env.example` is the source of truth for the env surface. Copy to `.env.dev`, `.env.prod`, or both. The build resolver (`app.config.ts`) applies these rules:

- `APP_ENV=dev` → uses `DEV_*` vars; if `DEV_API_BASE_URL` is empty, falls back to `PROD_*` (with warning).
- `APP_ENV=prod` → uses `PROD_*` vars; if `PROD_API_BASE_URL` is empty, falls back to `DEV_*` (with warning).
- Both empty → build fails loudly.

`.env.dev`, `.env.prod`, `.env.local` are all gitignored. Local merge order: `.env.dev` < `.env.prod` < `.env.local` < `process.env`.

## Build & deploy (local EAS)

| Command | What it does |
| --- | --- |
| `pnpm build:dev` | `eas build --profile dev --platform all --local` |
| `pnpm build:prod` | `eas build --profile prod --platform all --local` |
| `pnpm submit:dev` | `eas submit --profile dev --platform all` (Play track: `internal`) |
| `pnpm submit:prod` | `eas submit --profile prod --platform all` (Play track: `production`, `draft`) |
| `pnpm release:dev` | build then submit (dev) |
| `pnpm release:prod` | build then submit (prod) |

Every `build:*` / `submit:*` invocation re-renders `eas.json` via `scripts/render-eas-json.js` using your `.env.*` values. The script only emits an iOS submit profile when all five Apple credentials are present, so `pnpm build:*` works with no submit creds set.

`eas build --local` runs the native toolchain on your machine; `eas submit` uploads the resulting artifact to ASC / Play Console (no local-only flag for submit — the upload itself necessarily talks to Apple/Google).

> **macOS required for iOS builds.** `--platform all` on Linux will fail at the Xcode step. Use `--platform android` on Linux, or split into `--platform ios` and `--platform android` invocations.

### Build versioning

`eas.json` uses `appVersionSource: "remote"` — EAS tracks `buildNumber` (iOS) and `versionCode` (Android) on its server. `autoIncrement: true` advances both on each build. The marketing version (`CFBundleShortVersionString`) still comes from `app.config.ts`.

One-time seed from the current native values so the next build picks up where the last `build:ios:testflight` left off (currently iOS `CFBundleVersion=34`, Android `versionCode=1`):

```bash
pnpm exec eas build:version:set --platform ios     --profile prod --value 34
pnpm exec eas build:version:set --platform android --profile prod --value 1
```

### One-time setup for `submit:*`

1. **App Store Connect API key** — ASC → Users & Access → Integrations → App Store Connect API → `+`. Download the `.p8` (one-time). Save as `mobile/credentials/asc-api-key.p8`. Record the Key ID and Issuer ID in `.env.dev` / `.env.prod`.
2. **Google Play service account** — Play Console → Setup → API access → create service account with at least "Release manager" role and grant Play Console access. Download JSON. Save as `mobile/credentials/play-service-account.json`.
3. Fill in the `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_ASC_APP_ID`, `ASC_API_KEY_ID`, `ASC_API_KEY_ISSUER_ID` values in `.env.dev` / `.env.prod`. EAS validates these formats (email, 10-char team ID, etc.) — see `.env.example` for the exact shape each expects.

`credentials/` is gitignored — never commit `.p8` or service-account JSON files.

`pnpm build:*` does not need submit credentials; it only needs `*_API_BASE_URL`.

> **Known limitation:** dev and prod ship the same Android package (`fun.chronoai.nyxid`) and share `google-services.json`, so Firebase / Crashlytics events from dev testers and prod users land in the same Analytics property. Filter by `APP_ENV` in dashboards, or split out a per-env Firebase Android app if/when this becomes a problem.

## What ships in git vs. what stays local

| In git | Gitignored |
| --- | --- |
| `app.config.ts` | `eas.json` (generated) |
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

- Deep link scheme: `nyxid://challenge/{challenge_id}` → `ChallengeMinimal`
- Supported payload fields: `deeplink`, `url`, `challenge_id`, `challengeId`
- Universal Links: `applinks:nyxid.onelink.me` + `applinks:<host of API_BASE_URL>` (host injected by `app.config.ts`)

## Session

- Access token persisted via `SecureStore`
- Cold start restores session into `Dashboard` or `Auth`

## Key files

- `app.config.ts` — env-driven Expo config with dev↔prod fallback
- `scripts/render-eas-json.js` — generates `eas.json` from template + env
- `src/lib/api/mobileApi.ts`, `src/lib/api/http.ts` — HTTP client
- `src/features/auth/AuthSessionContext.tsx` — auth state + telemetry init
- `src/lib/auth/sessionStore.ts` — SecureStore wrapper
- `src/app/linking.ts` — deep link routing
