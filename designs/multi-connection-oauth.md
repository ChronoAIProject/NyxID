# Multi-Connection OAuth + Wizard UX

**Status**: Draft — needs sign-off before implementation
**Issues**: extends the gap surfaced during #653 work; supersedes the deferred β + α follow-ups
**Estimated effort**: 9-14 working days

## 1. Problem statement

Today NyxID's OAuth model assumes one OAuth identity per `(user, provider)`. This is correct for Google, GitHub, OpenAI, etc. (where the user has one account on the provider) but **wrong for Lark, Feishu, and Twitter/X**, where the OAuth `client_id` belongs to an *app the user registered* — not to the user themselves. A single NyxID user can legitimately own multiple Lark Custom Apps (e.g. "Marketing", "Support", "Internal"), each with its own client_id, scopes, and tenant approval scope.

Today, attempting to register a second Lark app would silently overwrite the first in `user_provider_credentials`, causing every existing service backed by the first app to start using the second app's credentials — usually with the wrong scopes, often broken.

Compounding the data model gap, the wizard makes three silent decisions that hide what's happening:

1. Skips the credentials prompt when `user_provider_credentials` exists
2. Skips the OAuth flow entirely when `user_provider_tokens` exists (jumps to "Done" in ~200ms)
3. Auto-disambiguates slug collisions to `-2`, `-3`, etc.

Users reported "the wizard exits without doing anything" — which is the second silent decision masquerading as a bug. The actual bug is that the wizard never tells the user it's reusing existing state.

## 2. Goals

1. Support **multiple distinct OAuth connections per `(user, provider)`** for app-scoped providers (Lark, Feishu, Twitter, future per-app providers).
2. Wizard **explicitly surfaces every reuse / collision / replace decision** instead of silently skipping steps. Default action stays the fast-path; alternatives become visible.
3. Allow **scope expansion on an existing connection** without recreating any services. CLI: `nyxid connection edit-scopes`. Wizard: button on the reuse-confirm panel.
4. **Zero breaking changes**: existing single-connection users continue to work without intervention. All current API endpoints, CLI commands, and wizard flows accept old shapes.
5. **Migration safety**: forward-only, idempotent backfill that can be re-run if interrupted. Verifiable via post-deploy audit query.

## 3. Non-goals

- Cross-tenant connection sharing (e.g. "let User A's Lark connection be used by User B")
- Per-service OAuth (each service gets its own token) — overkill; multiple services per connection is correct
- OAuth provider self-hosting on NyxID's side (Lark Marketplace publication) — separate, larger initiative
- Migration of `user_provider_credentials` collection deletion — keep collection for one release as rollback safety, drop in follow-up

## 4. Key decisions (with rationale)

### 4.1 New collection `user_provider_connections` instead of repurposing `user_provider_credentials`

The existing collection has `label: Option<String>`. Repurposing it would require making the label required, adding a `connection_id` PK separate from the row's own `_id`, and dealing with all the legacy rows that have `label: None`. New collection is cleaner and gives us a fresh, well-modelled schema.

### 4.2 Default connection is named `"Default"` and auto-created during migration

Backfill creates one `user_provider_connections` row per existing `user_provider_credentials` row with `label = "Default"`, `is_default = true`. This means every existing service immediately maps to a real connection. Users see "Default" in the UI for what was previously implicit.

### 4.3 `is_default: bool` flag on connections

Exactly one connection per `(user, provider)` is marked default. Used by:
- `nyxid service add` without `--connection`: uses the default
- API `POST /api/v1/keys` without `connection_id`: uses the default
- Wizard with no explicit picker: starts on the default

Rationale: preserves zero-friction single-connection UX while enabling multi-connection. Without a default, every new service add would need a picker even for users with one connection.

### 4.4 Connection label is unique within `(user, provider)` but not globally

User can have a "Marketing" Lark connection AND a "Marketing" Twitter connection — those are different `(user, provider)` pairs. Avoids forcing globally-unique names.

### 4.5 Connection deletion is `block-if-services-use-it`

Three options were considered:
- **Cascade**: delete all services using the connection. Predictable but destructive.
- **Block**: refuse if any service uses it. Forces explicit cleanup, prevents accidents.
- **Mark disconnected**: leave services in a third terminal state. Adds operational complexity.

Choosing **block**. User must delete services first or transfer them to another connection. Error message includes the list of blocking services.

### 4.6 OAuth callback identifies connection via OAuth state row

`initiate_oauth_connect` already creates an `OAuthState` row keyed by a random state string, with `(user_id, provider_config_id)` recorded. Add `connection_id` to `OAuthState`. On callback, the state row tells us which connection received the new token, so `delete_many` becomes scoped by `connection_id`.

### 4.7 New `connection_id` column on `user_api_keys` is `Option<String>`

Optional because:
- Non-OAuth keys (api_key, basic, ssh_certificate, node_managed) don't have connections
- During migration, rows are backfilled but for a brief window may be `null`

Code paths that filter by connection check for `Some(connection_id)` first. If `None`, fall back to old `(user_id, provider_config_id)` resolution. After migration completes (and a release cycle), the fallback can be removed.

### 4.8 `user_provider_tokens` adds required `connection_id`

Required (not Option) on the model. Migration backfills all existing rows. Code that creates new tokens must set it. Hard guarantee that every token belongs to exactly one connection.

### 4.9 Edit-scopes uses re-OAuth, not in-place token mutation

The CLI `connection edit-scopes` and wizard "Add scopes" both:
1. Compute the union of existing scopes + new scopes
2. Call `initiate_oauth_connect` with `additional_scopes` set to the union, AND `prompt=consent`
3. User authorizes on the provider (sees the augmented consent screen)
4. Callback writes the new token (scopes = union) under the same connection_id, replacing the old token
5. `sync_provider_token_to_api_keys` propagates the new token to all linked services

No service rows touched. No scope-merging-without-provider-consent (which would lie about what the provider granted).

## 5. Schema changes

### 5.1 New collection: `user_provider_connections`

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserProviderConnection {
    #[serde(rename = "_id")]
    pub id: String,                       // UUID v4
    pub user_id: String,
    pub provider_config_id: String,
    pub label: String,                    // required, max 200 chars
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub client_id_encrypted: Option<Vec<u8>>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub client_secret_encrypted: Option<Vec<u8>>,
    pub is_default: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

pub const COLLECTION_NAME: &str = "user_provider_connections";
```

**Indexes:**
- Unique compound: `{user_id: 1, provider_config_id: 1, label: 1}`
- Unique partial: `{user_id: 1, provider_config_id: 1}` where `is_default = true`

### 5.2 Modify: `user_provider_tokens`

Add field:
```rust
pub connection_id: String,    // FK to user_provider_connections._id
```

**Index update**: drop unique on `{user_id, provider_config_id}`. Replace with unique on `{user_id, provider_config_id, connection_id}`.

### 5.3 Modify: `user_api_keys`

Add field:
```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub connection_id: Option<String>,    // FK; None for non-OAuth keys
```

No new indexes needed; existing `(user_id, provider_config_id)` lookups are still valid as a coarse filter, and the resolver narrows by `connection_id` post-fetch.

### 5.4 Modify: `oauth_states`

Add field:
```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub connection_id: Option<String>,
```

Optional because some flows (admin/SA initiated, future flows) may not have a connection. When present, callback uses it.

### 5.5 Deprecate (don't drop yet): `user_provider_credentials`

Code stops writing to it. Reads fall back to it during migration window. Drop after one release cycle.

## 6. Migration plan

### 6.1 Phases

**Phase 0 (this PR, day 1)**: deploy schema migration + dual-read code

1. `db.rs` `ensure_indexes()` adds `user_provider_connections` collection + indexes
2. `ensure_indexes()` adds `connection_id` field to existing rows with default behavior (null)
3. Code reads from new schema with fallback to old schema
4. Code writes to BOTH old (`user_provider_credentials`) and new (`user_provider_connections`) for any new connections created

**Phase 1 (this PR, day 2)**: backfill migration

A startup migration runs once per backend boot (idempotent):

```rust
// Pseudocode
for cred in user_provider_credentials.find({}).await {
    if user_provider_connections.find_one({user_id: cred.user_id, provider_config_id: cred.provider_config_id}).await.is_some() {
        continue; // already migrated
    }
    let conn_id = Uuid::new_v4().to_string();
    user_provider_connections.insert_one(UserProviderConnection {
        id: conn_id.clone(),
        user_id: cred.user_id.clone(),
        provider_config_id: cred.provider_config_id.clone(),
        label: cred.label.unwrap_or_else(|| "Default".to_string()),
        client_id_encrypted: cred.client_id_encrypted.clone(),
        client_secret_encrypted: cred.client_secret_encrypted.clone(),
        is_default: true,
        created_at: cred.created_at,
        updated_at: cred.updated_at,
    }).await?;
    user_provider_tokens.update_many(
        {user_id: cred.user_id, provider_config_id: cred.provider_config_id, connection_id: null},
        {$set: {connection_id: conn_id}}
    ).await?;
    user_api_keys.update_many(
        {user_id: cred.user_id, provider_config_id: cred.provider_config_id, connection_id: null},
        {$set: {connection_id: conn_id}}
    ).await?;
}
```

Verifiable via:
```js
db.user_provider_tokens.count({connection_id: null})        // should be 0
db.user_api_keys.count({provider_config_id: {$ne: null}, connection_id: null})  // should be 0
```

**Phase 2 (next release, optional)**: drop dual-write to `user_provider_credentials`. Drop fallback reads.

**Phase 3 (release after)**: drop `user_provider_credentials` collection.

### 6.2 Migration safety properties

- **Idempotent**: re-running on already-migrated data is a no-op (skip if connection already exists for the pair)
- **Forward-only**: no schema rollback needed; old code continues to read both old and new collections
- **Order-independent**: tokens and api_keys can be backfilled in either order
- **Transactional within a row**: insert connection → update tokens → update api_keys in one session per credentials row, or accept eventual consistency (the dual-read fallback covers the in-between window)

### 6.3 Rollback plan

If Phase 1 reveals a problem mid-migration:
1. Code is dual-read; old code path still works for any unmigrated rows
2. Stop the migration loop (kill backend)
3. Either: re-run after fix (idempotent), or drop new collection and start over (no data loss because old collection is unchanged)

## 7. API changes

### 7.1 New endpoints

| Method | Path | Description |
|---|---|---|
| GET | `/api/v1/connections` | List my connections (optionally filtered by `?provider_id=`) |
| GET | `/api/v1/connections/{id}` | Show one connection with token status |
| POST | `/api/v1/connections` | Create connection: `{provider_id, label, client_id?, client_secret?}` |
| PATCH | `/api/v1/connections/{id}` | Update: `{label?, client_id?, client_secret?}` |
| DELETE | `/api/v1/connections/{id}` | Delete (errors with 409 + service list if any in use) |
| POST | `/api/v1/connections/{id}/oauth/initiate` | Get authorization URL for THIS connection |
| POST | `/api/v1/connections/{id}/scopes/add` | Initiate OAuth re-flow with merged scopes |

### 7.2 Modified endpoints

**`POST /api/v1/keys`**:
- New optional field: `connection_id` (string)
- If omitted: uses default connection for the provider; if no default exists for this provider, behaves as today (creates one with label "Default")
- If specified: uses that connection
- Existing callers without `connection_id` continue to work (default fallback)

**`GET /api/v1/providers/{id}/credentials`**:
- New behavior: returns metadata about the *default* connection (backwards-compat shape)
- New optional query param `?all=true` returns array of all connections for this user+provider
- New fields in response: `connection_id`, `label`, `last_authorized_at`, `current_scopes`

**`GET /api/v1/providers/{id}/connect/oauth`**:
- New optional query param `connection_id`: which connection to OAuth into
- New optional query param `prompt=consent`: pass to provider's authorization URL
- If `connection_id` omitted: uses or creates default connection (current behavior preserved)

**`POST /api/v1/providers/callback`**:
- Reads `connection_id` from `OAuthState`, scopes the `delete_many` of old tokens to that connection
- Falls back to `(user_id, provider_config_id)` scope if the state has no `connection_id` (old in-flight flows)

### 7.3 Backwards compatibility matrix

| Caller | Old behavior | New behavior | Compat? |
|---|---|---|---|
| Frontend POSTs `/keys` without `connection_id` | Used `(user_id, provider)` token | Uses default connection's token | ✓ identical to user |
| CLI `service add` without `--connection` | Used `(user_id, provider)` token | Uses default connection's token | ✓ identical to user |
| External API caller (SDK) without `connection_id` | Same fast-path | Uses default connection | ✓ identical |
| In-flight OAuth callback from before deploy | Uses `(user_id, provider)` scope | Uses fallback to old scope | ✓ continues |

## 8. CLI changes

### 8.1 New commands

```bash
nyxid connection list [--provider <slug>]
nyxid connection show <provider-slug>:<label>
nyxid connection add <provider-slug> --label <name>          # interactive OAuth wizard
nyxid connection delete <provider-slug>:<label>              # confirms, errors if services use it
nyxid connection edit-scopes <provider-slug>:<label> --add-scope <scope>...
```

### 8.2 Modified commands

```bash
nyxid service add [SLUG] --connection <provider-slug>:<label>    # new optional flag
```

If user has multiple connections for the picked catalog service's provider:
- Without `--connection`: prompt interactively (or error in non-TTY mode)
- With `--connection`: use the specified one

If user has zero connections: walk through new-connection setup inline.

### 8.3 Backwards compatibility

All existing commands (`service add` without `--connection`, etc.) work as before. The default connection is used implicitly.

## 9. Wizard UX

### 9.1 New panel: Connection picker (when multiple exist)

```
┌────────────────────────────────────────────────────┐
│ Pick a Lark connection for this service            │
│                                                    │
│   ⦿ Marketing      cli_aaa…  (5/12, 2 services)   │
│   ◯ Support        cli_bbb…  (5/13, 1 service)    │
│   ◯ Internal       cli_ccc…  (5/15, 0 services)   │
│   ◯ + Add a new connection                         │
│                                                    │
│   [Continue]                                       │
└────────────────────────────────────────────────────┘
```

### 9.2 New panel: Reuse confirm (single connection)

```
┌────────────────────────────────────────────────────┐
│ ✓ Using your existing Lark connection              │
│                                                    │
│   Label:           "Default"                       │
│   App credentials: cli_aaa… (stored 5/12)          │
│   Last authorized: 5/12 13:50                      │
│   Current scopes:  contact:user.base:readonly,     │
│                    offline_access                  │
│   Used by:         api-lark, api-lark-2 (2)        │
│                                                    │
│   [Continue]                                       │
│                                                    │
│   Need additional permissions?                     │
│   [Add scopes]                                     │
│                                                    │
│   Force a fresh consent screen on Lark?            │
│   [Re-authorize]                                   │
│                                                    │
│   [+ Add a different Lark connection]              │
└────────────────────────────────────────────────────┘
```

The destructive "Replace credentials" button from the earlier draft is **gone** — adding a different connection is a non-destructive operation now.

### 9.3 New panel: New connection setup

Entered when user clicks "+ Add a new connection" from picker or reuse panel.

```
┌────────────────────────────────────────────────────┐
│ Add a new Lark connection                          │
│                                                    │
│   Connection label:    [_________________________] │
│                        e.g. "Marketing", "Support" │
│                                                    │
│   Client ID:           [_________________________] │
│   Client Secret:       [_________________________] │
│                                                    │
│   [Cancel]              [Continue to authorize]    │
└────────────────────────────────────────────────────┘
```

Standard OAuth flow follows.

### 9.4 New panel: Edit scopes

Entered when user clicks "Add scopes" on reuse panel.

```
┌────────────────────────────────────────────────────┐
│ Add scopes to "Default"                            │
│                                                    │
│   Currently granted:                               │
│     ✓ contact:user.base:readonly                   │
│     ✓ offline_access                               │
│                                                    │
│   Available to add:                                │
│     ☐ im:message:send                              │
│     ☐ im:message:read                              │
│     ☐ contact:contact.base:readonly                │
│     ...                                            │
│                                                    │
│   [Cancel]    [Re-authorize with new scopes]       │
└────────────────────────────────────────────────────┘
```

Click → triggers OAuth with merged scopes + `prompt=consent` → user sees augmented consent on Lark → token replaces old (broader scopes) → all linked services automatically use new token.

### 9.5 New panel: Slug collision

```
┌────────────────────────────────────────────────────┐
│ ⚠ A service named "api-lark" already exists       │
│                                                    │
│   Use a different slug:                            │
│   [api-lark-marketing                            ] │
│                                                    │
│   Or auto-pick "api-lark-3":  [Use api-lark-3]     │
│                                                    │
│   Or replace existing:        [Replace api-lark]   │
└────────────────────────────────────────────────────┘
```

### 9.6 Modified panel: existing OAuth flow

Becomes a step within the flow, only entered when:
- Adding a new connection (5.3), OR
- Re-authorizing (existing flow), OR
- Edit-scopes (5.4)

Otherwise the flow is: pick connection / reuse confirm → service create → done.

## 10. Proxy + sync changes

### 10.1 `sync_provider_token_to_api_keys`

```rust
// Before
.find({user_id, provider_config_id, status: NIN [revoked, failed]})

// After
.find({user_id, provider_config_id, connection_id, status: NIN [revoked, failed]})
```

When `connection_id` is null on a row (during migration window), the sync function uses the old query. After Phase 2 cleanup, drop the fallback.

### 10.2 `handle_oauth_callback`

```rust
// Before
db.user_provider_tokens.delete_many({user_id, provider_config_id})
db.user_provider_tokens.insert_one(new_token)

// After
let connection_id = oauth_state.connection_id.clone()
    .or_else(|| { /* fallback: lookup default for backwards compat */ })?;
db.user_provider_tokens.delete_many({user_id, provider_config_id, connection_id})
db.user_provider_tokens.insert_one(new_token { connection_id, .. })
```

### 10.3 Proxy resolver

```rust
// Before
let token = find_user_provider_token(db, user_id, &api_key.provider_config_id);

// After
let token = if let Some(ref conn_id) = api_key.connection_id {
    find_user_provider_token_by_connection(db, conn_id)
} else {
    // Backwards-compat fallback for unmigrated keys
    find_user_provider_token(db, user_id, &api_key.provider_config_id)
};
```

After Phase 2, drop the `else` branch.

## 11. Risk areas

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Migration backfill loses tokens | Low | High | Idempotent + dual-read fallback + verification queries + drill on prod data copy |
| Proxy 5xx during deploy | Med | High | Dual-read code (handles either old or new schema) + rolling deploy |
| In-flight OAuth before deploy completes | Med | Med | OAuth state's `connection_id` is Optional; callback falls back to default |
| Two connections accidentally created with same label | Low | Med | Unique index enforces |
| Connection deleted while service uses it | Med | Med | Block-with-error (4.5) prevents |
| User loses access by deleting wrong connection | Med | High | Block-with-error + show service list in error message |
| `is_default` invariant violated (zero or two defaults) | Low | Med | Unique partial index; tests assert |
| Old client (CLI 0.5.x or earlier) submits OAuth state without `connection_id` | High during transition | Low | Callback handles None case via default-connection fallback |

## 12. Test plan

### 12.1 Backend unit tests
- `user_provider_connection_service` CRUD
- Migration backfill (idempotency, partial state, multiple users)
- `sync_provider_token_to_api_keys` with connection_id
- `handle_oauth_callback` with and without connection_id in state
- Proxy resolver with and without connection_id on api_key

### 12.2 Backend integration tests
- Full OAuth flow for a brand-new user (creates default connection)
- OAuth flow for user adding 2nd connection (default unchanged)
- Edit scopes flow (token replaced, services updated)
- Connection delete blocked when in use
- Connection delete succeeds when no services use it

### 12.3 Migration drill
- Run on copy of production data
- Verify counts: `user_provider_connections.count()` == count of distinct `(user_id, provider)` tuples in `user_provider_credentials`
- Verify all `user_provider_tokens` and OAuth-backed `user_api_keys` have non-null `connection_id`
- Spot-check: pick a sample user, verify their proxy resolution still works

### 12.4 CLI tests
- All new connection commands
- `service add --connection` and prompt fallback
- Backwards-compat: existing commands without `--connection`

### 12.5 Frontend tests
- Connection picker rendering with 0, 1, multiple connections
- Reuse confirm panel button states
- Edit-scopes flow end-to-end
- Slug collision panel
- Wizard test for backward compat (no connection_id in API response → falls back to default behavior)

### 12.6 End-to-end manual checklist
- New user signs up → connects Lark → adds Lark service (no picker, default connection auto-created)
- Existing user (with one connection from migration) → wizard shows reuse confirm → adds 2nd service (uses Default)
- Same user → adds 2nd Lark connection labeled "Marketing" → adds service backed by Marketing
- Same user → wizard now shows picker → picks Default → service created bound to Default
- Same user → edits scopes on Marketing → consent screen on Lark → returns → Marketing services have new scopes, Default services unchanged
- Same user → tries to delete Default → blocked with list of services
- Same user → deletes blocking services → deletes Default → Marketing now becomes default automatically? (Open question, see §13)

### 12.7 Rollback test
- Deploy old binary on top of migrated data
- Old binary's queries (without `connection_id`) still work because old schema is preserved (Phase 0/1)
- Confirm proxy still resolves; existing services still work

## 13. Open questions for review

1. **Default connection re-election**: when the default is deleted, do we automatically promote another (e.g. oldest) to default, or leave the user with no default until they explicitly mark one?
2. **Connection visible across orgs**: are connections strictly per-user, or can org-owned connections be shared across org members? (Probably per-user; defer org-shared connections to a future RFC.)
3. **Migration timing**: run inline at backend startup (one-time, blocks boot), or as a separate script? (Inline is simpler; one boot blocked by ~few-seconds backfill on a single-instance deploy is acceptable.)
4. **Connection label changes**: are they allowed to be edited later, or fixed at creation? (Allow edit; updates flow through to UI but doesn't trigger any token operations.)
5. **Same `client_id` across two connections of the same provider**: allowed or rejected? (Allowed — useful for testing; warn in UI.)
6. **`user_provider_credentials` deletion**: drop in next release after migration, or wait two? (Wait two — gives a release of buffer for any forgotten code path.)
7. **Telemetry**: do we want events on connection.create, connection.delete, connection.scopes_added for product insight?

## 14. Implementation milestones

- **M1 (1 day)**: Schema + migration + dual-read; no behavior change
- **M2 (2 days)**: New connection service + endpoints + tests
- **M3 (1 day)**: Modified `unified_key_service` + `handle_oauth_callback` + sync
- **M4 (1 day)**: Proxy resolver update + tests
- **M5 (2 days)**: CLI new + modified commands
- **M6 (3 days)**: Frontend wizard panels (picker, reuse confirm, new connection, edit scopes, slug collision)
- **M7 (1 day)**: End-to-end testing + manual drills
- **M8 (1 day)**: Migration drill on prod-data copy + docs

**Total: ~12 days** with single owner. Can parallelize M2/M5 with M6 if multiple owners.

## 15. Out of scope

These are deliberately deferred:

- **Lark Marketplace publication**: separate initiative; would let NyxID-owned Lark app be installed by tenants without each user needing to register their own. Removes most of the friction this design addresses, but is a months-long ops + legal cycle.
- **Org-shared connections**: defer until usage justifies the schema/permission changes.
- **Connection-level rate limits or ACLs**: out of scope; existing per-API-key isolation covers the agent-rate-limit use case.
- **Provider account discovery**: knowing "which Lark user is this Marketing connection authorized as" beyond opaque `credential_user_id`. Defer until needed.
