# Mobile production domain migration

The production frontend moves from `https://nyx.chrono-ai.fun` to `https://nyxid.ai`; the backend moves from `https://nyx-api.chrono-ai.fun` to `https://api.nyxid.ai`. The mobile API base keeps its `/api/v1` suffix.

Production builds embed both domain sets. The NyxID admin feature flag **`mobile:new-domains`** (Admin → Feature flags, global only, default off) selects which set an installed app uses. The app reads the flag at every launch, before any domain-dependent app work runs, so switching it needs no rebuild: users restart or reload the app to move to the selected domains.

## Environment configuration

Keep the existing values in `mobile/.env.prod` and add the new set to the same file:

```dotenv
PROD_API_BASE_URL=https://nyx-api.chrono-ai.fun/api/v1
PROD_FRONTEND_URL=https://nyx.chrono-ai.fun
PROD_LEGAL_BASE_URL=https://nyx.chrono-ai.fun

PROD_NEW_API_BASE_URL=https://api.nyxid.ai/api/v1
PROD_NEW_FRONTEND_URL=https://nyxid.ai
PROD_NEW_LEGAL_BASE_URL=https://nyxid.ai
```

Set all three `PROD_NEW_*` URLs or none. Each must be an HTTPS URL without credentials, a query or a fragment; a partial or invalid set aborts the build. Without them the build has a single domain set and ignores the flag. Only production builds (`APP_ENV=prod`) embed the second set. The retired `PROD_USE_NEW_DOMAINS` build variable is no longer read; the loader prints a warning if it is still present.

`PROD_IOS_BUNDLE_ID`, `PROD_ANDROID_PACKAGE`, `APP_SCHEME`, app identity, Apple team, analytics settings and release numbering keep using their current variables. Metro's cache version includes both domain sets, so changing either invalidates cached environment substitutions.

## How the app selects the domains

`index.ts` registers `AppRoot`, which renders nothing (the splash screen stays up) until `DomainGate` finishes. The gate calls `GET /api/v1/public/config` on both API hosts in parallel and applies the first valid `mobile_new_domains` boolean either host returns. A host that is unreachable, returns an error, or runs a backend too old to send the field gives no answer and is ignored. Each launch waits at most 5 seconds. If neither host answers in time, the last answer saved on this device applies, and a device that has never received an answer uses the original domains. A received answer is saved in SecureStore for that fallback. The gate also confirms the chosen set's API host is reachable (a successful `/public/config` response): it settles as soon as the flag is known and the host it selects has answered, so a hung host only delays launch when it is the chosen one. If the chosen host is unreachable but the other answered, that launch uses the reachable set, since both hosts serve the same backend and accept the same sessions; the saved choice still records only the flag. If neither host is reachable, the flag/saved choice applies unchanged.

The selection lasts until the next launch. It switches the API base used by every request (`src/lib/api/baseUrl.ts`), the frontend URL used by waitlist, registration and web links, and the legal base URL used by Privacy / Terms. The server only chooses between the two embedded sets; it never supplies a URL.

### URL resolution and startup boundaries

`domains.ts` keeps the selected set private. `activeDomains()`,
`getFrontendUrl()` and `getLegalBaseUrl()` read it when called;
`api/baseUrl.ts::getApiBaseUrl()` validates the selected API base and
`buildApiUrl()` joins a relative endpoint. There are no mutable URL exports.
Import accessors freely, but call them only when building a request or opening a
link, never in a module-level constant, a cached default or a configuration
object. Dual-set builds throw on an active-domain read before selection, so an
early consumer cannot silently retain the legacy host. Single-set builds are
ready immediately and perform no flag lookup.

Initialization always selects a set in `finally`, with precedence remote answer
→ saved answer → legacy. A SecureStore read failure means no saved answer; a
write failure is ignored and never replaces the remote answer. An unexpected
exception preserves whichever answers are already known, in that same order.
`DomainGate` admits the app when initialization settles. Session restore,
refresh and resume timers, public auth configuration, Query observers, push-token
registration, and telemetry requests all start inside that gate. The early
`bootstrapNotificationInfrastructure()` call in `index.ts` only configures native
notification delivery, defines the headless task and persists pending signals;
it neither builds URLs nor contacts NyxID. The telemetry relay and QueryClient
created during module evaluation hold functions and state, without making
requests or resolving domains. Navigation prefixes and QR allowlists are static
trust metadata built from **all** embedded sets, independent of the active set;
they never choose an execution destination. Native voice sockets obtain their
API base through a callback when each connection opens.

The former mutable exports worked under Expo's default live-binding transform
but retained stale values under its static transform; accessors avoid that
dependency. `mutableExports.test.ts` guards against mutable exports and their
re-exports across production source. The node-loader tests import consumers
before selection, reject early reads, and check auth, refresh, legal and frontend
URLs after selection. Gate tests cover startup, dependency failures, saved/legacy
fallbacks and headless notifications; auth screen tests cover registration and
waitlist callbacks.

Independent of the selection, the app trusts both embedded sets:

- QR login accepts web QR codes from both frontends and both API hosts, so it works whichever frontend the backend's `FRONTEND_URL` names. Scanned URLs never choose the backend.
- iOS keeps the existing API hosts plus the existing universal-link host until the owner chooses the A or B association set. Android keeps its existing universal-link filter by default; profile-local `DEV_ANDROID_HTTPS_APP_LINKS=true` or `PROD_ANDROID_HTTPS_APP_LINKS=true` adds verified filters for the selected host list using `shared/native-app-links.json` after Asset Links is deployed on every host. React Navigation accepts both API and frontend origins, with host boundaries enforced.
- `PROD_UNIVERSAL_LINK_HOST` / `PROD_UNIVERSAL_LINK_PATH_PREFIX` (for example `nyxid.onelink.me` / `/REzJ`) are unchanged. Frontend hosts attach to native paths only if the owner selects host set A; otherwise ordinary frontend web links keep opening in the browser.

The owner decision to associate API hosts only or API plus frontend hosts is pending. The generator's `associatedUrls` input attaches the same shared paths to either set; the current input preserves the existing associations. React Native routing prefixes always include the trusted API and frontend origins, regardless of which host set the operating system associates. Unsupported HTTPS paths received from a trusted API host open on the launch-selected frontend in an in-app browser, preserving path/query. Trusted frontend URLs retain their own origin. OneLink and untrusted hosts never use that fallback. SSH sockets resolve the active API base when opening. Service OAuth returns use `nyxid://services/oauth-return`, independent of the selected HTTPS hosts, and never navigate or cancel queued notification intents. See [Native integration](NATIVE_INTEGRATION.md) for external configuration and rollout order.

## Verify and build

From `mobile/`, preview the configuration and run the checks:

```bash
APP_ENV=prod pnpm exec expo config --type public
pnpm test
pnpm typecheck
pnpm check:native-sync
```

Confirm `ios.associatedDomains` lists `applinks:nyx-api.chrono-ai.fun`, `applinks:api.nyxid.ai` and the existing universal-link host. Android retains the existing OneLink filter; only the profile-local `*_ANDROID_HTTPS_APP_LINKS=true` opt-in adds selected-host filters for `shared/native-app-links.json` paths after all hosts are associated. Build as usual with `pnpm build:ios` and `pnpm build:android`; the logs show both domain sets. For direct Xcode or Gradle builds, keep the URLs in `.env.prod` so prebuild, native compilation and JavaScript bundling resolve the same values.

## Login and existing sessions

Email/password login uses `POST /auth/login` through the shared API resolver with `client: "mobile"`. Social login opens `/auth/social/{provider}?client=mobile&redirect_uri=...` on the selected API base. The provider returns to the backend, which completes login and redirects into the app at `nyxid://auth/social/callback`.

The backend builds provider callbacks from `BASE_URL` or a matching explicitly configured `BASE_URL_ALIASES` origin selected by the server. The mobile flag does not change those server settings. Register both the existing callbacks and these new callbacks before turning the flag on:

| Provider | Backend callback |
| --- | --- |
| Google | `https://api.nyxid.ai/api/v1/auth/social/google/callback` |
| GitHub | `https://api.nyxid.ai/api/v1/auth/social/github/callback` |
| Apple | `https://api.nyxid.ai/api/v1/auth/social/apple/callback` |

Keep both callback sets reachable throughout the rollout. If `COOKIE_DOMAIN` is explicitly configured, it must be valid for the serving API host; configure `COOKIE_DOMAIN_ALIASES` for the other controlled domain as described in the [backend domain migration runbook](../../docs/DOMAIN_MIGRATION.md).

Stored access and refresh tokens are kept when the app switches domains. Follow the [backend runbook](../../docs/DOMAIN_MIGRATION.md) so both hosts accept them: deploy the alias-capable server to every replica, configure both API origins, and pin the existing `JWT_ISSUER` and `JWT_AUDIENCE`. For the standard claims that means `BASE_URL=https://api.nyxid.ai`, `BASE_URL_ALIASES=https://nyx-api.chrono-ai.fun`, and `JWT_ISSUER` / `JWT_AUDIENCE` pinned to `https://nyx-api.chrono-ai.fun`. Without the overlap, existing tokens can be rejected after the switch, and a protected request that stays unauthorized after refresh signs the user out.

QR login approval uses the selected API for previews and decisions. Both API domains must reach the same login-request records.

## Rollout and rollback

1. Make the new domains ready: DNS/TLS, the alias-capable backend on every replica, the provider callbacks above, `apple-app-site-association` on `api.nyxid.ai` with the existing app identifier, and the legal markdown on `nyxid.ai`.
2. Ship a build containing both domain sets, with `mobile:new-domains` off. Apps keep using the original domains.
3. Turn `mobile:new-domains` on globally in Admin → Feature flags. Each app switches on its next launch or reload.
4. On devices, verify social sign-in, session restoration and refresh, push registration and approval decisions, web QR login from both frontends, custom-scheme QR login, native links, the waitlist link, and Privacy / Terms. Confirm a QR URL from an unconfigured domain is still rejected.

To roll back, turn the flag off; apps return to the original domains on their next launch. Keep both domain sets available while any installed app may use them. Builds released before this change have only the original domains and ignore the flag.

The backend's HTTPS `BASE_URL` selects the native login QR origin. `FRONTEND_URL` selects the browser fallback and older QR responses. Both origins must be embedded in supported mobile builds; keep them on the original domains until older builds are no longer in use.
