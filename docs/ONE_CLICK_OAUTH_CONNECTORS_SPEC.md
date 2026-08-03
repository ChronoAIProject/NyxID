# One-Click OAuth Connectors — GitHub & Google (Spec)

Status: v2 implemented on this branch — 2026-07-21 (PM: Fable). v1 reviewed by
Codex (consult session `019f82fc`); all blockers and corrections incorporated
below, tagged `[v2]`. Code changes: B1-B6 (backend), F1-F4 (frontend), with
B7/F4 test coverage. Remaining before prod one-click: the ops runbook in §5
(app registration + staged provisioning) and Phase 2 Google verification.
Branch: `avatar-connector-specs`
Related: NyxID#1211 (assistant authorization blockers), NyxID#917 (OAuth scopes), [assistant action cards](chat/04-action-cards.md)

## 1. Goal

A user (in the dashboard or from an assistant-chat connect card) clicks **Connect GitHub**
or **Connect Google**, is redirected to the provider's own consent screen, clicks
**Authorize**, and the connector is live — no API key pasted, no developer OAuth app
registered, no client ID/secret form. Same trust shape as "Sign in with Google", but for
connector brokering.

`[v2]` Note on mechanics: the current dialog performs a **full-page redirect**
(`window.location.href`, `add-key-dialog.tsx:1354` via `lib/navigation.ts`), not a popup.
All UX copy, tests, and the callback-recovery flow assume full-page redirect + return.

Today only `openai-codex` achieves one-click (the sole provider with a platform-shipped
OAuth client, `provider_service.rs:151-187`). GitHub and Google are `credential_mode:
"user"` (bring-your-own OAuth app). This spec promotes both to platform-app connectors.

`[v2]` Scope discipline: this spec covers **GitHub and Google only**. Promoting further
providers is NOT "pure ops" — each needs a compatibility checklist pass (authorization
params, token response shape, token-endpoint auth method, refresh behavior, scope
semantics, revocation, PKCE, callback restrictions, tenant requirements) even though no
model/route changes are expected.

## 2. Current state (verified against code; corrected in v2)

| Fact | Where |
|---|---|
| `github` provider: `oauth2`, `credential_mode: "user"`, no platform creds, scopes `read:user user:email`, `supports_pkce: false`, `client_secret_post` | `provider_service.rs:622-655` |
| `google` provider: `oauth2`, `credential_mode: "user"`, no platform creds, scopes `openid email profile`, `supports_pkce: true`, `access_type=offline&prompt=consent` | `provider_service.rs:574-612` |
| **`[v2]` Startup is NOT insert-only for these rows**: `seed_default_providers` also runs `social_user_mode_migration`, which force-rewrites `credential_mode` back to `"user"` on every startup for all slugs in `SEEDED_USER_CREDENTIAL_OAUTH_PROVIDER_SLUGS` — a list that includes `google` and `github`. Any ops PUT to `"both"` today is silently reverted at next restart. | `provider_service.rs:25-38,551-565` |
| Linked catalog services: `api-github`, `api-google` | `provider_service.rs:2119-2138` |
| Scope catalogs for both exist (#917); GitHub `ScopeRemoval::Unsupported`, Google incremental. **`[v2]` The catalog is advisory only**: the UI submits any selection as `scope_override` and the backend accepts arbitrary scopes; the `sensitive` flag is a display hint, not Google's verification classification, and nothing gates sensitive scopes server-side. | `scope_catalog.rs:74-94,242+`, `add-key-dialog.tsx:1296` |
| `credential_mode: "both"` = user BYO wins, else platform creds, else clear error | `user_credentials_service.rs:197-239` |
| **`[v2]` BYO provenance gap**: when initiation resolves legacy provider-level `UserProviderCredentials` (BYO), the multi-connection callback stores only tokens on the `UserApiKey` — it does not embed the BYO client creds. Later `refresh_user_api_key_in_place` uses key-embedded creds or *current platform creds*, never the legacy BYO row → a BYO-authorized token can be refreshed with the wrong client (`invalid_client`/`invalid_grant`). | `user_token_service.rs:622,2056`, `user_api_key_service.rs:423` |
| **`[v2]` Platform tokens are not client-pinned**: `credential_user_id: None` means "use whatever is on `ProviderConfig` now", so replacing the platform client ID breaks all outstanding refresh tokens. Same-client secret rotation is the only in-place-safe rotation. | `user_credentials_service.rs:240+`, `user_token_service.rs:2056` |
| Admin provider-update API accepts `client_id` / `client_secret` / `credential_mode`, encrypted at rest, redacted Debug. **`[v2]` But**: it accepts empty strings, cannot unset/clear a credential (`Option<String>` + `$set`-when-present), and keeps no previous version. | `handlers/providers.rs:23-102` |
| Single generic redirect URI: `{BASE_URL}/api/v1/providers/callback` | `user_token_service.rs:680-684`, route `routes.rs:204-206` |
| **`[v2]` Platform-client signal already exists (partially)**: catalog responses expose the *decrypted platform `oauth_client_id`* whenever `credential_mode != "user"` (`decrypt_provider_client_id`), and `ProviderResponse` has `has_oauth_config` (via `provider_has_admin_oauth_credentials()`). Neither proves the secret is present/decryptable for a catalog consumer, and two parallel signals can disagree. | `catalog_service.rs:183+,328`, `handlers/providers.rs:131,200`, `user_credentials_service.rs:176` |
| Frontend: `AddKeyDialog` shows the BYO Custom-App form whenever `credential_mode` is `"user"` **or `"both"`** — both the forward branch and the OAuth/device-code `onBack` handlers route through `oauth_credentials`. | `add-key-dialog.tsx:2285-2286,2678` |
| `[v2]` `/keys` recognizes `tab`, `slug`, `action`, `service` — there is **no `connect` param** (TanStack `validateSearch` strips unknown params); `slug` alone opens the dialog. | `router.tsx:618`, `keys.tsx:816` |
| Existing `GOOGLE_CLIENT_ID` / `GITHUB_CLIENT_ID` envs serve social login only (callback `/social/{provider}/callback`) | `config.rs:762-764`, `social_token_exchange_service.rs:206,288` |
| `[v2]` The generic OAuth callback is intentionally usable without a browser session: possession of a valid, unexpired, one-time `state` is the trust anchor. "Human-only" applies to *initiation* (the `/keys`/`/providers` mounts), not the callback itself. | `handlers/user_tokens.rs:465+` |

## 3. Product decisions

### D1. `credential_mode: "both"`, not `"admin"` — with explicit credential provenance `[v2 amended]`

`"both"` keeps every current BYO setup working while making the platform app the
zero-setup default. **v2 addition:** BYO precedence must be made *durable*, not just
resolved at initiation. Decision: **embed resolved BYO client credentials onto the new
`UserApiKey` at connection creation** (Codex option 1) — when
`resolve_oauth_credentials` selects legacy provider-level BYO creds for a
multi-connection add, copy them into `user_oauth_client_id_encrypted` /
`user_oauth_client_secret_encrypted` on the key (the mechanism the Lark BYO path
already uses). Refresh then always uses the client that authorized. Reconnect reuses
the key-embedded creds (preserving the original credential source) and never silently
switches a BYO connection to the platform client.

### D2. Separate OAuth apps for connector brokering vs social login

Register **new** GitHub and Google OAuth apps for the broker, distinct from the
social-login apps behind `GOOGLE_CLIENT_ID`/`GITHUB_CLIENT_ID`:

- Different redirect URIs (`/api/v1/providers/callback` vs `/api/v1/auth/social/...`).
- Different scope posture: login apps stay minimal forever; connector apps will grow
  incremental scopes and carry the associated review burden.
- Blast-radius isolation: a connector-app secret rotation or provider-side suspension
  must not break login.

### D3. Provisioning is runtime ops (admin API), never source code — with a staged sequence `[v2 amended]`

Client secrets must not appear in the repo (Critical Rule 5). Provisioning is an admin
PUT, stored AES-envelope-encrypted. **v2 additions** (from review):

- **Staged enablement**: PUT credentials while the provider is still effectively
  BYO-only → verify with an admin test connection → only then flip `credential_mode`
  to `"both"`. (Requires the validation hook in B5; until it exists, the verifying
  admin performs a real connect against their own account as the smoke test.)
- **Rotation policy**: same-client secret rotation is supported in place. **Client-ID
  replacement is a migration, not a rotation** — outstanding platform-issued refresh
  tokens are not client-pinned and will break. Replacing the app requires a
  reauthorization campaign; do not PUT a new client ID over a live one. Also avoid
  rotating mid-flight: an OAuth state lives 10 minutes, and initiation + callback each
  read current `ProviderConfig`, so rotation between them fails the exchange.
- **Rollback**: record prior values (out-of-band) before any PUT; B5 adds an explicit
  clear/unset operation and empty-string rejection so incident response can fully
  remove a compromised credential. Provider-side revocation ordering: deploy the new
  secret in NyxID *before* revoking the old secret at GitHub/Google.

### D4. Backend change is REQUIRED; there is no ops-only path `[v2 rewritten]`

v1 claimed seeding was insert-only and prod could be migrated by PUT alone. **Both
claims were wrong**: `social_user_mode_migration` reverts `google`/`github` to `"user"`
on every startup. Therefore:

- Remove `"google"` and `"github"` from `SEEDED_USER_CREDENTIAL_OAUTH_PROVIDER_SLUGS`
  (and update that migration's tests) so ops updates stick.
- Flip the two seed literals to `"both"` for fresh installs.
- Required tests: (a) a fresh install stays `"both"` across two startup runs; (b) an
  ops-patched existing row stays `"both"` after restart.

### D5. Default scopes minimal; platform-client scope allowlist enforced server-side `[v2 amended]`

- GitHub: default `read:user user:email`; Google: default `openid email profile`.
- **v2:** "Phase 2 gating" must be real. Add a **server-side per-provider scope
  allowlist that applies only when the flow resolves platform credentials**: requested
  scopes (defaults + additions + `scope_override`) must be a subset of the allowlist or
  initiation fails with a clear error. Launch allowlists: google = `openid email
  profile`; github = `read:user user:email` (+ `repo` if product wants it at launch).
  BYO flows keep today's free-form behavior (the user consents on their own app).
- The scope catalog's `sensitive` flag is a display hint and MUST NOT be used as the
  verification gate — Google's sensitive/restricted classification is a separate,
  manually maintained list (it does not match our `sensitive` booleans; e.g. Drive
  read-only is verification-relevant to Google).
- GitHub caveat: no scope removal (`ScopeRemoval::Unsupported`); re-auth mints supersets.

### D6. The chat connect card gets true one-click for these two providers

A #1211 blocker for `api-github` / `api-google` opens `AddKeyDialog` with the service
preselected, which now goes straight to the provider redirect. Deep link `[v2
corrected]`: `/keys?tab=services&slug=api-github` (there is no `connect` param; `slug`
alone opens the dialog).

### D7. One authoritative capability signal `[v2 new]`

Today the catalog exposes a decrypted platform `oauth_client_id` (non-`user` modes) and
providers expose `has_oauth_config` — two partial, disagreement-prone signals. Decision:

- Add `has_platform_oauth_credentials: bool` to catalog entries, derived via the
  existing `provider_has_admin_oauth_credentials()` helper (single source; not ad-hoc
  `is_some()` checks). Named "credentials", not "client", and documented as
  "ciphertext present", not "verified working" — it cannot prove decryptability,
  provider-side app health, or callback correctness (see B5/monitoring).
- Keep the existing `oauth_client_id` catalog field as-is for compatibility (client IDs
  are public by nature — they appear in redirect URLs); document it as display-only and
  non-authoritative. Removing it is a separate cleanup decision, not bundled here.

## 4. UX flow (target)

### Dashboard (`/keys` → Add service → GitHub)

1. User picks GitHub from the catalog (or deep link `/keys?tab=services&slug=api-github`).
2. Dialog shows: service summary, default scopes (pre-checked), allowlisted optional
   scopes, one primary button **"Connect GitHub"**. No client-ID form.
   A collapsed secondary action: **"Use your own OAuth app"**.
3. Click → full-page redirect to the provider consent screen (CSRF `state`; PKCE for
   Google).
4. Authorize → provider redirects to `/api/v1/providers/callback` → token exchanged,
   encrypted, stored; UserEndpoint + UserApiKey + UserService auto-provisioned.
5. User returns to `/keys`; the connection shows **Connected**; service proxy-ready at
   `{BASE_URL}/api/v1/proxy/s/api-github`.

### `[v2]` Dialog state machine (F1/F2 made explicit)

- Forward: for `"both"` + `has_platform_oauth_credentials=true`, route directly to the
  OAuth step (skip `oauth_credentials`). For `"user"`, or `"both"` without platform
  creds, keep today's BYO form.
- Secondary: OAuth step → "Use your own OAuth app" → `oauth_credentials` form; the form
  gains "Use NyxID's app instead" which **clears cached BYO state**
  (`byoOAuthClientId/Secret`) and returns to the OAuth step.
- Back-nav: the OAuth/device-code `onBack` handlers (`add-key-dialog.tsx:2678`)
  currently return every `"both"` provider to `oauth_credentials`; they must mirror the
  forward branch (skip the form when the platform path was taken), otherwise Back
  reveals a form the user never saw.
- Reconnect: bypasses forward routing (existing key passed straight to `OAuthStep`) —
  must preserve the key's original credential source (key-embedded BYO creds if
  present, else platform), never silently switch clients.
- Error path, two distinct failure points: (a) *initiation fails NyxID-side*
  (creds removed/corrupted between catalog fetch and click, scope rejected) —
  the backend's typed message is shown in-dialog with the BYO secondary action
  available beneath it. (b) *Provider-side-invalid client* (revoked/unregistered
  ID, wrong callback): initiation cannot detect this — it validates ciphertext
  presence only — so the user lands on the provider's own error page (e.g.
  GitHub's 404) and recovers by navigating back; the pending placeholder is
  cleaned up by the existing pending-auth cleanup path. The ops smoke test
  (runbook step 4) is the control that keeps (b) rare.
- Mode-switch cleanup: if the user switches platform ↔ BYO after a pending placeholder
  connection was created, the placeholder is cleaned up (delete or reuse), not leaked.

### Assistant chat

Connect card (per #1211) → same dialog. After connect, the user retries; the new turn
uses a fresh `clientRequestId`.

## 5. Engineering changes

### Backend `[v2 expanded — no longer "minimal"]`

| # | Change | Files |
|---|---|---|
| B1 | `has_platform_oauth_credentials: bool` on catalog entries (and provider list if needed — note: `GET /providers` is a shared, not admin-only, surface; the boolean is safe for it), derived via `provider_has_admin_oauth_credentials()`. | `handlers/catalog.rs`, `services/catalog_service.rs`, `handlers/providers.rs` |
| B2 | Remove `google`/`github` from `SEEDED_USER_CREDENTIAL_OAUTH_PROVIDER_SLUGS`; flip their seed literals to `"both"`; update migration tests. | `services/provider_service.rs` |
| B3 | Embed resolved legacy BYO client creds onto the new `UserApiKey` at multi-connection creation (D1), so refresh/reconnect stay on the authorizing client. | `services/user_token_service.rs`, `services/user_api_key_service.rs` |
| B4 | Platform-client scope allowlist (D5): enforced at `initiate_oauth_connect` when platform creds resolve; per-provider allowlist lives with the provider seed/config. | `services/user_token_service.rs`, `services/scope_catalog.rs` or provider config |
| B5 | Admin credential hygiene: reject empty/whitespace `client_id`/`client_secret`, require the pair together for oauth2, add an explicit clear/unset operation. | `handlers/providers.rs`, `services/provider_service.rs` |
| B6 | Callback audit sanitation: stop persisting raw provider `error_description` into audit metadata (provider-controlled string); store a mapped/safe variant, keeping parity with `safe_provider_error_message()`. | `handlers/user_tokens.rs:473` |
| B7 | Tests: startup non-reversion (D4 both cases); BYO-embedded refresh uses BYO client; same-client secret rotation refreshes OK; client-ID replacement is detected/documented as breaking; platform-creds-invalid initiation returns a recoverable typed error; scope allowlist rejects non-allowlisted platform-flow scopes. | services above |

### Frontend

| # | Change | Files |
|---|---|---|
| F1 | Dialog forward branch per §4 state machine (platform path skips `oauth_credentials`). | `add-key-dialog.tsx:2285` |
| F2 | Secondary BYO path + mode-switch state clearing + back-nav mirroring (`:2678`) + reconnect source preservation + initiation-failure fallback, per §4. | `add-key-dialog.tsx` |
| F3 | Types/validation: add `has_platform_oauth_credentials` to the `CatalogEntry` interface (`types/keys.ts:138-215`). `[v2]` There is no existing runtime Zod schema for catalog responses — do not invent an unused one; typed interface + the schemas that do exist (`schemas/providers.ts`) are the touchpoints. | `types/keys.ts`, `schemas/providers.ts` if applicable |
| F4 | Tests: one-click render for platform-provisioned GitHub/Google; BYO form when `has_platform_oauth_credentials=false`; deep link `/keys?tab=services&slug=api-github` lands on the one-click step; back-nav does not expose the skipped form; mode-switch clears BYO state; reconnect preserves credential source. | `add-key-dialog.test.tsx`, `keys.test.tsx` |

### Ops runbook (per environment: dev / staging / prod)

1. Register the GitHub OAuth app + Google OAuth client (Web application); callback
   `https://<env-host>/api/v1/providers/callback`; publish the Google consent screen to
   **Production** (Testing mode expires refresh tokens after 7 days).
2. Deploy the backend containing B2 (nothing sticks before that).
3. PUT credentials (`client_id` + `client_secret`). While `credential_mode` is still
   `"user"` these are inert — `resolve_oauth_credentials` will not use them, so no
   user is exposed yet (and note: they CANNOT be smoke-tested in this state).
4. PUT `credential_mode: "both"`, then immediately smoke-test with an ops/canary
   account: real connect + forced refresh. The smoke test is the only validation
   that the client actually exists provider-side — initiation checks ciphertext
   presence only and cannot detect a bad client before the provider's own error
   page (see §6 limitation). If the smoke test fails, revert to `"user"` (one PUT)
   while investigating; user impact is bounded to the smoke-test window.
6. Verify: catalog shows `has_platform_oauth_credentials: true`; a fresh user connects
   end-to-end with zero typed input; restart the backend and re-verify the mode
   survived (D4 regression).
7. Rotation: same-client secret only (D3); record prior values before PUT; revoke the
   old secret provider-side only after the new one is live.

## 6. Security notes

- Client secrets: AES-envelope-encrypted via the existing provider write path; Debug
  redaction in place; no read API returns them. B1 exposes only a boolean; the
  long-standing `oauth_client_id` exposure is documented (client IDs are public) and
  unchanged here (D7).
- CSRF `state` (10-min, one-time) + encrypted PKCE verifier: existing. Google uses
  PKCE; GitHub OAuth apps ignore PKCE (`supports_pkce: false` stays).
- `[v2]` Trust boundary stated precisely: initiation is authenticated (human-only
  mounts); the generic callback is authenticated by possession of the one-time state,
  by design — it is not itself session-bound.
- `[v2]` Platform-client scope allowlist (B4) is the control that prevents arbitrary
  scope escalation on the shared app; BYO keeps free-form scopes on the user's own app.
- `[v2]` Callback audit metadata no longer stores raw provider `error_description` (B6).
- Assistant path unchanged: connecting always happens in the NyxID frontend; no
  authorize URL or credential material crosses to Aevatar.

## 7. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Google verification friction for sensitive scopes | B4 allowlist keeps platform flows on non-sensitive scopes until verification (Phase 2); maintain a manual Google-classification list, not the `sensitive` display flag |
| Platform client-ID replacement breaks all refresh tokens | D3: treat as migration with reauthorization campaign; never PUT a new client ID in place |
| Legacy provider-level BYO users refreshed with wrong client | B3 embeds BYO creds on the key at creation |
| Shared platform app rate limits / provider-side suspension | Per-user tokens carry most limits; monitor; BYO remains the escape hatch; D2 isolates login |
| Bad PUT (typo'd secret) bricks platform flows | Staged runbook (creds → smoke test → mode flip); B5 validation; rollback values recorded |
| `has_platform_oauth_credentials=true` but flow broken (revoked secret, wrong callback, suspended app) | Acknowledged limitation: boolean = ciphertext present, and initiation cannot pre-validate the client provider-side — the user lands on the provider's error page (§4 error path b). Mitigation is operational: smoke test per env (runbook step 4), revert to `"user"` on failure |
| Startup migration silently reverts mode | D4/B2 + restart-survival test in runbook step 6 |

## 8. Acceptance criteria `[v2 corrected]`

- [ ] Fresh user connects GitHub from `/keys` with zero typed input (full-page redirect
      consent → return → Connected, proxy-ready `api-github`).
- [ ] Same for Google (`api-google`), including a stored refresh token (offline access).
- [ ] Backend restart does not revert `github`/`google` `credential_mode` from `"both"`
      (fresh-install and ops-patched cases).
- [ ] A user with pre-existing provider-level BYO GitHub credentials connects via their
      own app, and a later token refresh uses that same BYO client (embedded on the key).
- [ ] Reconnect of a BYO-originated connection stays on the BYO client; reconnect of a
      platform-originated connection stays on the platform client.
- [ ] `AddKeyDialog` shows no client-ID/secret form when `credential_mode="both"` and
      `has_platform_oauth_credentials=true`; BYO form when platform creds absent; Back
      never reveals the skipped form; platform ↔ BYO switching clears stale BYO state.
- [ ] Platform-flow scope requests outside the per-provider allowlist are rejected at
      initiation with a typed error; BYO flows are unaffected.
- [ ] Same-client secret rotation: existing connections keep refreshing; runbook's
      client-ID-replacement warning validated by a test that documents the breakage.
- [ ] Admin credential API rejects empty values and supports explicit clearing.
- [ ] No API response, log line, audit row, or snapshot contains a client secret or raw
      provider `error_description`; platform `oauth_client_id` exposure is unchanged
      from today and documented.
- [ ] Assistant connect card for `api-github` lands on the one-click step (manual QA
      with #1211 once landed).

## 9. Out of scope / follow-ups

- Promoting further providers — requires the §1 compatibility checklist per provider
  plus app registration + staged runbook; not "pure ops".
- Lark/Feishu stay BYO by nature (per-company Custom Apps).
- Removing the decrypted platform `oauth_client_id` from catalog responses (D7 cleanup).
- Env-based credential provisioning for infra-as-code.
- Google sensitive-scope verification + allowlist expansion (Phase 2).
- Provider-health monitoring beyond the presence boolean (active probe of authorize/token
  endpoints).
- Aevatar connector manifest / `connect` descriptor on `GET /api/v1/proxy/services`
  (separate spec on this branch).
