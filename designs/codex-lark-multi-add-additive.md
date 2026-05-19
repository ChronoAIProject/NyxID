# Multi-OAuth Adds Without Schema Migration

**Status**: Draft v2 — reviewed once (4 blockers found and addressed), ready for re-review
**Scope**: Fix "can't add a second codex / second Lark" without touching existing data or dropping any index
**Replaces**: the migration-heavy plan in `multi-connection-oauth.md` for this specific fix
**Canonical source**: this doc is the single source of truth for the codex/Lark multi-add fix. Any deviation in implementation requires updating this doc first.

## 1. Problem

A user who runs `nyxid service add codex` (or `add lark`) a second time hits silent failures:

- **OAuth2 (Lark, Feishu)**: silently overwrites the first token on the upstream-callback `delete_many` at `user_token_service.rs:1429`, OR (WIP) is blocked with a 409.
- **Device-code (codex / OpenAI ChatGPT)**: `unified_key_service.rs:521` calls `find_existing_provider_token`, which returns the existing token. `create_api_key_from_provider_token` at line 641 aliases the new `UserApiKey` to that same token. The placeholder returns `status: "active"` and the wizard short-circuits to "Done" without authorizing anything new.

**Goal**: second add runs the full OAuth/device-code flow, user authorizes another account (or the same one — no dedup), new service binds to its own independent token. First service is untouched. CLI and UI surfaces don't change.

## 2. Hard constraints

> No changes to BE schema. Only additive changes. No data migration. No mutating existing rows. No dropping or replacing indexes.

This rules out the broader plan in `multi-connection-oauth.md` (drop unique index, backfill `connection_id`, etc.). Instead:

- Optional `connection_id: Option<String>` fields on three models (already merged in this branch — additive).
- **One new index**: unique partial on `UserApiKey.connection_id where connection_id exists`. Additive — doesn't affect rows where the field is absent.
- Existing indexes on `user_provider_tokens`, `user_provider_credentials`, and `user_api_keys` are untouched.
- Existing rows are untouched. No backfill, no migration script.

## 3. Architecture

### 3.1 Why this works

`UserApiKey` already carries all encrypted token fields inline:

```rust
pub struct UserApiKey {
    pub access_token_encrypted: Option<Vec<u8>>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub token_scopes: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub provider_config_id: Option<String>,
    pub user_oauth_client_id_encrypted: Option<Vec<u8>>,      // for BYO-client providers (Lark)
    pub user_oauth_client_secret_encrypted: Option<Vec<u8>>,
    pub connection_id: Option<String>,                         // added in this branch — additive
    ...
}
```

`proxy_service.rs:1668` reads the primary credential token directly off `UserApiKey.access_token_encrypted`. `user_provider_tokens` is consulted only by refresh and a couple of secondary-auth paths.

**Conclusion**: `UserApiKey` is a viable self-contained token store. For new (multi-connection) adds we write tokens directly to the `UserApiKey` row and skip `user_provider_tokens` entirely. Legacy paths continue writing to `user_provider_tokens` exactly as today.

### 3.2 Two parallel paths discriminated by `connection_id`

| Flow | Discriminator | Token storage | Refresh path | Refresh client creds |
|---|---|---|---|---|
| **Legacy** — existing single-connection user | `connection_id = None` everywhere | `user_provider_tokens` (existing) + sync to `UserApiKey` | `oauth_flow::refresh_oauth_token` on `UserProviderToken` row | `user_provider_credentials` (existing) |
| **Multi-connection** — every new add post-deploy | `connection_id = Some(uuid)` on `UserApiKey`, `OAuthState`, `UserProviderToken` (if any) | **`UserApiKey._id` direct write**. `user_provider_tokens` NOT written. | `refresh_user_api_key_in_place` on `UserApiKey` row | `UserApiKey.user_oauth_client_{id,secret}_encrypted` |

The two paths share no state. They never collide because the discriminator partitions the data.

### 3.3 No collision with the existing unique index

`user_provider_tokens` retains its `unique(user_id, provider_config_id)` index. Safe because:
- Legacy flows write exactly one row per `(user, provider)` per user — same as today.
- Multi-connection flows do NOT write to `user_provider_tokens` at all.

`user_provider_credentials` retains its `unique(user_id, provider_config_id)` index. Safe because:
- Legacy flows continue to upsert credentials there.
- Multi-connection flows write client_id/secret into the `UserApiKey` row itself (the fields are already there — currently used only for UI display, now also used by the new refresh path).

## 4. Code changes

All changes are in the backend. Frontend is touched in one place (defensive, optional).

### 4.1 Remove silent-alias reuse (`unified_key_service.rs`)

**POST `/keys` (create path)**:
- Line 521-526: remove the `find_existing_provider_token` lookup.
- Line ~640-680: remove the `create_api_key_from_provider_token` branch.
- Every catalog add for an `oauth2` or `device_code` provider now flows through the `else` branch that mints a fresh `pending_auth` `UserApiKey`.

**PUT `/keys/:id` (upgrade path)** — same fix:
- Line 2194: remove the `find_existing_provider_token` lookup.
- Line 2336-2347: remove the `create_api_key_from_provider_token` branch.

The other two `find_existing_provider_token` call sites are read-only (async gate at 1934, proxy target resolution — verified in review) — leave untouched.

### 4.2 Mint fresh `connection_id` on every new add

In `create_key` (POST) and the upgrade-to-OAuth branch (PUT), when minting a new `UserApiKey` for an `oauth2` or `device_code` provider:

```rust
let connection_id = Uuid::new_v4().to_string();
// CreateApiKeyParams { connection_id: Some(&connection_id), ... }
// initiate_oauth_connect(..., connection_id: Some(&connection_id))
// initiate_device_code_flow(..., connection_id: Some(&connection_id))
```

**ORDERING REQUIREMENT (race-window guard — surfaced by the step 16 re-review).**
`reconcile_pending_oauth_placeholder` Pass 2 marks a multi-connection
`pending_auth` placeholder `failed` if no live `OAuthState` carries its
`connection_id`. If `create_key` mints the placeholder *before* the
`OAuthState` exists, a `GET /keys/:id` poll landing in that gap would
fail the placeholder prematurely. Mitigation — the implementer of step 19
MUST do one of:

1. **Insert the `OAuthState` before minting the placeholder** (or in the
   same logical unit), so a poll can never observe placeholder-without-state; OR
2. **Add a minimum-age grace guard to Pass 2**: skip the `failed` write
   when `api_key.created_at` is within a short grace window (e.g. 30s) —
   the `OAuthState` insert is expected imminently.

Today this is dormant: no live path mints a `connection_id: Some`
`pending_auth` key yet, so Pass 2's narrowed branch is exercised only by
tests. Step 19 makes it live — handle the ordering there.

### 4.3 Thread `connection_id` through OAuth state creation

`user_token_service::initiate_oauth_connect` (line ~515-590): add `connection_id: Option<&str>` parameter. Set on the inserted `OAuthState` row.

Existing callers (`handlers/user_tokens.rs:292`, `handlers/admin_sa_providers.rs:239`): pass `None` to preserve today's behavior. Only the new multi-connection path from `unified_key_service::create_key` passes `Some(uuid)`.

Same change for `initiate_device_code_flow` at line ~810-870.

### 4.4 Branch OAuth callback on `OAuthState.connection_id`

`user_token_service::handle_oauth_callback` (line 1243):

```rust
if let Some(ref conn_id) = oauth_state.connection_id {
    // Multi-connection: write tokens directly to the matching UserApiKey
    user_api_key_service::write_oauth_tokens_to_key(
        db, encryption_keys, conn_id,
        access_token, refresh_token, scope, token_expires_at,
    ).await?;
    // Skip user_provider_tokens insert. Skip sync_provider_token_to_api_keys.
} else {
    // Legacy: existing behavior unchanged.
    db.user_provider_tokens.delete_many({user_id, provider_config_id}).await?;
    db.user_provider_tokens.insert_one(token).await?;
    sync_provider_token_to_api_keys(db, user_id, provider_config_id).await?;
}
```

### 4.5 Branch device-code completion (BLOCKER B5 fix)

`user_token_service::store_device_code_tokens` (line 1154) currently takes only `state: &str`. **Signature must change** to receive `connection_id: Option<&str>` from the caller. Callers are `poll_device_code` and the recursive call at line 1129; both have the `OAuthState` in scope and pass `oauth_state.connection_id.as_deref()`.

Inside `store_device_code_tokens`, branch identically to 4.4.

### 4.6 New helper: `write_oauth_tokens_to_key`

In `user_api_key_service`:

```rust
pub async fn write_oauth_tokens_to_key(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    connection_id: &str,
    access_token: &str,
    refresh_token: Option<&str>,
    token_scopes: Option<&str>,
    expires_at: Option<DateTime<Utc>>,
) -> AppResult<()> {
    // Find the UserApiKey by connection_id (unique within the partial index).
    // Encrypt and update its access/refresh tokens, scopes, expires_at,
    // updated_at. Flip status from pending_auth → active.
    // Audit: emit oauth_callback_succeeded with connection_id in event_data.
}
```

### 4.7 New helper: `refresh_user_api_key_in_place`

Mirrors `oauth_flow::refresh_oauth_token` but operates on `UserApiKey` instead of `UserProviderToken`:

```rust
pub async fn refresh_user_api_key_in_place(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    api_key: &UserApiKey,
) -> AppResult<UserApiKey> {
    // 1. Decrypt refresh_token from api_key.refresh_token_encrypted
    // 2. Resolve client_id + client_secret:
    //    - If api_key.user_oauth_client_id_encrypted is Some (Lark/BYO):
    //        decrypt from api_key (covers BLOCKER B4)
    //    - Else (provider-owned, e.g. codex):
    //        load ProviderConfig.client_id_encrypted, decrypt
    // 3. POST to provider.token_url with grant_type=refresh_token
    // 4. On success: update_one({_id: api_key.id}) with new tokens + expires_at + status="active"
    // 5. On failure: update_one with status="failed" (note: "failed", not "refresh_failed",
    //    to be consistent with the wizard polling terminal-status list — see C3 below)
}
```

Note on (5): the existing legacy refresh writes `status: "refresh_failed"` on `UserProviderToken`. For `UserApiKey` we use `"failed"` so the wizard polling (`auth-flow-polling.ts` `isTerminalAuthFailureStatus`) treats it as terminal. This is a deliberate behavior choice for the new path; legacy path is unchanged.

### 4.8 Branch refresh in proxy path

`proxy_service::maybe_refresh_provider_backed_api_key` (line 1623):

```rust
if !needs_refresh { return Ok(api_key); }
let Some(provider_config_id) = api_key.provider_config_id.as_deref() else {
    return Ok(api_key);
};

if api_key.connection_id.is_some() {
    // Multi-connection: refresh in place on the UserApiKey
    refresh_user_api_key_in_place(db, encryption_keys, &api_key).await
} else {
    // Legacy: unchanged
    match user_token_service::get_active_token(...).await {
        Ok(_) => {
            sync_provider_token_to_api_keys(db, user_id, provider_config_id).await?;
            // re-fetch as today
        }
        Err(NotFound) => Ok(api_key),
        Err(e) => Err(e),
    }
}
```

### 4.9 Scope `sync_provider_token_to_api_keys` to legacy keys only (BLOCKER B2 fix)

`user_api_key_service.rs:243-252`:

```rust
let keys: Vec<UserApiKey> = db
    .collection::<UserApiKey>(COLLECTION_NAME)
    .find(doc! {
        "user_id": user_id,
        "provider_config_id": provider_config_id,
        "status": { "$nin": ["revoked", "failed"] },
        "connection_id": null,                    // NEW — exclude multi-connection keys
    })
    .await?
    .try_collect()
    .await?;
```

This prevents a legacy `user_provider_tokens` refresh from clobbering a multi-connection `UserApiKey`'s independently-managed tokens. Multi-connection keys are refreshed only via `refresh_user_api_key_in_place` (4.7).

### 4.10 Scope `reconcile_pending_oauth_placeholder` (BLOCKER B1 fix)

`reconcile_pending_oauth_placeholder` runs on every `GET /keys/:id` poll. It has two passes:

- **Pass 1** — "a token landed; pull it forward": reads `user_provider_tokens` by `(user, provider)` and, if newer than the placeholder, syncs it onto the key.
- **Pass 2** — "abandoned-flow sweep": if the placeholder is still `pending_auth` and no live `OAuthState` remains, mark it `failed` so the wizard exits instead of hanging.

**The fix is NOT a blanket early-return.** An early review iteration tried `if api_key.connection_id.is_some() { return Ok(()) }`, but that also skipped Pass 2 — leaving abandoned multi-connection placeholders as permanent `pending_auth` orphans (no TTL on `user_api_keys`, and `fail_pending_placeholders_for_provider` is now `connection_id: null`-scoped so it skips them too). That's a regression in cleanup behavior.

Correct fix — **skip only Pass 1 for multi-connection keys; keep Pass 2, scoped by `connection_id`**:

```rust
// Pass 1: LEGACY-ONLY. Multi-connection keys are activated by their
// own OAuth callback (`write_oauth_tokens_to_key`); they must never
// inherit a token from `user_provider_tokens`. Running Pass 1 for
// them is the B1 bug: a legacy *sibling* token's refresh bumps
// `user_provider_tokens.updated_at` past this pending key's
// `updated_at`, Pass 1 inherits that unrelated token, and the
// placeholder flips to `active` with the wrong credentials.
if api_key.connection_id.is_none() {
    // ... existing Pass 1 logic ...
}

// Pass 2: runs for BOTH legacy and multi-connection placeholders.
// For multi-connection keys the live-state lookup is narrowed by
// `connection_id` — each connection's flow has its own `OAuthState`
// carrying that id. Without the narrowing, a sibling connection's
// in-flight `OAuthState` for the same `(user, provider)` would keep
// this placeholder pending forever and abandonment would never be
// detected.
let mut state_filter = doc! { /* $or user_id/target_user_id, provider, expires_at */ };
if let Some(ref conn_id) = api_key.connection_id {
    state_filter.insert("connection_id", conn_id.as_str());
}
```

This preserves the B1 fix (no token inheritance) AND keeps abandonment cleanup working for multi-connection keys — no permanent orphan rows.

### 4.11 Fix delegated/secondary auth and gateway URL (BLOCKER B3 fix)

**`delegation_service::resolve_delegated_credentials` — NO CHANGE NEEDED (revised after implementation).**

The original draft prescribed branching this function on `api_key.connection_id`. Implementation review found that prescription was based on a misread: `resolve_delegated_credentials` does not receive an `api_key` at all (it takes `user_id` + `service_id`), and — more importantly — **every one of its four callers gates it behind the legacy `DownstreamService` path**:

- `handlers/proxy.rs` — skipped when `resolved_user_service_id.is_some()`
- `handlers/llm_gateway.rs` (×2) — skipped when `resolved_via_user_service`
- `services/mcp_service.rs` — skipped for `McpToolSource::UserManaged`

A multi-connection `UserApiKey` only ever backs a new-path `UserService`, whose credential is injected directly from `target.credential`. It can never reach `resolve_delegated_credentials`. And if a future refactor ever misrouted one here, `get_active_token` returns `NotFound` (no `user_provider_tokens` row for a connection-scoped key) → an explicit `BadRequest`, not a silent wrong-credential injection. The function is left untouched; a doc comment was added pinning the invariant.

**`resolve_gateway_url_override` — NO CHANGE NEEDED (revised after implementation).**

The original draft proposed adding a `gateway_url` field to `UserApiKey` so the resolver could read a per-connection gateway URL. Implementation review found this unnecessary: `resolve_gateway_url_override` is **legacy-path only** — it's called exclusively from `resolve_proxy_target` / `resolve_proxy_target_lenient`, which operate on a legacy `DownstreamService`. The new-path `UserService` resolver (`finish_resolution`) takes `base_url` straight from `UserEndpoint.url`, and **every `UserService` add provisions its own `UserEndpoint`** — so the gateway URL is already per-connection in the new model. A multi-connection `UserApiKey` never reaches `resolve_gateway_url_override`.

A `gateway_url` field was briefly added to `UserApiKey` (and then reverted) once this was confirmed — it would have been dead weight. The function is left untouched with a doc comment pinning the invariant. OpenClaw multi-connection works out of the box via per-endpoint URLs.

### 4.12 New unique partial index

In `db.rs::ensure_indexes()`:

```rust
db.collection::<UserApiKey>(USER_API_KEYS)
    .create_index(
        IndexModel::builder()
            .keys(doc! { "connection_id": 1 })
            .options(
                IndexOptions::builder()
                    .unique(true)
                    .partial_filter_expression(doc! {
                        "connection_id": { "$exists": true }
                    })
                    .build(),
            )
            .build(),
    )
    .await?;
```

Defense-in-depth — UUIDs are unique by construction, but this catches bugs. The partial filter excludes rows where `connection_id` is absent, so existing rows (which serialize with `skip_serializing_if = "Option::is_none"`) are not affected.

**Pre-index-create check** (BLOCKER C5 fix): before creating the index, run a one-shot startup audit:

```rust
let dupes = db.collection::<Document>(USER_API_KEYS)
    .aggregate(vec![
        doc! { "$match": { "connection_id": { "$exists": true } } },
        doc! { "$group": { "_id": "$connection_id", "n": { "$sum": 1 } } },
        doc! { "$match": { "n": { "$gt": 1 } } },
    ])
    .await?;
// If any duplicates, log loudly and refuse to create the unique index.
// This is purely defensive — no prior code path wrote non-unique values.
```

### 4.13 Wizard short-circuit (CONCERN C4)

Keep `frontend/src/components/cli-wizard/auth-flows.tsx:857` and `:1325` as-is.

The reviewer's recommendation: the `if (placeholder.status === "active")` defensive check is correct fail-safe. Backend won't return `active` on a fresh multi-connection add (because the silent-alias paths are gone), but the client check protects against future regressions or other paths that might return `active`. Removing it provides no benefit.

This deviates from v1 of the doc, which proposed removing it. Decision: **keep**.

## 5. What stays unchanged

- **`user_provider_tokens`** indexes and data: untouched.
- **`user_provider_credentials`** indexes and data: untouched. Legacy keys continue to read client creds from here. Multi-connection keys read from `UserApiKey.user_oauth_client_*_encrypted` (already-existing fields).
- **`user_api_keys`** existing rows: untouched. The one new optional field (`connection_id`) is absent on existing rows; its absence keeps them on the legacy path.
- **`oauth_states`** existing rows: untouched.
- **Proxy primary read** at `proxy_service.rs:1668`: untouched. Always read from `UserApiKey.access_token_encrypted`.
- **Telegram + api_key flows** at `user_token_service.rs:340, 467`: untouched. Not OAuth multi-add scenarios.
- **CLI**: no command changes, no new flags.
- **API surfaces**: no new endpoints. `connection_id` is internal and is NOT serialized in API responses (existing `skip_serializing_if = "Option::is_none"` keeps it out).
- **Existing tests**: continue to pass. New tests added for new behavior.

## 6. Edge cases

### 6.1 User with one codex (legacy), refreshes their token
Token in `user_provider_tokens`, `UserApiKey.connection_id = None`. Refresh takes the legacy branch (4.8 else). Identical to today.

### 6.2 User with legacy codex + new multi-connection codex — both expire
- Legacy: refresh via `user_provider_tokens`, sync to legacy `UserApiKey` (filter at 4.9 picks up `connection_id: null` only). ✓
- New: in-place refresh on the new `UserApiKey`. ✓
- No interference (B1, B2 fixes guarantee).

### 6.3 Aliased codex slugs (codex, codex-2 both `connection_id: None`)
Both keys have `connection_id: None`, share one `user_provider_tokens` row. Today's behavior, unchanged. A third codex added post-deploy gets a fresh `connection_id` and lives in `UserApiKey` only — no interference with the legacy pair.

### 6.4 In-flight OAuth callback during deploy
Pre-deploy `OAuthState` rows have no `connection_id` field → deserializes as `None` (backward-compat verified by `bson_backward_compat_missing_new_fields` test). Callback takes legacy branch (4.4 else). Identical to pre-deploy behavior.

### 6.5 Multi-connection key refresh fails (provider revokes grant)
`refresh_user_api_key_in_place` writes `status: "failed"` (not `refresh_failed` — see 4.7 note on C3). Wizard polling sees terminal status, surfaces error. Legacy keys for the same `(user, provider)` are untouched. ✓

### 6.6 User deletes legacy codex while multi-connection codex exists
Legacy key deletion is unchanged: `delete_api_key` removes the `UserApiKey` row. The `user_provider_tokens` row becomes orphaned (no legacy key references it). Reviewer's concern C7: does the orphan cause issues?

- `sync_provider_token_to_api_keys` filter at 4.9 only acts on `connection_id: null` keys. After deletion, there are no `connection_id: null` keys for that `(user, provider)`. The orphan is silently ignored. ✓
- `reconcile_pending_oauth_placeholder` filter at 4.10 returns early for `connection_id: Some(...)` keys. The orphan token row is never read. ✓

No active cleanup needed; the orphan is harmless. Document this in the migration-cleanup follow-up if desired.

### 6.7 Reverse: orphaned `user_provider_tokens` row without legacy key
Already happens today (e.g. revoke key but provider token still around). Already handled — no new risk.

### 6.8 User scope-edits the legacy codex
Re-running OAuth on the legacy key: wizard generates a new `OAuthState` with `connection_id = None` (because the existing `UserApiKey` has `connection_id: None`). Callback takes legacy branch, writes to `user_provider_tokens`, syncs to the legacy `UserApiKey`. Scopes updated. ✓

### 6.9 Multi-connection PUT-upgrade
Same as POST: 4.1 removes the alias branch. Test case in §9.

## 7. Risks

1. **Two refresh code paths (legacy + in-place) to maintain.** Trade-off accepted for zero migration. Legacy path can be retired in a future cleanup once all keys naturally migrate (which they won't without user action — so probably permanent).

2. **`refresh_user_api_key_in_place` must support BYO client creds for Lark.** Implementation must resolve client_id/secret from `UserApiKey.user_oauth_client_*_encrypted` first, falling back to `ProviderConfig` for provider-owned credentials (codex). Tested in §9.

3. **Cross-connection delegation explicitly unsupported in v1.** A service that requires delegation (`ServiceProviderRequirement`) cannot be backed by a multi-connection key unless delegation can be resolved from the api_key alone. Errors with explicit message. Acceptable scope cut.

4. **`store_device_code_tokens` signature change** is a real source-incompatible change to a private function. All in-tree callers are in `user_token_service` itself — easy to fix. No external callers.

5. **Defense-in-depth index** (4.12) could fail at deploy if some prior code path mistakenly wrote a duplicate `connection_id`. The pre-check audit catches and refuses; ops sees a loud log message. Practically impossible unless an out-of-band script populated the field.

6. **Audit logging consistency**: both legacy and new branches emit equivalent `oauth_callback_succeeded` events with `connection_id` in event_data (None or Some) so a debugger can tell which path executed.

## 8. What we're explicitly NOT doing

- No `user_provider_connections` collection (the broader design doc's approach).
- No data backfill, no migration script, no startup hook that touches existing data.
- No index drops or replacements (only one new partial index added).
- No new API endpoints.
- No CLI changes.
- No UI for managing connections (`nyxid connection list` etc.).
- No scope-editing UI (separate work — orthogonal).
- No automated cleanup of orphan `user_provider_tokens` rows.
- No support for cross-connection delegation in v1.

## 9. Test plan

### 9.1 Unit
- `write_oauth_tokens_to_key`: writes encrypted bytes to the right row, flips status pending_auth → active, sets `updated_at`.
- `refresh_user_api_key_in_place` with provider-owned creds (codex): refreshes successfully without touching `user_provider_tokens`.
- `refresh_user_api_key_in_place` with BYO creds (Lark): reads client_id/secret from `UserApiKey.user_oauth_client_*_encrypted` and refreshes.
- `refresh_user_api_key_in_place` on provider failure: writes `status: "failed"` and the error message.
- Pre-index audit: detects duplicates and refuses to create the index.

### 9.2 Integration
- **Codex multi-add (the user-facing bug)**: add codex → authorize → add codex again → second placeholder is `pending_auth`, full device-code flow runs, NEW token row in `UserApiKey._id`, first codex untouched. Assert: wizard does NOT short-circuit (placeholder.status returned to client is `pending_auth`, not `active`).
- **Lark multi-add**: same as above but for Lark OAuth2 flow. Two `UserApiKey` rows with different `connection_id` and different `user_oauth_client_id_encrypted` (different Lark Custom Apps).
- **Lark same-Custom-App twice**: two `UserApiKey` rows with the same `user_oauth_client_id_encrypted` but different `connection_id`. Both work independently.
- **Legacy single-codex refresh**: token in `user_provider_tokens` refreshes, syncs to legacy `UserApiKey` (with `connection_id: null` filter at 4.9). Multi-connection keys for same user are NOT touched (test the B2 fix).
- **Reconcile interaction**: legacy refresh bumps `user_provider_tokens.updated_at` while a multi-connection codex is `pending_auth`. Poll the multi-connection key. Assert it does NOT inherit the legacy token (test the B1 fix).
- **In-flight callback during deploy**: insert `OAuthState` with `connection_id: null`, run callback. Takes legacy branch, writes to `user_provider_tokens`.
- **PUT-upgrade (C1)**: existing service upgraded to OAuth — verify it mints a fresh `connection_id` and runs the full flow.
- **Cross-connection isolation**: connection A's refresh fails → only A's `UserApiKey.status = "failed"`. Connection B's status untouched.

### 9.3 Index verification
- Two `UserApiKey` rows with `connection_id: None` — both insert successfully (partial filter excludes them).
- Two rows with the same non-null `connection_id` — second is rejected with duplicate-key error.
- Pre-check audit reports zero duplicates on a clean test DB.

### 9.4 End-to-end manual
- Codex single user: refresh-on-expiry → token updates → proxy returns updated bearer.
- Codex multi-user: same as above but with two connections, each authorized to a different ChatGPT account, used by different `nyxid service` slugs simultaneously.
- Lark two-Custom-Apps: see Lark scenarios above + manual proxy call to each service.

## 10. Implementation order

The order that was actually executed (annotations note where reality diverged from the original draft):

1. Index migration: add partial unique on `UserApiKey.connection_id` in `db.rs` + pre-check audit. (Low risk; doesn't enable new behavior.)
2. ~~Add `gateway_url` field to `UserApiKey`.~~ **Reverted.** Implementation review found the new-path proxy resolver already takes the gateway URL from `UserEndpoint.url` (per-connection by construction); the field was dead weight. See §4.11.
3. Helpers: `write_oauth_tokens_to_key` + `refresh_user_api_key_in_place`. Unit tests.
4. Thread `connection_id` through `initiate_oauth_connect` + `request_device_code`. Update callers to pass `None`.
5. Branch `handle_oauth_callback` (returns new `OAuthCallbackOutcome`) and `store_device_code_tokens` (signature change). Test legacy + new.
6. Branch `proxy_service::maybe_refresh_provider_backed_api_key`.
7. Add `connection_id: null` filters to `sync_provider_token_to_api_keys` + `fail_pending_placeholders_for_provider` (B2); restructure `reconcile_pending_oauth_placeholder` — Pass 1 legacy-only, Pass 2 connection-scoped (B1).
8. `delegation_service` (B3): **no code change** — `resolve_delegated_credentials` is legacy-path only (all callers gate it); doc comment added pinning the invariant.
9. **Remove silent-alias branches in `unified_key_service.rs` (POST + PUT). Mint fresh UUID.** ⚠ Must insert the `OAuthState` before — or in the same logical unit as — minting the placeholder, OR add a minimum-age grace guard to reconcile Pass 2 (see §4.2 ORDERING REQUIREMENT). This is the step that flips multi-add on.
10. Integration tests: codex multi-add, Lark multi-add, in-flight callback, PUT-upgrade.
11. cargo build + cargo test.

Each step independently testable; PR can be split across 1-2 boundaries if needed.

## 11. Open questions

1. **Should `connection_id` be serialized in API responses?** Currently kept out via `skip_serializing_if`. Trade: leaking it could help debugging UI display ("this key belongs to connection X") but adds API surface. Default: keep internal until UI need arises.
2. **Long-term retirement of the legacy refresh path**: never auto-migrates without user action. Acceptable, or do we want a future "background re-authorize" to drain legacy?
3. **Abandonment of multi-connection placeholders**: handled — reconcile Pass 2 now runs for multi-connection keys (connection-scoped). No permanent orphan rows. See §4.10.
