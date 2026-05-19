# Multi-Connection Custom-App Credentials

**Status**: Draft — needs sign-off before implementation
**Extends**: `codex-lark-multi-add-additive.md` — completes the write-side wiring that doc asserted (§3.1, §4.7) but never operationalized, and adds multi-app + link-to-existing.
**Distinct from**: `multi-connection-oauth.md` — that is the schema-migration design (new `user_provider_connections` collection, index drops, backfill). This doc stays **additive**: no new collection, no migration, no index changes.
**Folds into**: the `feat/multi-connection-oauth` branch.
**Estimated effort**: mid-sized — ~400–600 LOC across backend + wizard + tests. Comparable to ~2 implementation steps of `codex-lark-multi-add-additive.md`. The `UserApiKey` model fields and the refresh-read side already exist on the branch; this is the write side + initiate/callback credential resolution + the copy mechanism + wizard wiring + bundle rebuild + tests.

## 1. Problem

For app-scoped OAuth providers (`credential_mode: "user"` — Lark, Feishu, Twitter/X), the OAuth `client_id`/`client_secret` belong to a **Custom App the user registered**, not to NyxID. The user brings their own.

Today those credentials are written to `user_provider_credentials`, which is keyed `unique(user_id, provider_config_id)` — **exactly one Custom App slot per `(user, provider)`**. Two consequences:

1. **Multi-connection refresh is broken.** The `feat/multi-connection-oauth` branch mints a `connection_id` for every `oauth2` add (Lark included) and routes it to the new path, where `refresh_user_api_key_in_place` reads the client credentials from `UserApiKey.user_oauth_client_id_encrypted` / `user_oauth_client_secret_encrypted`. Nothing populates those fields — `create_api_key` initializes both to `None`. So a multi-connection Lark connection works until its access token expires (~2h), then refresh fails: the key has no client credentials, and `ProviderConfig` has none either for a `credential_mode: "user"` provider. `codex-lark-multi-add-additive.md` §3.1/§4.7 assume these fields are populated but no step ever wired the write.

2. **A user cannot own more than one Custom App per provider.** The `unique` constraint caps it at one. Registering a second Lark app overwrites the first.

The expanded requirement from product: a user must be able to (a) register **multiple** distinct Custom Apps per provider, and (b) when adding a new connection, either **enter a new Custom App** or **reuse ("link to") the Custom App of an existing connection**.

### 1.1 Terms

- **Custom App** — an OAuth application the user registered with the provider (e.g. in their Lark developer console). Identified by a `client_id` + `client_secret`.
- **Connection** — one authorization instance: a `UserApiKey` row with a `connection_id`, holding an access/refresh token obtained by running OAuth against a Custom App, plus its own copy of that app's `client_id`/`client_secret`.
- **Link to existing** — a wizard convenience: instead of re-typing Custom App credentials, the user picks an existing connection's app and the backend **copies** its credentials onto the new connection. After creation the two connections share no state (see §5.1).

## 2. Requirements

### 2.1 Storage

- **R1** — For a multi-connection add of a `credential_mode: "user"` provider, the Custom App `client_id` and `client_secret` are stored **encrypted on the connection's `UserApiKey` row** (`user_oauth_client_id_encrypted` / `user_oauth_client_secret_encrypted`).
- **R2** — These credentials are **not** written to the `user_provider_credentials` collection. The wizard must stop calling `PUT /providers/{id}/credentials` for multi-connection `credential_mode: "user"` adds.
- **R3** — There is **no** per-`(user, provider)` cap on the number of connections. A user may hold any number of Custom Apps for a given provider, each as its own connection.

### 2.2 Credential sources at connection creation

- **R4 — New-app path.** The user can supply brand-new Custom App credentials (`client_id` + `client_secret`); they are stored on the new connection's `UserApiKey`.
- **R5 — Link-to-existing path.** The user can instead identify an existing connection whose Custom App should be reused. The backend copies that source connection's stored `client_id`/`client_secret` onto the new connection's `UserApiKey`.
- **R6 — Copies are independent.** After creation, a linked connection shares no state with its source. Rotating, re-authorizing, or deleting one connection does not affect the other. (See §5.1 for the precise scope of "independent".)
- **R7 — Server-side copy.** The link-to-existing copy is performed entirely server-side. The client identifies the source by id; it never receives or re-transmits the source's `client_secret`. (`client_secret` is write-only across the API today — `get_user_credentials_metadata` strips secrets, and model structs are never serialized to responses.)
- **R8 — Same Custom App may back multiple connections.** Two connections may legitimately hold copies of the same `client_id`. This is allowed with **no warning and no deduplication** — they are independent connections that happen to share an app.
- **R9 — Ownership check on copy.** The link-to-existing copy verifies the source connection is owned by the same principal (user, or org owner) as the new connection.

### 2.3 Credential resolution

- **R10** — For a connection that carries its own `user_oauth_client_*_encrypted`, the OAuth client credentials used for **(a)** the authorization URL (`initiate_oauth_connect`), **(b)** the auth-code → token exchange (`handle_oauth_callback`), and **(c)** token refresh (`refresh_user_api_key_in_place`) are all resolved from the connection's own `UserApiKey` — not from `user_provider_credentials` and not from `ProviderConfig`.
- **R11** — When the connection has no `user_oauth_client_*_encrypted` (legacy keys, and provider-owned device-code keys such as codex), resolution falls back to the **existing** behavior unchanged. The discriminator is "does this key carry its own client credentials," mirroring what `refresh_user_api_key_in_place` already does.
- **R12** — A multi-connection Lark/Feishu connection's access token refreshes successfully for the lifetime of the connection, using the connection's own stored client credentials.

### 2.4 Wizard

- **R13** — When adding a connection for a `credential_mode: "user"` provider, the wizard presents an **app-source step**: reuse an existing Custom App (picker) or enter new credentials.
- **R14** — The picker is derived from the user's existing OAuth connections (distinct `client_id` + connection label). No new "apps" collection backs it.

### 2.5 Compatibility (non-functional)

- **R15 — Additive only.** No new MongoDB collection, no schema migration, no index drops or replacements. Consistent with `codex-lark-multi-add-additive.md`'s hard constraint.
- **R16 — Legacy untouched.** Existing single-connection users, the `user_provider_credentials` collection and its indexes, `PUT /providers/{id}/credentials`, and the legacy `oauth_flow::refresh_oauth_token` path all continue to work unchanged.
- **R17 — Secrets** are encrypted at rest with the existing `EncryptionKeys` envelope; all Debug impls already redact. No plaintext client secret is logged or returned.

## 3. Non-goals

- **Shared-reference Custom Apps.** Connections do **not** reference a shared app entity; each holds an independent copy (decided — §5.1). No `user_oauth_apps` collection, no `app_id` FK.
- **Secret rotation cascade.** Because copies are independent, rotating one connection's secret does not propagate. A bulk "rotate everywhere" tool is out of scope.
- **The `multi-connection-oauth.md` migration** (new `user_provider_connections` collection, `connection_id` backfill, index drops).
- **Connection-management UI / CLI** (`nyxid connection list`, etc.) — separate work.
- **Scope-editing UI** — separate, orthogonal work.
- **Cross-tenant / org-shared Custom Apps.**

## 4. Background: what already exists

- **Lark, Feishu, Twitter/X are already `credential_mode: "user"`** — seeded that way in `provider_service.rs`, ship no admin `client_id`/`client_secret`, and are re-pinned to `"user"` by a startup migration. There is nothing to change about Lark's provider config; this doc is purely about *where* and *how many* Custom App credentials live.
- **The branch already added the model fields.** `UserApiKey.user_oauth_client_id_encrypted` / `user_oauth_client_secret_encrypted` and `connection_id` exist (`models/user_api_key.rs`). **Keep them — they are the correct home.**
- **The refresh-read side already works.** `refresh_user_api_key_in_place` (`user_token_service.rs:1743`) reads `api_key.user_oauth_client_id_encrypted` first (lines 1765–1799), falling back to `ProviderConfig`. It already handles the Lark/Feishu "HTTP 200 with non-zero `code`" refresh-failure mode. Once R1 populates the field, refresh works with **no change** to that function — verified.
- **The `connection_id` machinery already exists.** `OAuthState.connection_id`, the `handle_oauth_callback` branch on it, `write_oauth_tokens_to_key`, `initiate_oauth_connect(..., connection_id)`, and `resolve_connection_id_for_key` are all on the branch and are reused as-is.
- **`user_provider_credentials`** stays as the legacy/admin home, keyed `unique(user_id, provider_config_id)`. Untouched.

## 5. Key decisions (with rationale)

### 5.1 Independent copies, not a shared-reference app entity

A new connection that links to an existing Custom App gets an **independent copy** of the credentials, not a reference to a shared record.

Rationale (product): connections added separately should be maintained separately — independent deletion and re-keying at the NyxID layer, no shared-lifecycle surprises.

**Scope of "independent":** copies are independent *at the NyxID storage layer* — you can delete or re-key one connection without touching another's stored copy. Two connections that copied the same app still point at the **same Custom App on the provider's side** (same `client_id`); a revoke performed *in Lark* affects both. Provider-side isolation comes only from registering genuinely separate Custom Apps, which the new-app path (R4) supports.

This is why there is no `user_oauth_apps` collection and no `app_id` FK — there is no shared entity to model.

### 5.2 Credentials live on `UserApiKey`, written at `POST /keys`

Per §4, the fields already exist and the refresh side already reads them. The remaining gap is the **write**. `POST /keys` accepts the credentials and `create_key` stores them on the minted `UserApiKey`, atomically — no half-provisioned placeholder, one round-trip. This is the "Option A" that was discussed and is now confirmed by the independent-copies decision.

### 5.3 Link-to-existing is a server-side copy

Because `client_secret` is write-only across the API, the wizard cannot fetch a source app's secret to re-submit it. The copy must happen server-side: the client passes a **source key id**, the backend decrypts the source's credentials and re-encrypts them onto the new key. The source is identified by a specific key id (unambiguous) rather than by `client_id` (which can be non-unique across copies).

### 5.4 Resolution discriminator is "key carries its own client creds", not "connection_id is present"

`initiate_oauth_connect`, `handle_oauth_callback`, and `refresh_user_api_key_in_place` resolve from the key **iff** the key has `user_oauth_client_*_encrypted`. Otherwise they fall back to existing behavior. This keeps provider-owned device-code keys (codex) on their existing path while fixing BYO providers, and it mirrors the precedence `refresh_user_api_key_in_place` already implements.

### 5.5 Fold into `feat/multi-connection-oauth`

At ~400–600 LOC, additive, no migration, this is small enough to land on the branch rather than as a follow-up.

## 6. Architecture

### 6.1 The model

```
UserApiKey (a "connection")
  connection_id                     ← per-add unique id (already on branch)
  user_oauth_client_id_encrypted    ← this connection's copy of the Custom App client_id
  user_oauth_client_secret_encrypted← this connection's copy of the Custom App client_secret
  access_token_encrypted / refresh_token_encrypted / token_scopes / expires_at
  provider_config_id, status, ...
```

No shared app entity. "Link to existing" copies `user_oauth_client_*_encrypted` from a source `UserApiKey` into the new one at creation time.

### 6.2 Credential resolution — the three points

All three resolve client credentials. Today only the refresh point reads the key; the other two read `user_provider_credentials`. After this change, all three prefer the key.

| Point | Function | Today | After |
|---|---|---|---|
| Authorization URL | `initiate_oauth_connect` (`user_token_service.rs`) → `resolve_oauth_credentials` | reads `user_provider_credentials` | if key has `user_oauth_client_*`, use it; else existing |
| Code → token exchange | `handle_oauth_callback` (`user_token_service.rs:1434`) → `resolve_token_oauth_credentials` | reads `user_provider_credentials` — **unconditionally, before the `connection_id` branch at :1555** | if key (resolved via `oauth_state.connection_id`) has `user_oauth_client_*`, use it; else existing |
| Refresh | `refresh_user_api_key_in_place` (`user_token_service.rs:1743`) | already reads `api_key.user_oauth_client_*_encrypted`, falls back to `ProviderConfig`; does **not** use `credential_user_id` | unchanged |

The callback's exchange-credential resolution at `:1434` is the non-obvious one: it runs *before* the `connection_id` branch and is required for the auth-code → token exchange. Without changing it, a multi-connection Lark callback fails at `:1440` (`resolve_token_oauth_credentials` finds nothing once the wizard stops writing `user_provider_credentials`) — long before `write_oauth_tokens_to_key` at `:1560`.

**Suggested shared helper** — `resolve_connection_oauth_credentials(db, encryption_keys, connection_id) -> AppResult<Option<ResolvedOAuthCredentials>>`: returns `Some` when the connection's key carries its own client credentials, `None` otherwise. Both `initiate_oauth_connect` and `handle_oauth_callback` become `if let Some(c) = resolve_connection_oauth_credentials(...).await? { c } else { <existing resolution> }`. The decrypt logic already exists inside `refresh_user_api_key_in_place` and can be extracted.

## 7. Data model

No model changes. The fields are already present:

- `UserApiKey.user_oauth_client_id_encrypted: Option<Vec<u8>>` — already on branch
- `UserApiKey.user_oauth_client_secret_encrypted: Option<Vec<u8>>` — already on branch
- `UserApiKey.connection_id: Option<String>` — already on branch

No new collection. No new index. No migration.

## 8. API changes

### 8.1 `POST /api/v1/keys`

`CreateKeyRequest` gains three optional fields:

| Field | Type | Meaning |
|---|---|---|
| `oauth_client_id` | `Option<String>` | New Custom App client_id (new-app path, R4) |
| `oauth_client_secret` | `Option<String>` | New Custom App client_secret (new-app path, R4) |
| `copy_oauth_client_from` | `Option<String>` | Source `UserApiKey` id to copy credentials from (link-to-existing path, R5) |

Validation:
- `oauth_client_id` and `oauth_client_secret` must be supplied together (both or neither).
- `copy_oauth_client_from` is mutually exclusive with the raw pair — supplying both is rejected.
- For a `credential_mode: "user"` oauth2 add, exactly one source (raw pair or `copy_oauth_client_from`) is **required** — a `"user"`-provider connection with no Custom App cannot authorize, exchange, or refresh.
- `copy_oauth_client_from` must reference a key owned by the same principal that carries `user_oauth_client_*_encrypted`; otherwise `BadRequest` with a clear message (e.g. source is legacy / provider-owned / a credential-less placeholder).
- Rejected for any provider where `user_credentials_service::supports_user_credentials(provider)` is false (`credential_mode: "admin"`), reusing the existing `AppError::BadRequest("This provider does not accept user-provided credentials")` — so the `POST /keys` gate matches the existing `PUT /providers/{id}/credentials` gate exactly (§16.2).

### 8.2 Keys list

`GET /keys`'s `KeyResponse` does **not** currently expose the Custom App `client_id` (confirmed — `handlers/keys.rs`, `KeyResponse` ~lines 297–388; `source_app_id` is a Developer-App id, not the Custom App client_id). Add `oauth_client_id: Option<String>` to `KeyResponse`, decrypted from `UserApiKey.user_oauth_client_id_encrypted` when present — following the `catalog_service` precedent of decrypting and exposing a (non-secret) client_id. `client_secret` stays unexposed (R7). This needs `encryption_keys` at response-build time; thread it into `key_response_from_view` if not already available. The wizard picker reads this from the existing list endpoint — no new endpoint (§16.1).

### 8.3 Unchanged

`PUT /providers/{id}/credentials` and the `user_provider_credentials`-backed `GET /providers/{id}/credentials` are untouched — legacy/admin only.

## 9. Backend changes

1. **`CreateApiKeyParams` + `create_api_key`** (`user_api_key_service.rs`) — accept `oauth_client_id` / `oauth_client_secret`, encrypt and store into `user_oauth_client_*_encrypted` (currently hardcoded `None`).
2. **`CreateKeyRequest` + `POST /keys` handler** (`handlers/keys.rs`) — the three new fields + validation (§8.1).
3. **`create_key`** (`unified_key_service.rs`) — thread the credentials through to `CreateApiKeyParams`. For `copy_oauth_client_from`: load the source `UserApiKey`, ownership-check, copy its `user_oauth_client_*_encrypted` onto the new key. Raw pair: pass through to be encrypted.
4. **`initiate_oauth_connect`** (`user_token_service.rs`) — when `connection_id` is set and the key carries client creds, resolve from the key instead of `resolve_oauth_credentials`.
5. **`handle_oauth_callback`** (`user_token_service.rs:1434`) — same branch for the exchange-credential resolution, keyed off `oauth_state.connection_id`.
6. **`refresh_user_api_key_in_place`** — no change; already reads the key.
7. **(Optional) extract** `resolve_connection_oauth_credentials` helper shared by 4 and 5 (§6.2).

## 10. Wizard UX

- **App-source step** (R13): for `credential_mode: "user"` providers, before the OAuth redirect, a step that branches "Reuse an existing Custom App" vs "Enter new credentials" — modeled on the boolean-flag step branching in `ai-key-confirm-panel.tsx` and the `phase` state machine in `auth-flows.tsx`. "Enter new" reuses the existing `needs-credentials` client_id/secret form.
- **Picker** (R14): a card grid matching `CatalogGrid` (the wizard's established "pick one of N" pattern) — **one card per distinct `oauth_client_id`**, showing the truncated client_id and the labels of connections using it (a flat per-connection list would show duplicate apps — the user is picking an *app*, not a connection). The card resolves to a concrete source `UserApiKey` id; selecting it → `POST /keys` with `copy_oauth_client_from: <that id>`. "Enter new" → `POST /keys` with `oauth_client_id` / `oauth_client_secret`. The wizard does not list user keys today — this is a new `GET /keys` fetch (§16.4).
- **Stop writing the shared table** (R2): remove the `saveUserCredentials` → `PUT /providers/{id}/credentials` call from the multi-connection `credential_mode: "user"` path. Credentials now flow only through `POST /keys`.
- Rebuild the bundled wizard assets (`cli/src/wizard/assets/`, `bundle-meta/index.hash`).

## 11. What stays unchanged

- `user_provider_credentials` collection, its `unique(user, provider)` index, and `PUT /providers/{id}/credentials` — legacy/admin only.
- `oauth_flow::refresh_oauth_token` and the legacy `UserProviderToken` refresh path.
- `ProviderConfig` and `credential_mode` semantics; Lark/Feishu/Twitter stay `credential_mode: "user"`.
- The branch's `connection_id` machinery (`OAuthState.connection_id`, `write_oauth_tokens_to_key`, callback branch, `reconcile_pending_oauth_placeholder` passes).
- Legacy single-connection keys (`connection_id = None`) — resolution falls back to existing behavior (R11).
- No new collections, no index changes, no migration (R15).

## 12. Edge cases

- **Copy source has no client creds** (legacy key, provider-owned device-code key, or a credential-less placeholder) → `POST /keys` rejects with a clear message.
- **Copy source owned by a different principal** → `404`/`403`, no leak of existence.
- **Raw pair + `copy_oauth_client_from` both supplied** → rejected (ambiguous).
- **Only one of `oauth_client_id` / `oauth_client_secret` supplied** → rejected (must be paired).
- **`credential_mode: "user"` oauth2 add with neither source** → rejected (the connection could never authorize).
- **Same `client_id` copied onto N connections** → allowed, no warning, no dedup (R8). Each has an independent copy; the partial unique index is on `connection_id` (a fresh UUID per add), so no index collision.
- **Stranded dev keys** — multi-connection Lark connections created on the branch *before* this lands have `connection_id` set but empty `user_oauth_client_*`. They are unfixable by code and require re-auth. Dev/staging only; no production exposure since the branch is unmerged.
- **`OAuthState.credential_user_id`** — no action, no model change. Once `initiate_oauth_connect` resolves from the connection (R10), the multi-connection path never calls `resolve_oauth_credentials`, so `credential_user_id` is never computed and the `OAuthState` field stays `None` naturally. It remains load-bearing for the legacy path (`handle_oauth_callback`, `oauth_flow::refresh_oauth_token`, `try_revoke_token_remote` all read it); `refresh_user_api_key_in_place` does not read it — verified (§16.3).

## 13. Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Callback exchange-cred branch missed → multi-connection Lark callback 500s | Med | High | §6.2 calls it out explicitly; integration test covers the full Lark add |
| Server-side copy leaks a secret into a response/log | Low | High | Copy is decrypt→re-encrypt only; never placed in a response struct; Debug impls already redact |
| Wizard still writes `user_provider_credentials`, masking the real cred source | Med | Med | R2 is explicit; test asserts no `PUT /providers/{id}/credentials` on the multi-connection path |
| Resolution branch keyed on `connection_id` presence instead of "key has creds" → breaks codex | Low | Med | §5.4 fixes the discriminator; test legacy + codex + Lark side by side |
| Copy source picked from a `pending_auth` placeholder with no creds yet | Low | Low | §12 rejects credential-less sources |

## 14. Test plan

### 14.1 Unit
- `create_api_key` stores `oauth_client_id`/`oauth_client_secret` encrypted; round-trips.
- `create_key` with `copy_oauth_client_from`: copies source's encrypted creds onto the new key; ownership-check rejects foreign/legacy/credential-less sources.
- `POST /keys` validation: paired-fields, mutual-exclusion, required-for-`user`-provider, rejected-for-`admin`-provider.
- `resolve_connection_oauth_credentials`: returns `Some` for a key with creds, `None` otherwise.

### 14.2 Integration
- **New-app add**: `POST /keys` with raw creds → authorize → callback exchange resolves from the key → token written → **refresh after expiry succeeds**.
- **Link-to-existing add**: register app A, then `POST /keys` with `copy_oauth_client_from = A` → new connection has its own copy → authorizes and refreshes independently.
- **Same app twice**: two connections, same `client_id`, different `connection_id` — both authorize and refresh independently.
- **Independence**: delete/re-key connection A; connection B (copied from A) still authorizes and refreshes.
- **Legacy untouched**: a `connection_id = None` Lark key still resolves via `user_provider_credentials` and refreshes via `oauth_flow::refresh_oauth_token`.
- **Codex (provider-owned)**: a device-code multi-connection key with no `user_oauth_client_*` still resolves from `ProviderConfig` (R11).

### 14.3 Wizard
- App-source step renders for `credential_mode: "user"` providers; picker is populated from existing connections.
- "Enter new" → `POST /keys` carries `oauth_client_id`/`oauth_client_secret`.
- "Reuse existing" → `POST /keys` carries `copy_oauth_client_from`.
- No `PUT /providers/{id}/credentials` call on the multi-connection path.

### 14.4 Build
- `cargo build` + `cargo test`; `npm run build` + `npm run test` (frontend); wizard bundle rebuilt and hash updated.

## 15. Implementation order

1. `CreateApiKeyParams` + `create_api_key`: accept & store `user_oauth_client_*`. Unit test.
2. `CreateKeyRequest` + `POST /keys` handler + `create_key`: three new fields, validation, the server-side copy branch. Unit tests.
3. Credential resolution: extract `resolve_connection_oauth_credentials`; branch `initiate_oauth_connect` and `handle_oauth_callback` (`:1434`). Tests for legacy + codex + Lark.
4. Keys-list: add `oauth_client_id: Option<String>` to `KeyResponse` (§8.2).
5. Wizard: app-source step + picker; remove the `PUT /providers/{id}/credentials` call on the multi-connection path; rebuild bundle.
6. Integration + wizard tests; full `cargo` + `npm` build/test.

Each step is independently testable. Steps 1–3 are backend-only and land the refresh fix; 4–5 land the wizard UX.

## 16. Resolved decisions

Resolved against the current code and existing patterns.

### 16.1 Keys-list exposure
`KeyResponse` does not expose `client_id` today (`source_app_id` is a Developer-App id, not the Custom App client_id). **Decision:** add `oauth_client_id: Option<String>` to `KeyResponse`, decrypted from `user_oauth_client_id_encrypted`, mirroring `catalog_service`'s decrypt-and-expose-client_id pattern. No new endpoint. Folded into §8.2.

### 16.2 `credential_mode: "admin"` providers
**Decision:** reject the BYO credential fields for any provider where `supports_user_credentials(provider)` is false, reusing the existing `AppError::BadRequest("This provider does not accept user-provided credentials")`. This makes the `POST /keys` gate identical to the existing `PUT /providers/{id}/credentials` gate (`handlers/user_credentials.rs:98–102`) — one consistent rule, not new behavior. Folded into §8.1.

### 16.3 `OAuthState.credential_user_id`
**Decision:** no change, no cleanup. The multi-connection cred-resolution branch (R10) means `initiate_oauth_connect` never computes `credential_user_id` on that path, so the field stays `None` without explicit clearing. It remains load-bearing for the legacy path. Verified that `refresh_user_api_key_in_place` does not read it. Folded into §12.

### 16.4 Picker grouping
**Decision:** card grid matching `CatalogGrid`, one card per distinct `oauth_client_id` — not a flat per-connection list, since the user is picking an *app*, not a connection. The card resolves to a concrete source key id for `copy_oauth_client_from`. Folded into §10.
