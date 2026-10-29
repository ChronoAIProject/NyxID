# Mobile production domain migration

The production frontend moves from `https://nyx.chrono-ai.fun` to `https://nyxid.ai`; the backend moves from `https://nyx-api.chrono-ai.fun` to `https://api.nyxid.ai`. The mobile API base keeps its `/api/v1` suffix.

## Plan

1. Keep the current production URL variables and add a second URL set behind a build-time feature flag that defaults to off.
2. Resolve the new API, frontend, and legal URLs together in the shared environment loader. Fail the build if an enabled migration has a missing or invalid URL. Preserve the app's existing identity, signing, and version configuration.
3. Feed the selected URLs into the existing API clients, QR-login trust, legal screens, waitlist link, Expo native link configuration, and React Navigation prefixes. Preserve the existing universal-link host/path while moving the API's iOS association. Let new-mode production QR scanning also trust the configured legacy frontend during rollout.
4. Verify legacy and new configurations, development fallback, invalid settings, QR trust, and generated native settings. Obtain an Opus review and address findings before handoff.

## Environment configuration

Keep the existing values in `mobile/.env.prod`:

```dotenv
PROD_API_BASE_URL=https://nyx-api.chrono-ai.fun/api/v1
PROD_FRONTEND_URL=https://nyx.chrono-ai.fun
PROD_LEGAL_BASE_URL=https://nyx.chrono-ai.fun
```

Add these variables to the same file:

```dotenv
PROD_USE_NEW_DOMAINS=false
PROD_NEW_API_BASE_URL=https://api.nyxid.ai/api/v1
PROD_NEW_FRONTEND_URL=https://nyxid.ai
PROD_NEW_LEGAL_BASE_URL=https://nyxid.ai
```

An unset or `false` flag uses the original URLs and native link settings. `true` selects all three new URLs. The flag accepts `true` and `false` case-insensitively and ignores surrounding whitespace; other values fail configuration. Enabled new URLs must use HTTPS without credentials, query strings, or fragments. Missing new values never fall back to old or development URLs. New values are ignored while the flag is off.

Both modes keep `PROD_UNIVERSAL_LINK_HOST` / `PROD_UNIVERSAL_LINK_PATH_PREFIX` (for example, `nyxid.onelink.me` / `/REzJ`). In new mode, iOS associates `api.nyxid.ai` plus the existing universal-link host. Android's existing host/path filter stays the same. With no universal-link host configured, no Android HTTPS filter is added. The new frontend is not automatically registered as a native link host, so ordinary web links continue opening in the browser. React Navigation recognizes the configured app scheme and selected API/frontend/link origins, with host boundaries enforced.

QR login trusts the selected API, frontend, and link host. New-mode production builds additionally trust the fixed, configured legacy `PROD_FRONTEND_URL` for QR scanning, exposed as `EXPO_PUBLIC_LEGACY_FRONTEND_URL` at build time. This lets the new app scan web QR codes while the backend still issues the old frontend origin. Scanned URLs never choose the API backend; requests always use the selected API base. Legacy builds keep their original QR trust set. The waitlist link uses the selected frontend (including the dev frontend in dev builds), and Privacy / Terms use the selected legal base URL. If no frontend resolves, the waitlist link is hidden.

At runtime, `src/lib/api/baseUrl.ts` reads the selected `EXPO_PUBLIC_API_BASE_URL` once, normalizes it, and supplies the same base to every API caller, QR trust, and navigation. HTTP requests, refresh/retry requests, and social authorization URLs use `buildApiUrl(path)`. There is no runtime localhost fallback: an API request with missing configuration fails explicitly. Endpoint paths cannot override the configured backend with an absolute URL.

`PROD_IOS_BUNDLE_ID=fun.chrono-ai.nyxid`, `PROD_ANDROID_PACKAGE=fun.chronoai.nyxid`, `APP_SCHEME=nyxid`, app identity, Apple team, analytics settings, and release numbering keep using their current variables. Increment build numbers as usual for each release.

The loader merges `.env.dev`, `.env.prod`, `.env.local`, then shell variables, in that order; later values win. A shell flag therefore overrides `.env.prod`. **Persist the release flag in `.env.prod`** so prebuild, native compilation, and JavaScript bundling resolve the same values. Shell overrides are useful for previews and the scripted build commands, whose child processes inherit the override. Explicit dev fields keep their values; empty dev fields retain the existing fallback to the selected production profile. An enabled flag requires a complete new production URL set even when resolving dev builds. The extra legacy QR origin is enabled only for production builds using the new domains.

## Verify and build

From `mobile/`, preview each configuration before building:

```bash
PROD_USE_NEW_DOMAINS=false APP_ENV=prod pnpm exec expo config --type public
PROD_USE_NEW_DOMAINS=true APP_ENV=prod pnpm exec expo config --type public
```

In the new configuration, confirm `ios.associatedDomains` includes `applinks:api.nyxid.ai` and the existing universal-link host (if configured), the Android intent filters keep their existing host/path, and `extra.LEGAL_BASE_URL` is `https://nyxid.ai`.

Run the automated checks:

```bash
pnpm test
pnpm typecheck
pnpm check:native-sync
```

Then save `PROD_USE_NEW_DOMAINS=true` in `.env.prod` and build:

```bash
pnpm build:ios
pnpm build:android
```

The build logs show the selected API, frontend, and legal URLs. The normal build scripts run Expo prebuild so the selected native link configuration is written into the native projects. Prefer these scripted release builds. For direct Xcode or Gradle builds, persist the flag in `.env.prod`, consistently supply `APP_ENV=prod` to prebuild and bundling, and run Expo prebuild for that platform after switching the flag. A flag supplied only to a prebuild shell command will not reach a later Xcode GUI archive, which can split native and JavaScript configuration. Committed native settings can reflect a previous domain selection. Restart Metro with `pnpm start --clear` after changing build-time values during development.

## Login and existing sessions

Email/password login uses `POST /auth/login` through the shared API resolver with `client: "mobile"`. Social login opens `/auth/social/{provider}?client=mobile&redirect_uri=...` on the same selected API base. The provider returns to the backend, which completes login and redirects into the app at `nyxid://auth/social/callback`. This app return link keeps using the existing `nyxid` scheme.

The backend builds provider callbacks from `BASE_URL` or a matching explicitly configured `BASE_URL_ALIASES` origin selected by the server. The mobile feature flag does not change those server settings. Register both the existing callbacks and these new callbacks before routing mobile login through `api.nyxid.ai`:

| Provider | Backend callback |
| --- | --- |
| Google | `https://api.nyxid.ai/api/v1/auth/social/google/callback` |
| GitHub | `https://api.nyxid.ai/api/v1/auth/social/github/callback` |
| Apple | `https://api.nyxid.ai/api/v1/auth/social/apple/callback` |

Keep both callback sets reachable throughout the rollout, including in-flight logins. If `COOKIE_DOMAIN` is explicitly configured, it must be valid for the serving API host; configure `COOKIE_DOMAIN_ALIASES` for the other controlled domain as described in the [backend domain migration runbook](../../docs/DOMAIN_MIGRATION.md). Browser cookies do not transfer between unrelated sites.

Mobile stores access and refresh tokens in SecureStore and restores them after an app update. Refresh and authenticated retries use the selected API base. The backend now supports verification overlap through `BASE_URL_ALIASES` and an explicit `JWT_AUDIENCE`. Follow the [backend runbook](../../docs/DOMAIN_MIGRATION.md): deploy the alias-capable server to every replica, pin the actual existing `JWT_ISSUER` and `JWT_AUDIENCE`, configure both controlled API origins, and retain the same signing keys, database, and session/refresh records. Native sessions can then continue across the domain switch without a forced sign-in.

For the standard existing issuer/audience, a new-canonical backend uses `BASE_URL=https://api.nyxid.ai`, `BASE_URL_ALIASES=https://nyx-api.chrono-ai.fun`, and keeps `JWT_ISSUER` and `JWT_AUDIENCE` pinned to `https://nyx-api.chrono-ai.fun`. Use the actual previously deployed claims if customized. Later issuer/audience migration requires the overlap and external-client coordination in the backend runbook. Changing the base without pinning claims or configuring overlap can still reject existing tokens; a protected request that remains unauthorized after refresh triggers the app's existing sign-out flow. This mobile PR changes no backend deployment configuration.

QR login approval continues to use the selected API for previews and decisions, regardless of the scanned frontend origin. The old and new API domains must reach the same login-request records for a QR created through one domain to be approved through the other.

## Release checks and rollback

Before distributing the new-domain build, ensure DNS/TLS, OAuth provider callback URLs, and legal markdown hosting are ready on the new domains. Serve Apple's `apple-app-site-association` on the new API domain with the existing app identifier, and keep the existing universal-link host's Apple/Android association configuration.

The backend's `FRONTEND_URL` selects the web QR origin for every app version. Keep it on the old frontend while distributing the new-domain app: the new build can scan both configured frontend origins. Once adoption is sufficient, change the backend's frontend URL to `https://nyxid.ai`. **Remaining legacy mobile builds will reject web QR codes using the new origin and need updating**; keeping old DNS alive does not solve QR trust. The `nyxid://` QR format remains accepted by both versions. Only the new-domain backend/API must be available before shipping the new-domain app; the backend's frontend QR origin can switch afterward.

On devices, verify social sign-in and its app callback, session restoration/refresh, push registration and approval decisions, web QR login from both configured frontend origins, custom-scheme QR login, native links, the waitlist link, and Privacy / Terms. Confirm a QR URL from an unconfigured domain is still rejected. Old-domain builds require their original domains to remain available during the app-store rollout.

To roll back future builds, set `PROD_USE_NEW_DOMAINS=false`, increment the build numbers, rebuild, and distribute. This flag is embedded at build time and cannot remotely switch an installed binary. Keep both domain sets available while old and new app versions are in use.
