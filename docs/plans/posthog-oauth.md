# PostHog OAuth implementation plan

Status: reviewed with the requested Fable 5.1 advisor, 2026-10-07. This is the
primary agent's implementation plan after reviewing that consultation; the
[full Fable response](posthog-oauth-fable-review.md) is preserved separately.
This document describes future work; PostHog is not yet a seeded NyxID OAuth connector.
Verified vendor behavior is recorded in [POSTHOG_OAUTH.md](../POSTHOG_OAUTH.md).
The current cloud connector changes are documented in
[CLOUD_PLATFORM_OAUTH.md](../CLOUD_PLATFORM_OAUTH.md).

## Proposed scope

Keep the existing authorization-code engine. Add explicit public OAuth client
authentication, then publish a managed PostHog Client ID Metadata Document
(CIMD) and bind each connection to its trusted US or EU API destination.
Dynamic client registration, `private_key_jwt`, and arbitrary runtime discovery
are separate features and are not prerequisites for the recommended CIMD flow.

Cloudflare, Supabase Management, and Railway use registered confidential apps.
Their permission and consent differences fit existing provider configuration:
app-configured permissions for Supabase, app permissions and account selection
for Cloudflare, and resource roles plus `prompt=consent` for Railway.

## PR 1: explicit public-client authentication

Add `none` alongside `client_secret_basic` and `client_secret_post` in provider
validation. Retain the stored default of `client_secret_post` for older rows;
do not reinterpret an absent secret as a public client. `credential_mode`
continues to identify who supplies the app (`admin`, `user`, or `both`), while
the token authentication method identifies how that app authenticates.

Use one small shared policy/helper for required credential fields and token
request authentication in the service layer (`oauth_flow.rs` or a small shared
OAuth client-auth module). Keep Mongo models as plain serde data; Fable's
suggested model method is placed in services to follow this repo's layering
rule. A public client requires a client ID and S256 PKCE,
sends `client_id`, and sends neither a secret nor a Basic Authorization header.
Basic and POST clients continue to require a nonempty secret. Reject invalid
authentication methods and invalid public-client/PKCE configurations before
redirecting or sending a token request. Handle secret removal explicitly rather
than inheriting an old secret after a switch to `none`: reject submitted or
stored secrets for this method, and require deliberate clearing when changing
an existing confidential provider. A three-state token-auth helper must never
emit a secret or Basic header for `none`, even for an inconsistent stored row.

Update these paths together:

| Area | Existing code to change |
| --- | --- |
| Provider creation, updates, readiness | `handlers/providers.rs`, `services/provider_service.rs`, `services/user_credentials_service.rs::provider_has_admin_oauth_credentials` |
| User and connection credential resolution | `services/user_credentials_service.rs`, including shared, legacy user, claimed-key, and connection resolution |
| Modern BYO key creation and copying | `services/unified_key_service.rs::OauthClientCredentialsInput` and `resolve_oauth_client_credentials_input`; raw and `copy_oauth_client_from` must permit ID-only public clients |
| Key request parsing and persistence | Key handlers and `services/user_api_key_service.rs`; validate against the chosen provider rather than requiring every OAuth client to be a pair |
| Initiation, callback, modern refresh | `services/user_token_service.rs`, including `ensure_oauth_provider_configured` and both callback/refresh stores |
| Legacy refresh and shared wire encoding | `services/oauth_flow.rs` |
| Revocation | `services/oauth_revocation.rs`; inherited public authentication must use client ID without a secret, with explicit vendor revocation overrides retained |
| UI capability and validation | Provider/catalog response types, `frontend/src/types/api.ts`, `frontend/src/types/keys.ts`, `frontend/src/schemas/providers.ts` |
| BYO submission and settings | `frontend/src/components/dashboard/add-key-dialog.tsx`, user credential dialog, provider editors, and assistant credential forms |
| CLI and CLI wizard | `cli/src/commands/service.rs` and `frontend/src/components/cli-wizard/auth-flows.tsx`; derive the secret requirement from the catalog's explicit token-auth method |

In particular, `buildCatalogKeyParams` currently includes BYO credentials only
when both client ID and secret are present. Change this with the form validation
so an ID-only public BYO client stays BYO throughout creation, copying, callback,
refresh, reconnect, and disconnect. It must never silently use the shared app.
Validate every credential source with the same policy. Preserve encrypted
storage, redacted debug output, owner checks, and refresh revision fences.

Tests should exercise public/shared and public/BYO flows through real Mongo
state and a mock OAuth server, including two independent BYO connections,
credential copying, callback claims, refresh rotation, and remote revocation.
Assert that `none` emits no secret or Authorization header, that S256 verifier
matching works, and that confidential clients still reject missing secrets.
Keep the existing Basic/form, POST/form, legacy JSON, device-code, and cloud
connector protocol tests passing. Background refresh must preserve
`credential_epoch`; explicit client/credential replacement must follow the
existing replacement fence.

## PR 2: managed CIMD and scope policy

Publish a public JSON metadata document at a stable HTTPS URL controlled by the
deployment. That URL is the managed client ID. The document contains only public
client metadata: app name/logo, exact callback allowlist, the public token auth
method, and explicit required/optional scope lists. The callback must match the
backend's effective `BASE_URL` plus `/api/v1/providers/callback`; never construct
it from an untrusted request Host header or a caller-provided redirect.

Use Fable's proposed public route shape,
`/api/v1/oauth-clients/posthog/client-metadata.json`, beside the existing public
catalog-spec route. Keep the managed client ID an explicit operator setting;
do not seed a derived URL that silently replaces its identity after `BASE_URL`
changes. Report a mismatch between the configured ID and published URL.

Use a nonempty required list and an explicit optional list under `com.posthog`.
Use a small required permission such as `project:read` after confirming CIMD
grantability, with `insight:read` and `dashboard:read` optional for the initial
read operations. Add `query:read` or `feature_flag:read` only when those features
ship. NyxID's managed scope allowlist must stay inside the published ceiling.
Returned grants, including missing optional permissions, are authoritative for
feature availability.

Define the required/optional sets once in an audited metadata policy registry
and reuse it for publication, seeded defaults, the scope menu, and the managed
allowlist. Keep the metadata handler independent of mutable database defaults;
provider scope edits must be validated against this policy. This resolves an
ambiguity in the advisory response, which proposed both a static handler and
deriving its scope document from an editable provider record. Enforce the
managed client's required scopes before redirecting.

Use `Cache-Control: public, max-age=300` and document the propagation window.
PostHog preserves stored scope values when a field is omitted during metadata
refresh; use an explicit `optional_scopes: []` to clear them. Scope expansion
still needs user consent. Plan client ID/redirect changes as reconnection or
versioned identity changes rather than silently reassigning existing refresh
tokens to a new client. Retain the old client metadata URL for grants that
still use it until those grants are retired. Fence the issuing public client
identity and authentication method across initiation, callback, refresh, and
revoke. Audit existing platform/BYO provenance first; if a live provider lookup
can replace that identity, add a version/fingerprint check with a reconnect
error or a retained encrypted identity snapshot. Test a client-ID change during
an in-flight consent flow and during refresh. Secret rotation with an unchanged
confidential client ID remains a separate existing operation.

PostHog fetches the hosted metadata. NyxID does not need a general server-side
metadata fetcher for the managed client. If BYO metadata inspection is added,
use the existing bounded, DNS-pinned, redirect-safe fetch path and validate the
HTTPS URL; do not introduce a new arbitrary URL fetch path. Check the vendor's
accepted metadata fields with a test client before finalizing the JSON shape.

A new metadata handler belongs in `handlers/`, its configuration/policy in
`services/`, and its route in the existing router. The document needs public
access because the vendor fetches it without a NyxID session. It must expose no
credentials or owner metadata. Add protocol-shaped GET routes to delegated-read
deny rules if they would otherwise enter authenticated management routing.

Acceptance: the production-style URL serves JSON over HTTPS without login, the
callback and scope ceiling match initiation, secretless shared readiness works,
and redirect/scope edits have documented cache and reconnect behavior.

## PR 3: region binding and PostHog catalog connection

Use the private API hosts `https://us.posthog.com` and
`https://eu.posthog.com`. The `.i.posthog.com` ingestion hosts do not provide
this private API surface. Treat region as a finite selection mapped by the
server to those exact trusted origins, not as a free-form downstream URL.

Use Fable's proposed single `posthog` provider and `api-posthog` service, with
`requires_gateway_url: true` reusing the existing Supabase endpoint mechanism.
Offer a US Cloud / EU Cloud picker and map it to a trusted per-connection URL;
the CLI can accept the same allowlisted origins through `--endpoint-url`.
Keep the documented region-agnostic OAuth endpoints on the provider. Validate
actual token/region binding and cross-region error behavior with real accounts;
do not hardcode an assumed 401 before that check. The region-agnostic OAuth
domain's discovery response must not be treated as the region of a particular
user's grant. Automatic region detection remains conditional on evidence from
the actual flow; do not probe both resource hosts with a user's token to guess.

Add the exact-host rule to
`user_endpoint_service::normalize_catalog_endpoint_url` and ensure both key
creation and `normalize_endpoint_url_for_update` apply it. Reject HTTP,
nondefault ports, arbitrary paths, userinfo, query, fragment, unknown hosts, and
ingestion hosts. Preserve the existing destination/SSRF validation after this
narrower rule. Do not allow the service's placeholder catalog URL to become an
execution destination when region selection is missing.

Reuse the existing connection endpoint destination instead of mutating a global
catalog URL when one user connects EU. Pin the region before consent, retain it
through refresh/reconnect, and apply the existing personal/org ownership checks.
Any later destination change must be explicit and invalidate pending exact
approvals through the existing execution-authority digest, which already binds
the resolved destination. Preserve SSRF checks and prevent cross-region
credential forwarding through redirects or arbitrary host overrides.

Seed the provider, service, scoped read overlays, branding, and scope catalog
only after public-client support is deployed on every serving replica. Reuse
the wizard's trusted region choice. Do not default existing credentials to a different region during
startup. Self-hosted PostHog OAuth is outside this first connector; supporting
its different authorization/token hosts is a separate extension.

Acceptance: separate US/EU connections and independent BYO clients work together,
their proxies resolve the correct trusted hosts, refresh retains region and
client provenance, and a changed destination fails pending approval redemption.

## Vendor validation before activation

Use test US and EU organizations to validate:

1. CIMD acceptance and exact HTTPS callback matching with S256 PKCE.
2. Actual granted scopes, optional-scope decline, and required-scope filtering.
3. Token exchange/refresh/revocation request authentication and response fields.
4. A private project/insight/dashboard read through each NyxID regional proxy.
5. Refresh rotation, reconnect, disconnect, and invalid/expired refresh handling.
6. Metadata cache propagation and behavior after scope/client identity changes.
7. Region mismatch handling without sending a token to an unselected host.

Publisher verification is optional, separate for US and EU, and asks for HTTPS
redirects on the same production domain as the metadata. It is an operational
step after the protocol works, not a blocker for local implementation.

## Rollout and unresolved decisions

Deploy the backward-compatible public-client capability before enabling any
`none` provider. Older replicas reject writes configuring that method and
report an ID-only shared OAuth app as unconfigured, so keep PostHog inactive
until all serving replicas are upgraded. Enable the managed metadata route,
complete real-vendor validation, then activate the connector. Rollback disables
new PostHog connects while preserving encrypted grant records for supported
replicas or deliberate remote revocation.

Fable recommends keeping the cloud trio's protocol configuration and validating
two vendor-specific scope questions: whether Cloudflare's configured API
permissions are granted with the current default scope string, and whether
Supabase accepts omitting its deprecated `scope` parameter. Also record
Supabase's real `expires_in` behavior because an absent expiry prevents
proactive refresh. These checks are added to the cloud setup runbook.

Remaining PostHog checks are required-scope grantability, actual token region
binding, identity-change behavior, and the accepted CIMD document shape. A
personal PostHog API key can use the existing custom bearer-key connection as
an interim manual path. Fable also suggests an optional seeded
`posthog-key` / `api-posthog-key` connector sharing the same region validation
and overlay; it can ship independently if requested. Never substitute a
project ingestion key for private API access.
