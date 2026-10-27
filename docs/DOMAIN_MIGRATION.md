# Domain migration

NyxID can serve old and new API origins from the same backend while minting URLs
with the current `BASE_URL`. Alias settings are opt-in. With them unset, token
verification, URL checks and cookie domains retain their existing behavior.
This runbook changes no production configuration by itself.

The planned migration is:

| Surface | Existing | Target |
| --- | --- | --- |
| Browser app | `https://nyx.chrono-ai.fun` | `https://nyxid.ai` |
| API | `https://nyx-api.chrono-ai.fun` | `https://api.nyxid.ai` |

Both API hosts must serve the same backend, database and signing keys. Do **not**
redirect the old API: redirects can discard authorization, change POSTs, break
WebSocket upgrades and invalidate signatures. The old browser host can redirect
pages after cutover. Existing bearer/API-key clients continue using their stored
URLs. Browser cookies cannot move between unrelated domains; users sign in once
on the new site. Existing session lifetimes and refresh/revocation rules still
apply.

## 1. Prepare and register both destinations

- Deploy the alias-capable server to **every replica** with aliases unset first.
  Preserve MongoDB data, JWT active/retained keys, encryption keys and all client
  registrations. Do not rotate keys or revoke sessions as part of the move.
- Inventory URLs in clients, provider configuration, catalog entries, webhooks,
  gateway profiles, deployment manifests and monitoring. Capture the current
  effective `JWT_ISSUER`, audience (`JWT_AUDIENCE` or `BASE_URL`), `COOKIE_DOMAIN`,
  CORS/CSRF settings and proxy trust ranges. An explicitly customized issuer is
  not inferred from the old API hostname: keep it pinned until consumers migrate.
- Provision DNS, certificates and ingress for both API origins. Route both to the
  same backend, including `/oauth`, `/mcp`, WebSockets, callbacks, catalog assets
  and specs. Preserve path/query bytes. Overwrite forwarded headers at ingress;
  configure `TRUSTED_PROXY_IPS` narrowly. Cookie alias selection honors a single
  `X-Forwarded-Host` only from those trusted peers, otherwise Host. A comma chain
  or malformed authority is never trusted. RFC `Forwarded` is not used here.
- Add the **new redirect URI alongside the old** in Google, GitHub, Apple, X and
  every connected OAuth provider. Register the exact paths emitted by NyxID;
  do not assume all providers share a callback path. Review Apple return URLs
  and verified domains, Google consent/authorized domains, GitHub OAuth apps,
  X callback restrictions, and each provider's client-secret/domain rules.
  Providers that permit only one callback may need a second registration and
  a coordinated migration. Keep old callbacks reachable for in-flight logins.
- Re-register/update IFTTT service OAuth settings as required by IFTTT. Add the
  new Meta app domains, valid OAuth redirects and webhook callback URLs. Update
  the Lago webhook destination while preserving its secret and event dedupe.
  Keep Telegram manager/bot, Twilio and other webhook URLs alive on the old API
  until their configurations are migrated. Do not regenerate webhook secrets.
- Prepare the new frontend's nginx `BACKEND_URL`, `NYXID_PUBLIC_ORIGIN`, CSP and
  browser connect-origin configuration. These are separate deployment controls;
  backend alias settings do not rewrite the frontend image or ingress.

## 2. Preconfigure verification overlap

While the old origins are still canonical, deploy aliases for the new origins
on all replicas. Explicitly pin the **existing** issuer and audience before
changing either base URL. For the standard production defaults:

```dotenv
BASE_URL=https://nyx-api.chrono-ai.fun
FRONTEND_URL=https://nyx.chrono-ai.fun
JWT_ISSUER=https://nyx-api.chrono-ai.fun
JWT_AUDIENCE=https://nyx-api.chrono-ai.fun
BASE_URL_ALIASES=https://api.nyxid.ai
FRONTEND_URL_ALIASES=https://nyxid.ai
```

If the existing issuer is different, use that actual value. OIDC discovery always
reports `JWT_ISSUER`; accepting aliases inside NyxID does not cause external OIDC
libraries to accept another issuer. Test discovery and authorization with those
libraries before cutover. MCP protected-resource metadata also advertises the
pinned issuer when it matches a configured API origin. The old discovery/JWKS
endpoints must stay available.

Aliases are comma-separated HTTPS **origins**, without paths, userinfo, queries
or fragments; one trailing slash is allowed and removed. At most 32 entries per
list. Frontend aliases do not grant JWT trust. Never add an origin you do not
control. No request Host or forwarded header expands the configured trust set.

## 3. Flip canonical origins

After all replicas understand both origins, deploy:

```dotenv
BASE_URL=https://api.nyxid.ai
FRONTEND_URL=https://nyxid.ai
JWT_ISSUER=https://nyx-api.chrono-ai.fun
JWT_AUDIENCE=https://nyx-api.chrono-ai.fun
BASE_URL_ALIASES=https://nyx-api.chrono-ai.fun
FRONTEND_URL_ALIASES=https://nyx.chrono-ai.fun
COOKIE_DOMAIN=.nyxid.ai
COOKIE_DOMAIN_ALIASES=.chrono-ai.fun
```

The cookie example assumes `.chrono-ai.fun` was the old cookie domain. Use the
actual previously deployed domain and the narrowest appropriate scope; do not
broaden cookie access across sibling applications casually. Cookie aliases are
configured DNS domains, not URLs. On an old API callback, NyxID uses the matching
alias domain for its auth cookies, including clearing cookies. Social provider
authorization and code exchange also use the matched configured API origin for
`redirect_uri`, so a callback on the old host keeps its original URI. The new host uses
`COOKIE_DOMAIN`. Aliases do not copy a browser session to the new site. If the old
configuration used host-only cookies, test the callback flow and cookie cleanup
explicitly: a configured Domain cookie is not identical to a host-only cookie.

On startup, NyxID rewrites stored `openapi_spec_url` values in catalog services and
user endpoints from an API alias to the current base **only** for exact registered
`/api/v1/catalog-specs/...` paths. Unknown specs, query/fragment variants and
foreign origins stay untouched. Updates compare the old value before writing and
are idempotent. The existing hosted-spec resolver already uses compiled NyxID
overlays for recognized paths without fetching that host; that behavior is
preserved. No remote spec gains permission to provide NyxID-owned extensions.

Resource URIs and delegation resources compare alias/current origins as
equivalent; newly minted resource URLs use the canonical base. DPoP verification
accepts proofs for either configured API origin and retains its replay fence.
Twilio verifies against each configured API base without changing signed bytes.
Catalog icon references and existing Telegram manager callbacks accept configured
aliases. Persisted gateway **NyxID issuer** checks use the overlap; external
gateway service destinations remain strict and are not redirected or rewritten.

Enable the old frontend's 301 only after the new frontend works. Preserve paths
and queries. If the old frontend hostname also reverse-proxies OAuth/API/callback
traffic, keep those routes as aliases and redirect page routes only; redirecting
an in-flight OAuth POST or signed webhook is unsafe. Finish old browser login
flows before retiring such routes.

## 4. Verify and release clients gradually

Check both API hosts against the same account and pre-cutover credentials:

- Existing access tokens and refresh rotation, CLI/mobile sessions, assistant
  and MCP sessions, service-account tokens and delegated/relay operations.
- OAuth authorization/code exchange, refresh, introspection, token exchange,
  discovery/JWKS and client-specific ID-token audiences. Pinned issuer/audience
  should remain unchanged even though endpoint URLs use the new base.
- RFC 8707 requests and DPoP proofs naming the old API, including replay rejection.
- Twilio signed HTTP/WS ingress, Telegram manager readiness, Meta/provider and
  Lago webhook deliveries. Observe status/metadata only; never log credentials.
- Browser sign-in, social callbacks on the old host, CORS preflights, CSRF,
  approval flows, voice and logout. Re-login on the new site is expected.
- Hosted tools and overlays, catalog icons, Aevatar browser authorization and
  persisted gateway entries. Aevatar's accepted public API origins come from
  NyxID's own discovery, not a downstream service's claimed configuration.

Release CLI/plugin defaults and mobile builds separately, then update stored
callback/client URLs where supported. Existing API hostnames must remain live
until their users and integrations have migrated; this server PR does not alter
those defaults, app builds, agent prompts, sitemaps or production configuration.

For rollback, restore canonical URLs and keep the other origins as aliases on
all replicas. Leave issuer/audience pinned and retain the same keys and data.
Hosted-spec normalization can safely run again in the opposite direction.

## 5. Retire the old issuer and host with overlap

Coordinate issuer/audience changes with external OIDC clients and gateway
consumers: their validators may require an exact issuer. Change `JWT_ISSUER` and
`JWT_AUDIENCE` to the new API only once those consumers support it. Keep the old
API in `BASE_URL_ALIASES`; existing old tokens will still verify while new tokens
use the new values. Client-specific and dedicated relay audiences never change.

Keep the overlap for at least the longest outstanding token lifetime plus clock
skew, and until persisted grants/resources, pending gateway requests and stored
callback URLs have been migrated, completed or reauthorized. Rolling refresh
sessions and long-lived saved configurations make a fixed short delay
insufficient. Old API URLs may still be in client releases or user scripts.

`BASE_URL_ALIASES` couples old URL compatibility with old issuer/audience trust.
Removing an alias retires both: do not remove it just because the last access
token expired. If independent issuer retirement is required while old URLs stay
accepted, design a separate policy first. Finally remove obsolete aliases,
provider redirects and cookie domains, and decommission old API ingress only
when telemetry and client inventory show it is unused. External DNS/redirect
retirement is an operator action, never an automatic server migration.
