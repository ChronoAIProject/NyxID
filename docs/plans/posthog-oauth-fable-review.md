# Fable 5.1 consultation: non-standard OAuth flows (Cloudflare / Supabase Mgmt / Railway review, PostHog CIMD public-client plan)

Read-only consultation. Nothing below was implemented. Line numbers refer to the current `dusk-island` worktree (uncommitted diff included). Legend: **[V]** verified in code or official docs, **[I]** inference from standards or code shape, **[?]** unresolved vendor behavior to validate against the real vendor.

## Context

The branch adds three downstream OAuth connectors (Cloudflare, Supabase Management, Railway) as confidential clients. PostHog is next, but its recommended registration is a Client ID Metadata Document (CIMD) public client (`token_endpoint_auth_method: none`), which the current provider/credential code cannot express: readiness, resolution, admin validation, the BYO pair rule, the CLI, and the frontend schema all assume client ID + secret. The user asked for a plan that supports PostHog without a framework rewrite and without dummy secrets.

---

## 1. Review of the three cloud integrations

None of the three is non-standard. All are authorization-code + S256 PKCE confidential clients that NyxID's generic path already handles.

**What is verified correct [V]**
- Basic client auth on exchange and both refresh paths when `token_endpoint_auth_method == "client_secret_basic"`: `backend/src/services/user_token_service.rs:1878-1908` (exchange), `:2550-2565` (multi-connection refresh), `backend/src/services/oauth_flow.rs:207-230` (legacy refresh). Cloudflare docs allow Basic or POST; Supabase docs require Basic; Railway docs use `-u`.
- PKCE S256 emitted when `supports_pkce` (`user_token_service.rs:887-891`, `:965-971`); all three advertise S256.
- Railway `prompt=consent` survives the `extra_auth_params` blocklist (`user_token_service.rs:976-1000`; `prompt` is not listed).
- Supabase `supports_oauth_scopes: false` omits `scope` and rejects additional scopes (`user_token_service.rs:266`, `:804-807`).
- Refresh-token rotation (Railway) is persisted by both refresh paths (returned `refresh_token` overwrites stored value). Absent `expires_in` is tolerated (`user_token_service.rs:1971-1980`).
- Cloudflare revocation `auth: inherit` resolves to Basic (`backend/src/services/oauth_revocation.rs:200-203`), matching the seeded method.
- New slugs are not in `SEEDED_USER_CREDENTIAL_OAUTH_PROVIDER_SLUGS` (`provider_service.rs:32`), so an ops `credential_mode` survives restart; the new seed test covers this.
- Platform-scope allowlist only applies to Railway (`scope_catalog.rs:111`); Cloudflare/Supabase have none, which is right because their permissions are client-configured.

**Required before calling these done (vendor validation, not code)**
- **Cloudflare scope semantics [?]**: discovery `scopes_supported` is only `offline_access, offline, openid`, yet the client docs say "scope names correspond to API token permission names" and are selected on the client. Unresolved whether a request carrying only `openid offline_access` yields a token with the client's configured permissions, or whether permission scope names must be enumerated in `scope` (wrangler enumerates them). Resolve in the smoke test. Mitigation needs no code: `default_scopes` is admin-editable and the picker accepts free-form scopes. Record the outcome in `docs/CLOUD_PLATFORM_OAUTH.md`.
- **Supabase `scope` omission [?]**: the docs mark `scope` deprecated but their example authorize URL still carries `scope=all`. Verify that omitting it does not fail. If it does, the fallback is admin-only: set `supports_oauth_scopes: true` and `default_scopes: ["all"]` by PUT; `scope` is in the `extra_auth_params` blocklist so that route is unavailable by design.
- **Supabase `expires_in` [?]**: undocumented. If absent, NyxID stores no `expires_at` and schedules no proactive refresh (Notion precedent, `docs/NOTION_OAUTH.md`). Confirm in the smoke test whether tokens expire silently; if they do and `expires_in` is absent, this needs a Notion-style note and a follow-up (not blocking).
- **Cloudflare shared-app prerequisites [V]**: a NyxID-managed client must be public (permanent) with DNS TXT publisher verification before non-member accounts can authorize. Ops runbook item, already documented.

**Optional improvements**
- `scope_catalog::removal_capability` defaults new slugs to `Manual` (`scope_catalog.rs:75-84`); Cloudflare has RFC 7009 revoke, so `Auto` is defensible after the revoke smoke test.
- Railway returns `iss` on the callback (RFC 9207); NyxID ignores it. Fine today; could be checked later as defense in depth.

Verdict: the three seeds are correct as confidential clients. The only real risk is the two scope questions, which the smoke checklist in §5 settles without code changes.

---

## 2. Smallest safe extension: an explicit public-client contract

**Contract.** Extend `token_endpoint_auth_method` with the RFC 7591 value `none`:
- `none` means: client identifies with `client_id` only; token, refresh, and RFC 7009 requests carry `client_id` in the body (RFC 6749 §2.3.1 / §4.1.3); never Basic; never `client_secret`.
- `none` requires `supports_pkce: true` (validation error otherwise).
- A stored or submitted client secret with `none` is rejected at write time (admin create/update, BYO upsert, `POST /keys`). No dummy secrets anywhere; `ResolvedOAuthCredentials.client_secret` is already `Option` (`user_credentials_service.rs:222-228`), and device-code providers already run id-only (`:236-241`), so `None` is an exercised shape.
- Default stays `client_secret_post`; every existing provider keeps requiring a secret because the requirement is derived from the method, not relaxed globally.

**Relation to `credential_mode` / shared / BYO [I]**
- `admin`: platform client must have `client_id`; secret required iff method != `none`.
- `both`: BYO (connection-embedded or legacy row) wins, else platform; readiness for the platform side follows the same rule, so `has_platform_oauth_credentials` becomes true for an id-only `none` provider with no other change (`catalog_service.rs:243` already derives from `provider_has_admin_oauth_credentials`).
- `user`/BYO on a `none` provider: user supplies their own CIMD URL as `client_id`, no secret. Niche for PostHog but free.

**Single source of truth.** Add `ProviderConfig::requires_client_secret(&self) -> bool` in `backend/src/models/provider_config.rs` (oauth2 and method != `none`; device_code false; api_key false). Use it in:
- `user_credentials_service::provider_has_admin_oauth_credentials` (`:230-243`): oauth2 arm becomes `client_id.is_some() && (!requires_secret || client_secret.is_some())`.
- `user_token_service::ensure_oauth_provider_configured` (`:384-403`): same predicate for admin mode.
- `handlers/providers.rs` create (`:396-399` enum, `:438-452` pair rule): admin requires secret only when `requires_client_secret`; the "both/user fallback pair" rule becomes "client_id required whenever a secret is given; secret required with client_id iff requires_secret; secret forbidden when method is none".
- `provider_service::update_provider` (`:6737-6745`): add `none`; cross-field check: setting `none` while a secret is stored (or submitted) is a validation error telling the admin to clear the secret first (the clear path exists, see test near `:10598`).
- `provider_service::create_provider*` input validation: same.

**Token requests.** Replace the two-state `use_basic_auth` with a tiny enum in `oauth_flow.rs`: `pub enum TokenClientAuth { Basic, Post, PublicClientId }` and `pub fn token_client_auth(provider) -> TokenClientAuth`. Apply at `user_token_service.rs:1878-1908` (exchange), `:2550-2565` (multi-connection refresh), `oauth_flow.rs:207-230` (legacy refresh). `PublicClientId` pushes `client_id_form_field` only and ignores any resolved secret defensively. Also check the device-code poll path uses the same helper if it builds client auth.

**Revocation.** `oauth_revocation.rs:200-203`: `inherit` with `none` maps to `Rfc7009Auth::ClientId`. Today `_ => Post` would still send only `client_id` when secret is `None`, so behavior is already right; making it explicit avoids relying on the absence of a secret.

**BYO inputs.**
- `unified_key_service.rs:455-470` validates the Raw pair before the provider is loaded and requires a non-empty secret. Make the secret `Option` in `OauthClientCredentialsInput::Raw` and enforce "secret required iff provider.requires_client_secret()" at the point where the provider is known (`:1254` area, where `provider_requires_byo` is computed). `user_api_key_service::create_api_key` already takes `Option` secret.
- `handlers/user_credentials.rs:156` already accepts an optional secret; add the forbid-secret-on-`none` check there.
- CLI `cli/src/commands/service.rs:523-531`: it already fetches `/catalog/{slug}` (`:690`); relax "id requires secret" when the catalog entry reports a public client.

**Catalog/API surface.** Add `oauth_token_endpoint_auth_method: Option<String>` to `CatalogEntry` (`catalog_service.rs` `build_catalog_entry`, `frontend/src/types/keys.ts:178-225`). `ProviderResponse` already exposes it (`handlers/providers.rs:194`).

**Frontend.**
- `frontend/src/schemas/providers.ts:147-149`: enum adds `none`; `superRefine`: admin mode requires secret unless `none`; `none` requires `supports_pkce`; secret present with `none` is an error.
- `frontend/src/pages/provider-edit.tsx:702`: show the method; label `none` as "Public client (PKCE, no secret)".
- `frontend/src/components/dashboard/add-key-dialog.tsx:2697` (`requireSecret`) becomes false for public-client entries; `:3240` must send `oauth_client_id` alone for public entries instead of dropping the unpaired id. Mirror in `frontend/src/components/cli-wizard/auth-flows.tsx`.

No provider-specific hooks are needed for `none`. The only PostHog-specific code is in §3 (CIMD route) and §4 (host allowlist), both as small match arms following the `api-supabase` / IFTTT precedents (`user_endpoint_service.rs:59`, `oauth_flow.rs:apply_token_resource`).

**Mixed-fleet note [I].** An old replica reading a `none` provider treats it as POST with `client_secret: None` and sends `client_id` only, so token traffic is compatible. But its old `provider_has_admin_oauth_credentials` returns false for id-only, so a managed PostHog connect on an old replica fails cleanly with "requires either admin-configured… or your own" rather than misbehaving. Deploy PR B to all replicas before enabling the PostHog client (PR C).

---

## 3. Managed HTTPS CIMD publication

**Route.** Public GET beside the catalog-spec route (`backend/src/routes.rs:1791` in `api_v1_public`): `/api/v1/oauth-clients/{provider_slug}/client-metadata.json`, handler modeled on `handlers/docs.rs:catalog_spec_json` (`:85-118`), backed by a static registry `backend/src/services/oauth_client_metadata.rs` (one entry: `posthog`). Unknown slugs 404. No auth, no DB.

**Document contents [V from PostHog docs].**
- `client_id`: exactly the route URL built from `config.base_url` (never the `Host` header).
- `client_name`: "NyxID"; `logo_uri`: a static asset under `frontend_url` (optional).
- `redirect_uris`: `[ "{BASE_URL}/api/v1/providers/callback" ]`, matching `user_token_service.rs:945-948` byte for byte (scheme, host, port, path). HTTPS required unless loopback.
- `com.posthog.scopes` (required, locked on consent) and `com.posthog.optional_scopes` (declinable). Together they are the ceiling. PostHog drops ungrantable scopes and rejects the document if every required scope is dropped, so keep `scopes` minimal (one scope such as `project:read` **[?]** confirm grantable) and put the rest in `optional_scopes`. Always publish both lists explicitly: on a metadata refresh an omitted field keeps its stored value.
- `Cache-Control: public, max-age=300`. PostHog caches by this TTL; redirect or scope changes propagate after it elapses.
- Startup WARN (no new env var) when `BASE_URL` is not https: the document will be served but PostHog will not accept it.

**Scope ceiling source.** Derive `scopes` from the posthog provider `default_scopes` and `optional_scopes` from `scope_catalog::platform_scope_allowlist("posthog")` minus defaults, so the server-side allowlist check at `user_token_service.rs:820-845` rejects out-of-ceiling requests before the redirect. Add a `scope_catalog::for_provider("posthog")` menu (read scopes first; writes flagged `sensitive`). Do not assume every scope in `scopes_supported` is grantable to CIMD clients **[?]**.

**Client identity control.** Store the metadata URL as the provider's `client_id` via the existing admin PUT (encrypted at rest is harmless; the value is public). Do not auto-derive it at seed time: a `BASE_URL` change would silently change the client identity. Add a startup WARN when the stored `client_id` differs from the URL the registry would serve for the current `BASE_URL`. Treat a client-id change as a migration, not a rotation (same rule as `docs/ONE_CLICK_OAUTH_CONNECTORS_SPEC.md` D3): refresh tokens are bound to the issuing client **[I from OAuth semantics; ? for PostHog specifically]**, so existing connections must reconnect.

**Reconnect semantics.** Reconnect threads the same `connection_id`, so `credential_source = "platform"` keeps resolving the provider client (`user_token_service.rs:759-778`); nothing new.

**Deferred.** `private_key_jwt`: not required for CIMD, needs key management and JWKS publication; defer. Dynamic client registration (`registration_endpoint`): PostHog recommends CIMD, DCR needs a registration access token store and creates a client per instance; defer. Publisher verification: optional, only removes the "unverified" banner; needs the production HTTPS domain; request after smoke. `com.posthog.verification_token`: optional, raises provisioning limits; could later be an admin-settable provider field, not an env var.

---

## 4. Regional routing and per-connection destination binding

**Model [V reuses existing mechanisms].** Mark the `posthog` provider `requires_gateway_url: true` and seed `api-posthog` with a placeholder `base_url`, exactly like `api-supabase` (`provider_service.rs:664-716`, `:3550-3580`). Then:
- `POST /keys` requires `endpoint_url` (`unified_key_service.rs:1121-1124`); it is stored on the per-connection `UserEndpoint`, which is what the proxy uses (`proxy_service.rs:2910`, `:3014`) and what exact execution authority binds as `destination_base_url` (`execution_authority.rs:66,106`). Ownership, approvals, and node routing are untouched.
- OAuth endpoints stay region-agnostic on `ProviderConfig` (`https://oauth.posthog.com/oauth/{authorize,token,revoke}/`), per PostHog docs.
- Refresh retention is unaffected by region: tokens live on `UserApiKey` and refresh at the region-agnostic token endpoint.

**Trusted host mapping.** Add an `api-posthog` arm to `user_endpoint_service::normalize_catalog_endpoint_url` (`:53-90`): accept only `https://us.posthog.com` or `https://eu.posthog.com` (optional trailing slash), normalize to origin-only, reject everything else, including `*.i.posthog.com` ingestion hosts, http, paths, userinfo, query, fragment. The existing `validate_user_endpoint_url` (`url_validation.rs:136-180`, public-host check in hosted mode) still runs afterwards (`unified_key_service.rs:1137`); the allowlist is strictly narrower. Self-hosted PostHog is deferred: it would need per-connection authorization/token URLs, which `ProviderConfig` does not model.

**UX.** Region picker (US Cloud / EU Cloud, default US) in the connect dialog, following the `isSupabase` branch (`add-key-dialog.tsx:747`, `:991-1000`) and `ai-key-confirm-panel.tsx:1201`; CLI `nyxid service add api-posthog --oauth --endpoint-url https://eu.posthog.com`. Copy should say "pick the region you log into (us.posthog.com or eu.posthog.com)"; a wrong region shows as 401 from PostHog.

**Unknowns to validate with real PostHog [?]**
1. Does a token minted via `oauth.posthog.com` work only on the user's home region host, and what does a cross-region call return?
2. Does the token response or `userinfo` expose the region (discovery's `posthog_region` is server-side, not per user)? If yes, a later post-callback probe could auto-select or verify the region.
3. Are `openid`, `profile`, `email` grantable to CIMD clients?
4. `oauth.posthog.com/.well-known/oauth-protected-resource` returned 404; check the regional URL and whether PostHog's 401 carries `WWW-Authenticate: resource_metadata`.
5. Does PostHog bind refresh tokens to the CIMD `client_id` (expected yes)?

---

## 5. Phased plan, tests, smoke checklist, acceptance

**PR A (in flight): cloud trio.** Keep as is. Add the §1 vendor notes to `docs/CLOUD_PLATFORM_OAUTH.md` after smoke. Optional: Cloudflare `removal_capability` → `Auto`.

**PR C0 (optional, can ship before B): PostHog personal API key fallback.** Fits the `api-github` / `api-github-pat` precedent: `posthog-key` `api_key` provider + `api-posthog-key` service, bearer injection, `requires_gateway_url: true`, same `normalize_catalog_endpoint_url` arm, overlay shared via `SLUG_TO_SPEC_KEY` (`catalog_spec_registry.rs:170-215`). Zero framework change, immediate value, and it exercises the region allowlist and overlay before OAuth lands.

**PR B: public-client contract (`none`).** Files/functions in §2. Tests:
- Unit: `requires_client_secret`; readiness true for id-only + `none`, false for id-only + post/basic; validation rejects secret with `none`, rejects `none` without PKCE; update refuses `none` while a secret is stored.
- Protocol (wiremock, modeled on `cloud_oauth_connect_and_refresh_use_seeded_protocols`): shared and BYO initiation with id-only; exchange and refresh requests have no `Authorization` header, `client_id` in the form body, no `client_secret`, `code_verifier` present and matching the challenge; revocation sends `client_id` only.
- Regression: `oauth_refresh_contracts_cover_both_stores_and_legacy_encodings` and the cloud-trio test unchanged; every seeded `client_secret_basic`/`post` provider still fails readiness without a secret.
- Frontend vitest: schema accepts `none` only with PKCE and no secret; dialog sends `oauth_client_id` alone for public entries and still requires the pair for secret providers.

**PR C: PostHog CIMD connector.** Files: `services/oauth_client_metadata.rs` (new), handler + `routes.rs` public route, `provider_service.rs` seeds (`posthog` provider: oauth2, `credential_mode: both`, `token_endpoint_auth_method: none`, `supports_pkce: true`, form encoding, region-agnostic URLs, RFC 7009 revoke with `auth: inherit`, `requires_gateway_url: true`; `api-posthog` service with placeholder base URL), `catalog_spec_registry.rs` + `backend/specs/catalog/posthog.openapi.json` (read-first overlay), `scope_catalog.rs` (menu + allowlist), `user_endpoint_service.rs` (host arm), dialog/panel region picker, `provider-branding.ts`, `docs/POSTHOG_OAUTH.md` runbook. Tests: metadata document shape (client_id equals the route URL from `BASE_URL`, exact redirect, scope lists ⊆ allowlist, `Cache-Control`), unknown slug 404, seed idempotency across two startups with an ops-set client id, host normalization accept/reject table, wiremock end-to-end for shared id-only connect/refresh, allowlist rejection of out-of-ceiling scopes.

**Rollout order.** A → (C0) → B on every replica → C → ops PUT of the client id → smoke → optionally request PostHog verification.

**Backward-compatible defaults.** Method default unchanged; `none` only when explicitly set; existing secret providers unchanged; new catalog field optional; CLI only relaxes when the catalog says public.

**Real-vendor smoke checklist**
- Cloudflare (private client, member account): consent lists permissions; `GET /accounts` succeeds; settle the `scope` question (§1); refresh; RFC 7009 revoke returns 200 and the token stops working.
- Supabase: authorize without `scope`; record whether `expires_in` is present; `GET /v1/organizations`; refresh.
- Railway: consent with project selection; `externalWorkspaces` query; refresh rotates and the old refresh token is rejected; `workspace:admin` refused on the shared app, allowed on BYO.
- PostHog: PostHog fetches the metadata URL (observe in access logs); consent shows name/logo and the unverified notice; granted scopes equal the ceiling choices; `GET /api/projects/` on the chosen region works; cross-region call behavior recorded; refresh; revoke; change `max-age`-bounded metadata (add an optional scope) and confirm propagation after TTL; reconnect with a wider optional scope; BYO with a user-hosted metadata URL.

**Acceptance criteria**
- No code path can send Basic or `client_secret` for a `none` provider; no path accepts a stored secret with `none`.
- All existing secret-authenticated providers still fail readiness without a secret (test-enforced).
- `has_platform_oauth_credentials` is true for an id-only `none` provider and the dialog offers one-click connect.
- PostHog connections bind a per-connection regional destination that only accepts the two cloud hosts; execution authority digests include it unchanged.
- Metadata document is byte-stable for a given `BASE_URL` and never derived from request headers.
- Smoke checklist recorded in docs with the five §4 unknowns answered.

---

## 6. Recommendation

Ship PR A as is, then PR B (the `none` contract via one `requires_client_secret` helper and a three-state token-auth enum) before any PostHog seed, since mixed fleets fail closed but fail. Ship PR C0 (personal API key connector) early if product wants PostHog value now; it reuses the region allowlist and overlay. Land PR C last, with the client id set by ops PUT, and resolve the five PostHog unknowns and the two Cloudflare/Supabase scope questions in the smoke pass before announcing. Defer `private_key_jwt`, dynamic registration, self-hosted PostHog, and publisher verification.
