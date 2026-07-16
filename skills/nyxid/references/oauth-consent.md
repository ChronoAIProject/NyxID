# OAuth consent, service grants, and resource indicators

Use this reference when the user asks what an OAuth client can access, how a third-party app requests NyxID services, how to narrow or revoke an app's access, or how RFC 8707 `resource` parameters work.

NyxID follows the OAuth least-privilege pattern: a login consent is not automatically a proxy grant. The user explicitly grants sign-in scopes and, separately, service access for the client.

## What a consent grants

An OAuth consent is the user's grant to one OAuth client. It contains:

- OIDC/OAuth scopes such as `openid`, `profile`, `email`, `roles`, `groups`, `offline_access`, and `proxy`.
- A service grant for NyxID proxy access.
- Grant metadata such as client ID, app name, grant time, and expiry.

Service grants are deny-by-default:

- `allow_all_services = true` means the app may use every currently proxyable service available to the user under the token's effective owner/access rules.
- `allow_all_services = false` with `allowed_service_ids = []` means sign-in only. The app can authenticate the user but cannot call any NyxID proxied service for them.
- `allow_all_services = false` with specific `allowed_service_ids` means the app can call only those `UserService` records.
- `allowed_service_ids = null` with `allow_all_services = false` is a legacy pre-default-deny row. Treat it as a legacy unrestricted grant in management views, and expect the user to be prompted again on the next interactive authorize flow so the grant is rewritten as an explicit choice.

Tokens carry the granted service boundary, and the proxy enforces it. Do not describe a consent as "just sign-in" once the app has `proxy` capability or a service grant.

## Consent screen behavior

The consent screen is summary-first:

- The first view shows the app, redirect host, requested scopes, requested resources, and a read-only service-access summary.
- If no services are selected, the summary says the app only signs the user in.
- `Customize` opens the service picker and the `All services` switch.
- Approving without choosing a service grants sign-in only.
- Denying redirects the client with OAuth `access_denied`.

The picker currently lets users choose active personal services. Explicit RFC 8707 `resource` parameters can also resolve proxyable org services by slug when the backend can validate the user's org access, but do not promise that every org service appears in the picker UI.

## App-declared default services

Developer apps can declare `default_service_catalog_slugs`. These are catalog slugs the app wants to pre-request at consent time.

Important semantics:

- Defaults are a preselection hint, not a grant. The user keeps the final say.
- The backend resolves each declared catalog slug against the consenting user's own matching `UserService` rows when building the consent redirect.
- Matching services are preselected on the consent screen.
- Declared catalog services with no user match are shown informationally as unmatched defaults.
- The app owner may declare auto-connected or system catalog services if they exist in the catalog.
- Unknown catalog slugs are rejected when the developer app is saved.
- A developer app can declare at most 25 default catalog slugs.

Current management surfaces:

- Web UI: Developer Apps -> app detail -> `Default Services` uses the include-all catalog picker.
- API: `POST /api/v1/developer/oauth-clients` and `PATCH /api/v1/developer/oauth-clients/{client_id}` accept `default_service_catalog_slugs`.
- Dynamic Client Registration: `POST /oauth/register` accepts and returns `default_service_catalog_slugs`. NyxID validates the slugs before creating the ownerless client.
- Admin API: `POST /api/v1/admin/oauth-clients` and `PATCH /api/v1/admin/oauth-clients/{client_id}` accept `default_service_catalog_slugs`; admin responses include the persisted list. This is the repair path for ownerless DCR clients created before the field was supported.
- CLI: ordinary `nyxid developer-app create/update` exists, but default-service flags are not available yet. CLI parity is tracked separately in issue #1126.

Existing DCR clients are not backfilled automatically. Re-register the client with the desired defaults or patch the existing client once through the admin API after deploying support.

## RFC 8707 resource indicators

NyxID supports RFC 8707 Resource Indicators through repeatable `resource` parameters on authorize, PAR, and token requests.

Discover resource URIs from catalog and user-service responses. A service's canonical resource URI is:

```text
<NYXID_BASE_URL>/api/v1/proxy/s/<service-slug>
```

Examples:

```text
GET /oauth/authorize?...&resource=https%3A%2F%2Fnyx.example%2Fapi%2Fv1%2Fproxy%2Fs%2Fllm-openai

POST /oauth/par
resource=https://nyx.example/api/v1/proxy/s/llm-openai
resource=https://nyx.example/api/v1/proxy/s/api-github

POST /oauth/token
grant_type=refresh_token
refresh_token=...
resource=https://nyx.example/api/v1/proxy/s/llm-openai
```

Validation rules:

- Each `resource` must be an absolute URI and must not include a fragment.
- The URI must identify a NyxID user service at `/api/v1/proxy/s/{slug}`.
- The slug must resolve to a service the user owns or can proxy through.
- Invalid or unknown targets fail with OAuth `invalid_target`.

Narrowing rules:

- Authorization can request one or more resources. If the user approves only some of them, the stored grant and issued code/token are narrowed to the approved services.
- Refresh-token and token-exchange requests may narrow to a subset of the original consent, but must never widen beyond it.
- If the stored grant is `allow_all_services = true`, a token request with `resource` narrows the issued access token to those resolved services.
- If the stored grant is service-specific, a token request with `resource` must resolve entirely inside the stored allowlist.
- A token request without `resource` keeps the refresh token's stored service boundary.

OAuth token responses include `resource` when the issued access token is resource-scoped.

## Managing existing consents

Use `/settings/consents` in the web UI for the clearest user-facing view. The page is `Access & Authorizations`:

- `Authorized Apps` shows OAuth consents.
- `Authorizations` shows OAuth broker bindings. See `oauth-broker.md`.

For each authorized app, the UI shows scopes, grant time, expiry, and service access:

- `All services`
- `No services`
- Resolved service labels/catalog names
- `Legacy grant` for legacy unrestricted rows

The CLI can list and revoke consents:

```bash
nyxid profile consents --output json
nyxid profile revoke-consent <client_id> --yes
```

Use `--output json` when answering "which services can this app access?" because the table view currently shows only client, app name, scopes, and grant time.

There is no direct edit-in-place surface for a consent service allowlist yet. To restrict or expand an app today:

1. Re-run the app's OAuth flow with `prompt=consent`, then use `Customize` to select the desired services.
2. Or revoke the consent and sign in again, choosing the desired services on the consent screen.

If the app needs more services than its existing consent allows and it requests them through `resource`, NyxID prompts for consent again instead of silently widening the grant.

## Revocation effects

Revoking a consent:

- Deletes the consent row for that `(user, client)`.
- Revokes the client's refresh-token chain for that user.
- Revokes broker bindings for the same user/client.
- Forces the app to obtain fresh consent before it can refresh access or use brokered authorizations again.

Already-issued access tokens can keep working until they expire. The UI warns users about a tail of up to 15 minutes. Broker-issued access tokens are commonly shorter, but do not promise immediate invalidation unless the client also handles broker revocation webhooks.

## Quick answers

- "What services can this app reach?" -> Check `/settings/consents` or `nyxid profile consents --output json`; read `allow_all_services`, `allowed_services`, and `legacy_unrestricted`.
- "Why did approving grant no service access?" -> The user approved sign-in only because no service was selected and `All services` was off.
- "How does an app pre-request OpenAI?" -> The developer app declares the catalog slug in `default_service_catalog_slugs`; the user's matching service is preselected, but the user may remove it.
- "How does a client request one service programmatically?" -> Send the service's `resource_uri` as a repeatable RFC 8707 `resource` parameter.
- "Can refresh expand from OpenAI to GitHub?" -> No. Refresh/exchange can narrow only; widening requires a new interactive consent.
