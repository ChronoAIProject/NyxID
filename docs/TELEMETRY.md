# Telemetry — What goes where

| Field | Value |
|---|---|
| Status | Implementation ready |
| Ticket | [ChronoAIProject/NyxID#442](https://github.com/ChronoAIProject/NyxID/issues/442) |
| Branch | `feat/analytics-m1-discovery` |
| Parent plan | `TELEMETRY_PLAN.md` (original draft — lives outside the repo) |
| Reviews | Codex session `019db3de-1090-7130-b782-13b8b2be1a1a` (two passes — both findings folded) |

## 1. Purpose

Give us visibility across all four NyxID surfaces — FE, BE, CLI, Mobile — so we can see *where* users go and *how* they use the product. One PostHog **production project** consumes all surfaces' events (a second share-back project exists only for community opt-in contributions; see §3). One `distinct_id` per user across every surface. Clean data from the first production event; identity and erasure plumbed from day one.

PostHog project provisioning, DPA, consent banner copy, dashboards, and deploy-time env wiring are operator responsibilities (out of code scope). This doc is the file-by-file map of the code changes.

## 2. Architecture

```
 ┌──────────────┐    ┌──────────────┐    ┌──────────────┐    ┌──────────────┐
 │  Frontend    │    │  Mobile      │    │  CLI         │    │  Backend     │
 │ (posthog-js) │    │ (posthog-rn) │    │ (reqwest)    │    │ (reqwest)    │
 │              │    │              │    │              │    │              │
 │ autocapture +│    │ autocapture +│    │ cli.command_ │    │ mw/telemetry │
 │ $exception + │    │ mobile.* +   │    │  invoked +   │    │ derives      │
 │ ui.*         │    │ ui.mobile.*  │    │ $identify on │    │ `surface`    │
 │              │    │              │    │ login        │    │ from header  │
 │ X-NyxID-     │    │ X-NyxID-     │    │              │    │              │
 │  Client: ui  │    │  Client:     │    │ X-NyxID-     │    │ emits one    │
 │              │    │  mobile      │    │  Client: cli │    │ event per    │
 │              │    │              │    │              │    │ user-action  │
 └──────┬───────┘    └──────┬───────┘    └──────┬───────┘    └──────┬───────┘
        │                   │                   │                   │
        └─── direct PostHog ingest (HTTPS POST /capture/) ─────────┘
                            │                                        │
                            ▼                                        ▼
                   ┌────────────────────────────────────────────────────┐
                   │       PostHog Cloud US (us.i.posthog.com)          │
                   │   One production project for hosted deploy.        │
                   │   Separate `share-back` project for community opt- │
                   │   in (`NYXID_SHARE_ANALYTICS=true`) — isolated     │
                   │   from production so spam/poisoning can't leak in. │
                   │                                                    │
                   │   distinct_id = user_id (all surfaces, post-auth)  │
                   │   PostHog's identify() auto-merges anon trails.    │
                   └────────────────────────────────────────────────────┘
```

Common event props: `surface`, `app_version`, `environment`, optional `client_version`. `surface` is server-derived from `X-NyxID-Client` + auth (api-key auth wins → `agent`).

## 3. Config (env-driven, same pattern everywhere)

All env var names are vendor-neutral per the hot-swap contract (§5.0). The DSN values are PostHog-shaped today (`phc_...`) but the variable names survive any vendor swap.

| Surface | DSN var | Host var | Share-back flag | Where it lives |
|---|---|---|---|---|
| Backend | `NYXID_TELEMETRY_DSN` | `NYXID_TELEMETRY_HOST` | `NYXID_SHARE_ANALYTICS` | Process env, `.env.production` |
| Frontend | (reads from backend `/api/v1/public/config` at runtime) | — | — | No build-time env. Fetched via `usePublicConfig()`. |
| CLI | `NYXID_TELEMETRY_DSN` | `NYXID_TELEMETRY_HOST` | `NYXID_SHARE_ANALYTICS` | User shell env per invocation |
| Mobile (Expo) | `expo.extra.TELEMETRY_DSN` | `expo.extra.TELEMETRY_HOST` | `expo.extra.NYXID_SHARE_ANALYTICS` | `mobile/app.json` — edit literal values before `eas build`, or migrate to `app.config.js` reading `process.env.EXPO_PUBLIC_*` |

Frontend has no `VITE_*` telemetry envs. The DSN, host, and share-back flag are fetched from the backend's `GET /api/v1/public/config` at app boot (same endpoint that already powered the login/MCP UI). Rotation = edit `.env.production` on the backend host and restart the backend container; the frontend picks up the new values on next page load with no image rebuild.

Precedence on every surface:
1. `*_DSN` non-empty → use it (production DSN on the hosted deploy; self-hoster's DSN otherwise).
2. `*_SHARE_ANALYTICS=true` → fall back to compiled-in public ingest key (`NYXID_PUBLIC_TELEMETRY_DSN`, pointing at the **share-back** project).
3. Otherwise → hard off, telemetry never initializes.

Host default: `https://us.i.posthog.com` (PostHog's default region for new projects). Operators on PostHog EU override via `NYXID_TELEMETRY_HOST=https://eu.i.posthog.com`. In the common case only `NYXID_TELEMETRY_DSN` needs to be set.

CLI also honors `~/.nyxid/config.toml` `[telemetry]` section managed via `nyxid telemetry enable|disable|status`. Mobile in-app Settings toggle flipping off calls `reset()` (vendor-neutral wrapper verb), clears the stored anon ID, and sets the consent store `enabled=false` so subsequent `init()` calls are no-ops until the user flips it back on.

**CLI first-run consent — full spec:**

Config file `~/.nyxid/config.toml` `[telemetry]` section:
```toml
[telemetry]
enabled = false   # user's choice; telemetry only fires when true
asked = false    # have we prompted this user yet?
```

**Resolution precedence, first match wins:**

1. `NYXID_TELEMETRY=off` env var → off (always wins; for CI and non-interactive use).
2. `NYXID_TELEMETRY=on` env var → on (only takes effect if a DSN is also resolvable via §3 resolution precedence; otherwise still hard-off).
3. `[telemetry] enabled = true` in `~/.nyxid/config.toml` → on.
4. `[telemetry] enabled = false AND asked = true` → off (user said no; don't re-prompt).
5. `[telemetry] asked = false` **OR config file doesn't exist** on an interactive TTY → prompt once. "Yes" writes `{enabled=true, asked=true}` (creates the file if needed); "No" writes `{enabled=false, asked=true}`. Either choice exits the prompt with status `0` and the originating command proceeds (we do NOT bail out of `nyxid login` just because telemetry was declined).
6. `asked = false` **OR config file doesn't exist** on a non-interactive TTY (pipe / CI) → treated as "No" for this invocation but does NOT persist (`asked` stays `false` or the file stays absent, so the next interactive run still prompts). Command proceeds normally.

`nyxid telemetry enable|disable|status` is the canonical editor for the config. `enable` sets `{enabled=true, asked=true}`; `disable` sets `{enabled=false, asked=true}` and deletes `~/.nyxid/profiles/{profile}/anon_id`; `status` prints the resolved state and where it came from (env / config / default).

## 4. Identity (unified across surfaces)

Same NyxID `user_id` (UUID v4) as PostHog `distinct_id` on every surface post-auth.

| Surface | distinct_id (authenticated) | distinct_id (pre-auth) | Transition |
|---|---|---|---|
| Frontend | `user_id` via `identify(user_id)` (wrapper verb) | anon ID in localStorage, owned by the vendor SDK | On login/register success; on `checkAuth()` resolving an existing session on boot. `reset()` on every sign-out (explicit, 401-triggered, account switch). StrictMode-safe via module-level `inited` guard. |
| Mobile | `user_id` via `identify(user_id)` (wrapper verb) | anon ID in AsyncStorage, owned by the vendor SDK | On login/register success; on SecureStore-session restore. `reset()` on every sign-out path in `AuthSessionContext.tsx` (explicit, 401 via `setSessionInvalidationListener`, SecureStore wipe, account switch). |
| Backend | `AuthUser.user_id.to_string()` | — (no pre-auth events emitted; they can't merge into the identified person and would leave undeletable orphans) | BE only sees authenticated traffic for business events. |
| CLI | `user_id` from `~/.nyxid/profiles/{profile}/user_id` (derived from JWT `sub` on login, cached) | UUID at `~/.nyxid/profiles/{profile}/anon_id` | On `nyxid login` success: `telemetry_client.identify(user_id)` — the wrapper reads the current anon ID and hands the merge to the vendor (wire protocol invisible). On `nyxid logout`: delete both files. |

### 4.1 Identity plumbing (already landed 2026-04-22)

| File | Change |
|---|---|
| `cli/src/auth.rs` | New `jwt_sub_from_token` utility (base64url-decode payload, extract `sub`). `save_tokens_for` now derives `user_id` from the access_token JWT and writes to `~/.nyxid/profiles/{profile}/user_id`; clears the file if the new token has no `sub` (prevents stale attribution). `read_saved_user_id_for` prefers JWT-from-current-access-token (canonical) with file fallback for sessions where the token is unreadable. Logout cleans the file. 5 unit tests cover valid/malformed/missing/non-string `sub` + stale-cleanup. |
| `mobile/src/lib/auth/jwt.ts` (new) | `decodeJwtSub(accessToken)` — base64url normalize + `globalThis.atob` + JSON parse + `sub` extract. |
| `mobile/src/lib/auth/sessionStore.ts` | `StoredAuthSession.userId?: string`. `persistAuthSession` is the single writer — auto-derives via `decodeJwtSub` so callers in `AuthHomeScreen.tsx` and `http.ts` refresh path need no changes. `loadStoredAuthSession` is read-only (no backfill-write) to avoid races with `clearStoredAuthSession` during sign-out; pre-feature sessions get JWT-derived `userId` returned without persisting. Orphan cleanup extended: any of `refreshToken` / `userId` / `expiresAt` without `accessToken` triggers a full clear. `clearStoredAuthSession` deletes the new `nyxid.auth.user_id` SecureStore key. |

**Security note:** JWT decoding on FE/Mobile/CLI trusts the `sub` claim without verifying the signature — safe for telemetry attribution (server-issued token, rejected upstream if tampered), never safe for authorization. Anything beyond telemetry reading these derivations is a bug.

Both surfaces' `npm run typecheck` / `cargo test` green (156 + 25 tests on CLI including the new stale-cleanup test).

### 4.2 Erasure

PostHog's `identify()` internally merges the anon distinct_id into the `user_id` person. Account deletion enqueues a job with `{user_id}`; the worker calls `DELETE /api/projects/{pid}/persons/?distinct_id={user_id}` with exponential backoff. PostHog erases the merged record including all aliased anon trails across FE/Mobile/CLI. No server-side alias tracking needed.

## 5. Event coverage (where what gets added)

**Implementation ordering within each surface** — schema first, client second, middleware third, call sites last. Writing call sites before the schema file exists means the schema gets retrofitted to whatever was emitted, which defeats the allowlist. The discriminated-union / enum pattern below is what makes "new event name" a compile error, not a runtime surprise.

### 5.0 Hot-swap contract (non-negotiable)

PostHog is the vendor today. It probably won't be forever. We pay a small upfront cost so that swapping vendors later is a one-file-per-surface rewrite, not a codebase-wide grep-and-replace.

**The contract:** no caller outside the four wrapper files imports the vendor SDK, references vendor-specific event names, or knows the vendor's payload shape.

| Concern | Rule |
|---|---|
| Module / file names | `telemetry.ts`, `telemetry.rs` — vendor-neutral. **Never** vendor-named like `posthog.ts` / `posthog.rs` / `mixpanel.ts`. |
| Env var names | `*_TELEMETRY_DSN`, `*_TELEMETRY_HOST`, `*_NYXID_SHARE_ANALYTICS`. **Never** `*_POSTHOG_*`. |
| Struct / type names | `TelemetryClient`, `TelemetryEvent`, `TelemetryContext`. **Never** vendor-prefixed like `PostHogClient` / `MixpanelClient`. |
| Public API verbs | `init(...)`, `identify(user_id)`, `reset()`, `capture(event)`, `captureException(err)`. These are the intersection of PostHog, Mixpanel, Amplitude, Segment, Sentry. All safe to keep when swapping. |
| Vendor wire protocol | PostHog's `$identify`, `$create_alias`, `$exception` event names live **inside** the wrapper's `identify()` / `reset()` / `captureException()` method bodies. Callers never see them. |
| Imports at call sites | `import { capture } from '@/lib/telemetry'` — not from `'posthog-js'`. The `posthog-js` / `posthog-react-native` imports live exclusively inside `telemetry.ts`. |

**What a vendor swap looks like** under this contract: rewrite `frontend/src/lib/telemetry.ts`, `mobile/src/lib/telemetry.ts`, `backend/src/telemetry/mod.rs`, `cli/src/telemetry.rs`. Optionally update env var values in deploy configs (new DSN format from the new vendor). Zero changes to any handler, component, hook, store, or page. That's the whole point.

**What stays identical across any swap:** the `TelemetryEvent` enum (§5.1), the `UiEvent` / `MobileEvent` discriminated unions (§5.2, §5.3), the redaction/scrubbing rules (§6), the consent model (§3). Those are our domain model, not vendor artifacts.

### 5.1 Backend — one event per user-action domain

New `backend/src/telemetry/mod.rs` (TelemetryClient, reqwest fire-and-forget, 2s timeout) + `backend/src/telemetry/schema.rs` (event enum) + `backend/src/telemetry/scrub.rs` (egress regex pass) + `backend/src/mw/telemetry.rs` (derives `surface` from `X-NyxID-Client`, stashes `TelemetryContext` in request extensions). Registered in `routes.rs` before auth middleware. Config parsed in `backend/src/config.rs`. `AppState` carries `Arc<Option<TelemetryClient>>`. Handler emits the event after the DB write succeeds; props from the handler's existing context only (no new DB reads).

**`backend/src/telemetry/schema.rs` — event enum (canonical source, write first):**

```rust
/// One variant per canonical event. Adding a new event = adding a variant
/// + a branch in `name()` and `properties()`. Unknown event names become
/// a compile error, not a runtime surprise.
pub enum TelemetryEvent {
    UserSignedUp { method: SignupMethod, invite_code_used: bool },
    UserDeleted { reason: Option<DeleteReason> },
    AuthLoggedIn { method: AuthMethod, mfa_required: bool },
    AuthLoggedOut,
    KeyCreated { source: KeySource, catalog_slug: Option<String>, has_node_binding: bool },
    ServiceConnected { provider_slug: String, flow: ConnectFlow },
    ApprovalDecided { service_slug: String, mode: ApprovalMode, decision: Decision, decision_ms: u64 },
    ProxyError { service_slug: String, error_code: u32, status: u16 },
    ApiRateLimited { route_pattern: &'static str, limit_type: LimitType },
    // ~40 variants total per §5.1 table
}

impl TelemetryEvent {
    pub fn name(&self) -> &'static str { /* "auth.logged_in" etc. */ }
    /// Returns the scrubbed JSON props. Scrubbing runs inside this fn so
    /// there is no way to emit un-scrubbed data.
    pub fn properties(&self) -> serde_json::Value { /* scrub::apply on string fields */ }
}
```

**`backend/src/telemetry/mod.rs` — TelemetryClient (public API):**

```rust
pub struct TelemetryClient {
    dsn: String,
    host: String,
    environment: &'static str,
    app_version: &'static str,
    tx: mpsc::Sender<CaptureJob>,  // bounded, capacity 1024
}

impl TelemetryClient {
    /// None if DSN unresolvable per §3 precedence (hard-off).
    pub fn from_config(cfg: &AppConfig) -> Option<Arc<Self>>;

    /// Fire-and-forget. Drops the event if the bounded channel is full
    /// (treat a burst overflow as better than blocking request handlers).
    pub fn track(&self, distinct_id: &str, event: TelemetryEvent, ctx: &TelemetryContext);

    /// For the erasure worker to use directly.
    pub async fn delete_person(&self, distinct_id: &str) -> Result<(), reqwest::Error>;
}

/// Spawned once from main.rs on startup; drains the channel and POSTs to
/// `{host}/capture/` with 2s reqwest timeout. Retries on 5xx with jittered
/// backoff; drops on 4xx (event-shape bug, not a transient error).
async fn drain_loop(mut rx: mpsc::Receiver<CaptureJob>, client: reqwest::Client);
```

**`backend/src/mw/telemetry.rs` — middleware (public API):**

```rust
#[derive(Clone, Debug)]
pub struct TelemetryContext {
    pub surface: &'static str,           // "ui" | "cli" | "mobile" | "agent" | "backend"
    pub client_version: Option<String>,  // from X-NyxID-Client-Version header
}

pub async fn telemetry_mw(req: Request, next: Next) -> Response;
// Reads X-NyxID-Client + X-NyxID-Client-Version + auth.api_key_id,
// builds TelemetryContext, stashes in req.extensions_mut(), calls next.
```

**`backend/src/telemetry/scrub.rs` — egress scrubber (public API):**

```rust
/// Applies the §6 redaction rules to every string-valued property in-place.
/// Called internally by `TelemetryEvent::properties()`. Never skipped.
pub fn scrub_string(s: &str) -> Cow<'_, str>;
pub fn scrub_value(v: &mut serde_json::Value);
```

Per-event sampling (for future high-volume events like `channel.message_received`) is deferred — the helper and its deterministic-hash dependency will land in the PR that introduces the first sampled emission.

**Handler call pattern:**

```rust
async fn create_key_handler(
    State(state): State<AppState>,
    Extension(tele): Extension<TelemetryContext>,
    auth: AuthUser,
    Json(body): Json<CreateKeyRequest>,
) -> AppResult<Json<KeyResponse>> {
    let key = key_service::create(&state.db, &auth.user_id, body).await?;

    if let Some(tele_client) = &state.telemetry {
        tele_client.track(&auth.user_id.to_string(),
                          TelemetryEvent::KeyCreated {
                              source: key.source,
                              catalog_slug: key.catalog_slug.clone(),
                              has_node_binding: key.node_id.is_some(),
                          },
                          &tele);
    }
    Ok(Json(KeyResponse::from(key)))
}
```

**Erasure service public API (`backend/src/services/telemetry_erasure_service.rs`):**

```rust
pub struct TelemetryErasureService {
    db: mongodb::Database,
    telemetry: Arc<TelemetryClient>,
}

impl TelemetryErasureService {
    /// Called from handlers/users.rs delete-account flow BEFORE deleting user row.
    pub async fn enqueue(&self, user_id: &str) -> Result<ObjectId, AppError>;

    /// Spawned from main.rs at startup. Polls the collection every 30s,
    /// takes up to 16 pending jobs, calls `TelemetryClient::delete_person`,
    /// exponential backoff (2s, 4s, 8s, 16s, 32s), dead-letter after 5 attempts.
    pub fn spawn_worker(self: Arc<Self>);
}
```

| Handler file | Events |
|---|---|
| `handlers/auth.rs` | `auth.logged_in`, `auth.logged_out`, `auth.token_refreshed`, `auth.password_reset_requested`, `auth.password_reset_completed`, `user.signed_up` |
| `handlers/users.rs` | `user.deleted` (emit, then enqueue erasure, then delete user) |
| `handlers/mfa.rs` | `mfa.enrollment_completed`, `mfa.challenge_succeeded`, `mfa.challenge_failed` |
| `handlers/keys.rs` | `key.created`, `key.deleted` |
| `handlers/user_services_handler.rs` + connection handlers | `service.connected`, `service.disconnected` |
| `handlers/user_endpoints.rs` | `endpoint.updated`, `endpoint.deleted` |
| `handlers/catalog.rs` | `catalog.browsed`, `catalog.entry_viewed`, `catalog.endpoints_fetched` |
| `handlers/api_keys.rs` | `api_key.created`, `api_key.rotated`, `api_key.deleted` |
| `handlers/agent_bindings.rs` | `agent_binding.created`, `agent_binding.deleted` |
| `handlers/approvals.rs` | `approval.requested`, `approval.decided`, `approval.expired` |
| `handlers/nodes.rs` + `handlers/admin_nodes.rs` + `handlers/node_ws.rs` | `node.registered`, `node.connected`, `node.disconnected`, `node.deleted` |
| `handlers/channel_*.rs` | `channel.bot_registered`, `channel.message_received` (10% sampled, per-event hash), `channel.reply_sent` (10% sampled) |
| `handlers/mcp.rs` | `mcp.session_started`, `mcp.session_ended` |
| `handlers/ssh*.rs` | `ssh.tunnel_opened`, `ssh.tunnel_closed` |
| `handlers/oauth.rs` | `oauth.token_issued` |
| `handlers/notifications.rs` | `notification.channel_linked`, `notification.channel_unlinked` |
| `handlers/admin_*.rs` | `admin.user_suspended`, `admin.service_created`, etc. |
| `handlers/proxy.rs` | `proxy.error` (100%). No `proxy.request` in M1 (cardinality risk; revisit when a specific question needs it). |
| `mw/rate_limit.rs` | `api.rate_limited` |

### 5.2 Frontend — autocapture + taxonomy-driven `ui.*`

Autocapture (`$pageview`, `$autocapture`, `$pageleave`, `$exception`) is the catch-all for every interaction. On top, emit named `ui.*` events using the category taxonomy below — one event covers all instances of the pattern via props, rather than 65 per-CTA names.

**Categories** (emit per CTA that fits):

| Event | Props | When |
|---|---|---|
| `ui.dialog_opened` | `dialog_id`, `entry_point` | Flow/modal/wizard opened; BE doesn't see it |
| `ui.dialog_step_completed` | `dialog_id`, `step`, `total_steps` | Wizard progression |
| `ui.dialog_abandoned` | `dialog_id`, `final_step`, `duration_ms` | Closed without completing |
| `ui.provider_connect_initiated` | `provider_slug`, `method` (`oauth` / `device_code` / `api_key`) | Before OAuth redirect / device-code start |
| `ui.secret_copied` | `secret_type`, `context` | Copy-to-clipboard of API key / client_secret / ca_key / curl |
| `ui.inline_edit_started` | `domain`, `field` | Edit mode entered; may be abandoned |
| `ui.list_filtered` | `list`, `filter`, `result_count` | Filter/search applied |
| `ui.nav_target_opened` | `target`, `source` (`sidebar` / `breadcrumb` / `tab`) | Navigation click beyond autocapture granularity |
| `ui.docs_opened` | `page` / `url_domain` | External docs / integration guide link |
| `ui.destructive_confirmed` | `domain`, `action` — enum: `delete` / `revoke` / `rotate` / `suspend` / `unsuspend` / `disconnect` / `wipe` | Confirm step on a destructive or irreversible operation (admin user-suspend lives here) |
| `ui.decision_made` | `domain`, `decision` — enum: `approve` / `deny` / `skip` / `defer`; plus `decision_ms` (view→tap latency) | User made an affirmative decision. Mobile approval approve/deny, revocation decisions, etc. Complements the backend `approval.decided` event: backend sees the outcome, this captures client-side intent + latency |
| `ui.preference_toggled` | `name`, `value` | Theme / dense-table / any client-side toggle |

**Enforcement:** every `onClick`/`onSubmit` added in a PR must either emit one of the named categories above or have a `// autocapture:ok` inline comment. Reviewer rejects anything else. Fewer events, consistent properties, one-shot-able.

**Extending the taxonomy:** each event's prop enum is enforced by the schema in code (§6). If a new CTA doesn't fit an existing enum value (e.g. a new destructive action like `purge`), add the value to both the taxonomy table above and the schema file in the same PR. Do not loosen an enum to "any string" — the allowlist is what keeps PII and cardinality bombs out.

**`frontend/src/lib/telemetry-schema.ts` — discriminated union (write first):**

```ts
// One variant per category. Unknown event name is a compile error.
export type UiEvent =
  | { name: 'ui.dialog_opened'; props: { dialog_id: DialogId; entry_point: string } }
  | { name: 'ui.dialog_step_completed'; props: { dialog_id: DialogId; step: number; total_steps: number } }
  | { name: 'ui.dialog_abandoned'; props: { dialog_id: DialogId; final_step: number; duration_ms: number } }
  | { name: 'ui.provider_connect_initiated'; props: { provider_slug: string; method: ConnectMethod } }
  | { name: 'ui.secret_copied'; props: { secret_type: SecretType; context: SecretContext } }
  | { name: 'ui.inline_edit_started'; props: { domain: Domain; field: string } }
  | { name: 'ui.list_filtered'; props: { list: string; filter: string; result_count: number } }
  | { name: 'ui.list_searched'; props: { list: string; filter: string; result_count: number } }
  | { name: 'ui.nav_target_opened'; props: { target: string; source: 'sidebar' | 'breadcrumb' | 'tab' } }
  | { name: 'ui.docs_opened'; props: { page: string; url_domain?: string } }
  | { name: 'ui.external_link_opened'; props: { url_domain: string } }
  | { name: 'ui.destructive_confirmed'; props: { domain: Domain; action: DestructiveAction } }
  | { name: 'ui.decision_made'; props: { domain: Domain; decision: Decision; decision_ms: number } }
  | { name: 'ui.preference_toggled'; props: { name: string; value: string | boolean } };

export type DestructiveAction = 'delete' | 'revoke' | 'rotate' | 'suspend' | 'unsuspend' | 'disconnect' | 'wipe';
export type Decision = 'approve' | 'deny' | 'skip' | 'defer';
export type Domain = 'keys' | 'services' | 'approvals' | 'admin' | 'auth' | 'settings' | /* ... fixed enum */;
// SecretType, SecretContext, ConnectMethod, DialogId all narrow string unions.
```

**`frontend/src/lib/telemetry.ts` — client wrapper (public API):**

```ts
interface InitArgs { dsn: string | undefined; host: string | undefined; shareBack: boolean; consent: boolean }

let inited = false;  // StrictMode guard (§4.idempotency)

export function initTelemetry(args: InitArgs): void;
// Resolves DSN per §3 precedence. If consent=false or DSN resolved is empty, returns without calling the vendor SDK init. Callers never know which vendor is inside.

export function identify(userId: string): void;
export function reset(): void;
export function capture(event: UiEvent): void;  // type-safe: event.name + event.props compile-checked
export function captureException(err: unknown): void;
```

**`frontend/src/stores/consent-store.ts` — Zustand store (public API):**

```ts
interface ConsentState {
  enabled: boolean;          // user's choice
  asked: boolean;            // have we prompted?
  setConsent(enabled: boolean): void;  // persists to localStorage + re-initializes the telemetry wrapper
}
export const useConsent: StoreApi<ConsentState>;
```

**Call pattern — inside a component:**

```tsx
const openAddKey = () => {
  capture({ name: 'ui.dialog_opened', props: { dialog_id: 'add_key', entry_point: 'keys_list_header' } });
  setOpen(true);
};
```

### 5.3 Mobile — RN autocapture-equivalent + `mobile.*` + `ui.mobile_*`

Autocapture via `posthog-react-native` captureAppLifecycleEvents + `$pageview`. Session replay off. `beforeSend` drops deep-link URLs with tokens. `customAppProperties` strips `$device_name` (iOS device name often contains user's first name) and `$device_id`.

**Device-side `mobile.*`:**

| Event | Properties | Observable? |
|---|---|---|
| `mobile.deep_link_opened` | `link_type` (`challenge` / `other`) — token stripped | App-open only |
| `mobile.approval_viewed` | `service_slug`, `mode` | Yes |
| `mobile.push_received` | `type`, `app_state` (`foreground` / `background`) | Foreground + background only. `quit` state is **not** observable from JS in Expo managed workflow; treat push delivery proof as server-side or native, not PostHog. |
| `mobile.biometric_prompted` | `reason` (`app_open` / `approval_decision`) | Yes |
| `mobile.biometric_result` | `reason`, `outcome` (`success` / `failed` / `cancelled` / `unavailable`) | Yes |

**`ui.mobile_*` CTAs** — same category taxonomy as §5.2 with `mobile_` prefix. Actual feature modules on disk: `mobile/src/features/{auth, nyx, activity, account, legal}`. (Challenge list / history UI lives in `activity`, not `nyx`.)

**`mobile/src/lib/telemetry-schema.ts` — mirrors §5.2 shape with `MobileEvent` discriminated union** (includes `mobile.*` device events + `ui.mobile_*` CTAs). One type file, same discriminated-union pattern — adding a new event is a compile error until the schema opens a slot for it.

**`mobile/src/lib/telemetry.ts` — client wrapper (public API):**

```ts
interface InitArgs {
  dsn: string | undefined;         // from Constants.expoConfig.extra.TELEMETRY_DSN
  host: string | undefined;        // from Constants.expoConfig.extra.TELEMETRY_HOST
  shareBack: boolean;              // from extra.NYXID_SHARE_ANALYTICS === 'true'
  consent: boolean;                // from Settings / consent store
}

let inited = false;  // idempotency guard

export function initTelemetry(args: InitArgs): void;
export function identify(userId: string): void;
export function reset(): void;
export function capture(event: MobileEvent): void;
export function captureException(err: unknown): void;
```

**Consent store** — same Zustand pattern as FE, backed by **AsyncStorage** (consent state is policy, not secret — parity with FE `localStorage`; SecureStore would be overkill for a single boolean): `mobile/src/lib/consent.ts` exports `useMobileConsent` with `{ enabled, asked, setConsent(bool) }`.

**Wiring point — `mobile/src/app/App.tsx`:**

```tsx
function RootShell() {
  const { isRestoring } = useAuthSession();
  const consent = useMobileConsent();
  useEffect(() => {
    if (isRestoring) return;
    initTelemetry({
      dsn: Constants.expoConfig.extra.TELEMETRY_DSN,
      host: Constants.expoConfig.extra.TELEMETRY_HOST,
      shareBack: Constants.expoConfig.extra.NYXID_SHARE_ANALYTICS === 'true',
      consent: consent.enabled,
    });
  }, [isRestoring, consent.enabled]);
  // ... rest of shell
}
```

**Identify + reset wiring — `mobile/src/features/auth/AuthSessionContext.tsx`:** on `signInWithSession` success, after `/users/me` allowlist gate passes, pull `userId` off the stored session (read via `loadStoredAuthSession()` which now returns it) and call `identify(userId)`. Every sign-out path (explicit, 401 invalidation, SecureStore wipe, account switch) calls `reset()` before clearing state.

### 5.4 CLI

| File | Change |
|---|---|
| `cli/src/telemetry.rs` (new) | Fire-and-forget reqwest POST to `/capture/`, 1s timeout. Reads DSN per precedence in §3. Anon-ID at `~/.nyxid/profiles/{profile}/anon_id` (created lazily, used until `identify()`). |
| `cli/src/commands/telemetry.rs` (new) | `enable` / `disable` / `status` — manages `~/.nyxid/config.toml` `[telemetry]` section. `disable` deletes the anon_id file. |
| `cli/src/cli.rs` | Register `telemetry` subcommand. |
| `cli/src/main.rs` | Wrap command dispatch; emit `cli.command_invoked{command_group, subcommand, exit_code, duration_ms, profile, os, arch}` on exit. First-run interactive consent prompt (skipped if env var or config.toml already set). |
| `cli/src/api.rs` | Attach `X-NyxID-Client: cli` + `X-NyxID-Client-Version` on every API call. |
| `cli/src/auth.rs` | Identity plumbing **already landed** (§4.1). Pending: `run_login` calls `telemetry_client.identify(user_id)` via the wrapper; `run_logout` calls `telemetry_client.reset()` and deletes the anon_id file alongside the existing token cleanup. |

**`cli/src/telemetry.rs` — one file, contains client + inline schema + consent (public API):**

```rust
pub struct TelemetryClient {
    dsn: String,
    host: String,
    distinct_id: String,    // user_id from ~/.nyxid/profiles/{profile}/user_id,
                            // or anon UUID from ~/.nyxid/profiles/{profile}/anon_id if pre-login
    http: reqwest::Client,  // 1s timeout
    cli_version: &'static str,
}

impl TelemetryClient {
    /// Resolves DSN + consent per §3 precedence. None = hard-off.
    pub fn init(profile: Option<&str>) -> Option<Self>;

    /// Awaits the POST up to TRACK_TIMEOUT_MS (1s). Runs inline — the
    /// earlier fire-and-forget `tokio::spawn` design dropped ~100% of
    /// events because `#[tokio::main]`'s runtime teardown cancelled the
    /// spawned task before the TCP handshake completed.
    pub async fn track(&self, event: CliEvent);

    /// Associates the currently-active anon identity with `user_id` for
    /// future events. Reads the current anon_id from disk internally.
    /// Called from run_login() on success, before anon_id is deleted.
    /// Translates to whatever merge verb the underlying vendor needs —
    /// PostHog: POST an `$identify` event with `$anon_distinct_id`;
    /// Mixpanel would call `alias(anon, user)` then `identify(user)`;
    /// both are invisible to the caller.
    pub fn identify(&self, user_id: &str);

    /// Clears the local anon identity. Called from run_logout() AND from
    /// the telemetry subcommand on disable.
    pub fn reset(&self);
}

/// Inline schema — single enum, same pattern as backend.
pub enum CliEvent {
    CommandInvoked {
        command_group: &'static str,
        subcommand: &'static str,
        exit_code: i32,
        duration_ms: u64,
        profile: Option<String>,
        os: &'static str,
        arch: &'static str,
    },
    // (Backend is the canonical emitter for business events; CLI only owns this one.)
}
```

**`cli/src/telemetry/consent.rs` — resolves the 7-level precedence from §3:**

```rust
pub enum ConsentSource {
    EnvVarOff,         // precedence 1: NYXID_TELEMETRY=off
    EnvVarOn,          // precedence 2: NYXID_TELEMETRY=on
    ConfigEnabled,     // precedence 3: config.toml enabled=true
    ConfigDeclined,    // precedence 4: config.toml enabled=false + asked=true
    FirstRunPending,   // precedence 5/6/7 collapsed: config.toml asked=false OR
                       // no config file exists at all. Either case needs a prompt
                       // on an interactive TTY; default off on non-TTY.
}

pub struct ConsentState {
    pub enabled: bool,              // whether telemetry should fire
    pub source: ConsentSource,      // for `nyxid telemetry status` to print
    pub persisted: bool,            // true iff choice is already in config.toml
    pub needs_prompt: bool,         // true iff source == FirstRunPending AND stdin is a TTY
}

pub fn resolve_consent(profile: Option<&str>) -> ConsentState;
// Reads NYXID_TELEMETRY env + ~/.nyxid/config.toml in §3 precedence order.
// Pure function. No prompting. Missing config file is treated identically
// to `asked=false` — both resolve to `source = FirstRunPending`, `enabled = false`,
// `persisted = false`. `needs_prompt` is computed here via `atty::is(Stream::Stdin)`
// so the caller doesn't need to check TTY.

pub fn prompt_if_needed_interactive(profile: Option<&str>, state: &mut ConsentState) -> Result<()>;
// No-op when state.needs_prompt == false. Otherwise prints the consent
// question to stderr, reads y/N from stdin, writes `{enabled, asked=true}`
// to config.toml (creating the file if it didn't exist), updates state in
// place. Prompt refusal does NOT bail out of whatever command the user was
// running — returns Ok() either way.
```

**`cli/src/main.rs` wiring:**

```rust
#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let start = Instant::now();

    // 1. Resolve consent (pure, no I/O beyond config read)
    let mut consent = resolve_consent(cli.profile.as_deref());
    // 2. Prompt only on interactive TTY + asked=false, silently skip otherwise
    let _ = prompt_if_needed_interactive(cli.profile.as_deref(), &mut consent);

    let ph = if consent.enabled {
        TelemetryClient::init(cli.profile.as_deref())
    } else { None };

    let result = run_command(&cli).await;

    if let Some(ph) = &ph {
        ph.track(CliEvent::CommandInvoked {
            command_group: cli.command_group(),
            subcommand: cli.subcommand(),
            exit_code: if result.is_ok() { 0 } else { 1 },
            duration_ms: start.elapsed().as_millis() as u64,
            profile: cli.profile.clone(),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        });
    }

    if result.is_ok() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
```

## 6. PII / redaction rules (non-negotiable)

**Structured allowlist.** Each event has a declared prop shape (TS types on web/mobile; Rust struct on BE/CLI). Unknown keys dropped at send. Schema lives in code: `frontend/src/lib/telemetry-schema.ts`, `mobile/src/lib/telemetry-schema.ts`, `backend/src/telemetry/schema.rs`, `cli/src/telemetry.rs` (inline). CI grep scans `capture(...)` and `track(...)` calls across the repo and fails if the event name passed isn't declared in the corresponding schema file. CI also fails if the vendor SDK (currently `posthog-js` / `posthog-react-native` / raw `/capture/` POSTs) is imported or constructed outside the four `telemetry.{ts,rs}` wrapper files — keeps the hot-swap surface contained.

**Never capture:**
- Raw URLs with query strings or path IDs (use route patterns: `/api/v1/keys/:id`)
- Tokens, API keys, bearer values, session IDs, OTP, passwords
- Email addresses (split; use `email_domain` only if needed)
- Free-text user input (message bodies, search queries, form fields)
- Request/response bodies, headers (except the enum-valued `X-NyxID-Client`), cookies
- Raw channel conversation IDs / user handles (hash + truncate to 8 hex chars)
- Full exception messages where they may contain user input; scrub before ship
- Environment variable values, argv, CLI flag values

**Egress scrubber** — last-chance regex pass on every surface before `POST /capture/`:
- URL-with-query → `[URL_REDACTED]`
- Bearer / Authorization → `[AUTH_REDACTED]`
- Email → `[EMAIL_REDACTED]`
- UUID in message text (not structured ID fields) → `[UUID_REDACTED]`
- Project-prefix tokens (`nyx_\w+`, `nyxid_\w+`, `sk-\w+`, `ghp_\w+`, `phc_\w+`) → `[TOKEN_REDACTED]`

**Autocapture hardening (FE + Mobile):** `mask_all_text: true`, `mask_all_element_attributes: true`. CSS denylist: `input[type="password"]`, `input[name*="password"]`, `input[name*="secret"]`, `input[name="code"][autocomplete="one-time-code"]`, `input[name*="otp"]`, `[data-sensitive]`, `[data-api-key]`, `[data-credential]`. `before_send` drops entire events on paths: `/verify-email/*`, `/reset-password/*`, `/oauth/callback`, `/approve/*`.

### 6.5 Part 2 Leftovers

Events and sites defined in the §6 schema that Part 2 could not deliver
cleanly without modifying production code paths beyond telemetry. Kept
here as a punch list for follow-up work. Each follow-up lands in its own
PR with its own review scope — telemetry-driven refactors should not hide
inside an instrumentation sweep.

**Variant-level leftovers** (variant has zero live emit sites after Part 2):

| Event | Blocker | Chunk of origin | Follow-up |
|---|---|---|---|
| `auth.password_reset_requested` | `auth_service::initiate_password_reset` returns `Option<String>`; no `user_id` exposed to the handler. Emitting with a synthetic distinct_id would pollute cardinality. | BE-Auth | Change return to `Option<(token, user_id)>`; handler emits only on `Some`. HTTP response shape unchanged. |
| `auth.password_reset_completed` | `auth_service::reset_password` returns `AppResult<()>`; the handler does not get the `user_id` of the account whose password was reset. Adding a telemetry-only DB read would be a redundant query. | BE-Auth | Change return to `AppResult<String>` (the `user_id`); handler emits on success. HTTP response shape unchanged. |
| `api.rate_limited` | `mw/rate_limit.rs` is registered via `from_fn(...)`; middleware body cannot access `AppState.telemetry` without a `from_fn_with_state` conversion (which touches `main.rs`). | BE-Proxy-Infra | Convert middleware registration in `main.rs` to `from_fn_with_state(state, rate_limit_middleware)` and accept an `AppState` arg in the middleware body. |
| `oauth.token_issued` | All four `/oauth/token` grant-type branches lack a `user_id` in handler scope: `exchange_authorization_code` returns only tokens; `refresh_tokens` returns `IssuedTokens` (no `user_id`); `client_credentials` has only `client_id` (`service_account_service::authenticate_client_credentials` returns `ClientCredentialsResponse` without `service_account_id`); the token-exchange branch is covered separately below. | BE-AdminOps | Either (a) expose `user_id` / `service_account_id` on the relevant service response structs; or (b) decode the issued JWT before returning and emit from there. |
| `node.credential_configured` | Emitted from CLI (`nyxid node credentials` command), not backend. | (CLI follow-up) | Add emission at CLI command entry in a CLI-coverage PR. |

**Call-site-level leftovers** (variant still emits from other sites, but one
or more sites are blocked):

| Variant / site | Blocker | Chunk of origin | Follow-up |
|---|---|---|---|
| `user.signed_up` — social auth path | `social_auth_service::find_or_create_user` returns bare `User` without outcome kind. Cannot distinguish first-time signup from returning login for social providers without a racy `created_at` check. `auth.logged_in` still fires for social; the signup variant still fires from the email path (Part-1 live). | BE-Auth | Change return to `(User, SocialOutcomeKind)` where `SocialOutcomeKind = NewUser \| ReturningUser \| LinkedToExisting`. Also updates caller in `social_token_exchange_service`. |
| `auth.token_exchanged` — social token-exchange branch (`subject_token_type = id_token` with `provider`) | `SocialTokenExchangeResponse` strips `user_id_for_audit` before returning to the handler. `TokenExchangeResponse` (delegation branch) correctly includes `user_id`, so that site ships cleanly. | BE-AdminOps | Add `user_id` to `SocialTokenExchangeResponse`; handler reads it for distinct_id. |

**Degraded / best-effort emissions** (variant emits, but with reduced fidelity
that could not be improved without a refactor):

| Variant | Degradation | Chunk of origin | Follow-up |
|---|---|---|---|
| `node.disconnected` | Reason granularity: reader teardown can observe `"client_close"` / `"error"` only. Admin-force and heartbeat-timeout reasons are not distinguishable, because `NodeWsManager::disconnect_connection()` removes the connection record before teardown inspects it. | BE-Nodes | Refactor `disconnect_connection()` to signal close only (not remove the entry); reader owns removal + reason pickup. |
| `node.connected.profile` | Field is always emitted as `"unknown"` because the server never learns the node's profile name — profile is a CLI-side concept not sent through the WS handshake. | BE-Nodes | Thread `profile` through the node agent's WS handshake metadata; extend `NodeMetadata` and populate in `register_node`. |
| `mcp.session_ended` / `ssh.tunnel_closed` (abrupt disconnect) | Events fire on normal close paths. Process panic / socket drop may miss the emit. No `Drop` trick — complexity outweighs marginal coverage. | BE-ProductOps | Accepted. Do not chase unless a product question specifically needs it. |

**Do not widen this table during Part 2 implementation.** If a new blocker
surfaces at emit time, stop and ask — do not invent a workaround inline.

#### Mobile consent UI not wired (known gap)

`useMobileConsent().enabled` persists via AsyncStorage and gates
`initTelemetry()` in `AuthSessionContext`, but no in-app UI currently
flips it from the default `false` to `true`. That means on a fresh
install the client-side `capture()` calls added in Part 2 short-circuit
to no-ops, and mobile telemetry never turns on for real users.

Closing this requires either (a) a first-launch consent prompt, (b) an
in-Settings toggle that calls `setConsent(true)`, or (c) both. The
privacy-policy copy in `mobile/src/features/legal/PrivacyPolicyScreen.tsx`
already references "Settings" as the on/off control, so a Settings
toggle is the minimum shippable UI.

The backend-side mobile emissions are unaffected: they fire based on
server-side telemetry config, not the mobile client flag.

Follow-up: ship the consent toggle (Settings > Privacy > "Help improve
NyxID") in the next mobile release before treating client-side mobile
telemetry coverage as live.

#### Mobile-to-backend consent propagation (known gap)

The mobile client's local opt-out (the `useMobileConsent()` toggle) suppresses
client-side captures and tears down the PostHog client on that device. It does
NOT reach the backend: mobile HTTP requests identify themselves as
`X-NyxID-Client: mobile` regardless of consent, and backend handlers emit their
usual telemetry events on those requests. That means backend events
(`notification.device_registered`, `approval.decided`, `auth.logged_in`, etc.)
still fire for a mobile user who opted out locally.

This is a structural gap, not a specific emit-site fix. Closing it requires the
mobile HTTP client to send an explicit `X-NyxID-Telemetry-Consent: on|off`
header and the backend's `emit_event` helper to short-circuit when surface is
`mobile`/`agent` (originating from a mobile device) and consent is `off`. Both
ends need to ship in the same change; partial plumbing would be worse than the
current state.

Follow-up: design an end-to-end consent signal that works for hosted NyxID
(user chose opt-in/out) AND self-hosters (operator-level default). Until then,
the mobile app's in-app copy should make clear that opting out only stops
client-side captures and that server-side events tied to the user's account
still occur.

## 7. Infrastructure + legal requirements (operator owns)

These are non-code prerequisites for turning on production telemetry:

- PostHog production project provisioned (default region: US — the Vite/Rust/TS constants default to `us.i.posthog.com`; set `NYXID_TELEMETRY_HOST=https://eu.i.posthog.com` to target PostHog EU)
- PostHog **separate** share-back project provisioned (for `NYXID_SHARE_ANALYTICS=true`)
- DPA signed with PostHog (required: user_id as distinct_id = pseudonymous PII)
- Consent mechanism chosen + copy written: **opt-in banner on first FE/Mobile visit + Settings toggle + CLI first-run prompt** (default: opt-in banner; can be relaxed to legitimate-interest-with-opt-out if Legal signs a DPIA)
- `privacy.tsx` rewritten — current text asserts "no third-party tracking cookies or analytics services," which becomes false the moment telemetry ships
- App Store Data Collection labels + Play Store Data Safety form updated
- In-app "About → Privacy" screen in `mobile/src/features/legal/`
- `docs/TELEMETRY.md` at repo root — what gets captured, retention, how to opt out on each surface, what `SHARE_ANALYTICS=true` sends to the share-back project
- PostHog project retention set to 90 days raw / aggregates longer

## 8. Backend CORS — required change (not yet landed)

`backend/src/main.rs:550-558` currently whitelists `Content-Type, Authorization, Accept, Origin, Cookie, X-User-Email, X-User-Display-Name, X-API-Key`. **Browsers will CORS-block every `X-NyxID-Client` / `X-NyxID-Client-Version` request** until this list includes both. Add them as part of the backend telemetry PR, before FE/Mobile clients start sending the headers.

## 9. File map — every file touched, grouped by surface

### Frontend
- `frontend/src/lib/telemetry.ts` (new) — hardened init + module-level inited guard
- `frontend/src/lib/telemetry-schema.ts` (new) — event + prop shapes, CI-grepped
- `frontend/src/main.tsx` — call `initTelemetry()` after `checkAuth()` resolves (gated on consent)
- `frontend/src/stores/auth-store.ts` — `identify(user_id)` on login/register/OAuth-callback success; `reset()` on logout
- `frontend/src/stores/consent-store.ts` (new) — localStorage-backed consent state
- `frontend/src/components/consent-banner.tsx` (new) — opt-in UI
- `frontend/src/lib/api-client.ts` — attach `X-NyxID-Client: ui` on every request when telemetry is active. (FE does not send `X-NyxID-Client-Version`; CLI + mobile still do, using their native version strings.)
- `frontend/src/pages/privacy.tsx` — rewrite with telemetry disclosure
- `frontend/Dockerfile` — no build-time telemetry env. DSN / host / share-back flag are fetched at runtime from the backend's `/api/v1/public/config`.

### Backend
- `backend/src/telemetry/mod.rs` (new) — `TelemetryClient`, reqwest fire-and-forget, 2s timeout, bounded mpsc for backpressure
- `backend/src/telemetry/scrub.rs` (new) — `scrub_string`, `scrub_value` (sampling helper deferred to the first sampled-emission PR)
- `backend/src/telemetry/schema.rs` (new) — Rust struct per event
- `backend/src/mw/telemetry.rs` (new) — derive `surface`, stash `TelemetryContext`
- `backend/src/routes.rs` — register middleware before auth
- `backend/src/config.rs` — parse three env vars
- `backend/src/main.rs` — spawn erasure worker; **update CORS allowlist at `:550-558`**
- `backend/src/handlers/*.rs` — emit events per §5.1 matrix (`user.signed_up` lives in `handlers/auth.rs`, not `users.rs`)
- `backend/src/handlers/users.rs` — `delete_current_user_cascade` enqueues erasure job before deleting
- `backend/src/mw/rate_limit.rs` — emit `api.rate_limited`
- `backend/src/models/telemetry_erasure_job.rs` (new) — `{job_id, user_id, status, attempts, last_error, created_at}`
- `backend/src/services/telemetry_erasure_service.rs` (new) — enqueue + drain loop with exponential backoff, dead-letter after 5 attempts

### CLI
- `cli/src/telemetry.rs` (new) — reqwest client, DSN precedence resolver, anon-ID file
- `cli/src/telemetry/consent.rs` (new) — `ConsentState`, `ConsentSource` enum, `resolve_consent()`, `prompt_if_needed_interactive()`
- `cli/src/commands/telemetry.rs` (new) — `enable | disable | status`
- `cli/src/cli.rs` — register subcommand
- `cli/src/main.rs` — wrap dispatch with `cli.command_invoked` + first-run consent
- `cli/src/api.rs` — attach `X-NyxID-Client: cli` + version
- `cli/src/auth.rs` — ✅ identity plumbing landed; pending: `run_login` calls wrapper `identify(user_id)`, `run_logout` calls wrapper `reset()` + deletes anon_id

### Mobile
- `mobile/src/lib/telemetry.ts` (new) — hardened init, session replay off, device-ID scrub, deep-link `beforeSend`
- `mobile/src/lib/telemetry-schema.ts` (new) — TS types
- `mobile/src/lib/consent.ts` (new) — Zustand-over-AsyncStorage consent store (`useMobileConsent`)
- `mobile/src/app/App.tsx` — init after `AuthSessionContext` resolves (gated on consent); identify on restored session
- `mobile/src/features/auth/AuthSessionContext.tsx` — `identify` on login; `reset()` on **every** sign-out path (explicit, 401 invalidation, SecureStore wipe, account switch)
- `mobile/src/features/auth/AuthHomeScreen.tsx` — `identify(user_id)` on login/register/social success (session already persisted by `persistAuthSession`)
- `mobile/src/lib/auth/jwt.ts` — ✅ landed
- `mobile/src/lib/auth/sessionStore.ts` — ✅ landed (single-writer, read-only load, full orphan cleanup)
- `mobile/src/lib/api/http.ts` — attach `X-NyxID-Client: mobile` + `X-NyxID-Client-Version`
- `mobile/src/features/{auth,nyx,activity,account,legal}/**` — `ui.mobile_*` events per §5.2 taxonomy
- `mobile/src/app/linking.ts` — emit `mobile.deep_link_opened`
- Push handler + biometric wrappers — `mobile.push_received`, `mobile.biometric_*`
- Settings screen — telemetry toggle flip-off: `reset()` + anon-ID clear + `setConsent(false)` gates future `init()` calls
- About → Privacy screen in `features/legal/`
- `mobile/app.json` — `expo.extra` + `EXPO_PUBLIC_*` vars (not `app.config.ts` — doesn't exist)

### Docs
- `docs/TELEMETRY.md` (this file — file-by-file implementation map + event catalog + opt-out per surface)
- `.env.example` (new or extended)

## 10. Open questions / decisions

| Q | Current answer |
|---|---|
| OSS Docker / Expo release ships with `SHARE_ANALYTICS=true`? | No. Default off; documented opt-in only. |
| PostHog Cloud vs self-hosted for NyxID's projects? | Start on PostHog Cloud (US default; EU available via HOST override). Self-host when volume justifies ops. |
| Consent: opt-in banner vs legitimate-interest + opt-out? | Opt-in banner (safest default). Relax if Legal greenlights LIA + DPIA. |
| Copy the parent `TELEMETRY_PLAN.md` into the repo? | Resolved — this file is the in-repo implementation reference; the original draft lives outside the tree. |
| Mobile TestFlight / Android internal — same PostHog project as prod? | Separate project OR `environment=staging` prop on share-back. Pick before first TestFlight with telemetry. |
| Emit `proxy.request` sampled? | No. Revisit after first data review if a specific question needs it. |
| Emit backend pre-auth `anon:<ip_hash>` events? | No. Can't merge via `identify()`, creates undeletable orphans. |
| Pre-existing mobile npm peer-dep conflict (`react-native-reanimated@4.3.0` ↔ `react-native-worklets@0.7.2`) | Unrelated to this work. Install uses `--legacy-peer-deps`. Separate ticket. |
| Pre-existing CLI bug: `cli/src/auth.rs:176-179` — `save_tokens_for()` doesn't clear a stale `refresh_token` file when the new login returns no refresh token. Flagged by Codex third-pass review. | Unrelated to telemetry; user-auth hygiene issue. Separate ticket. |

## 11. Review history

- **2026-04-22 (first pass)** — Codex consult. Findings folded in: repo-path mismatches, `user.signed_up` handler location, CORS as hard gate, consent banner, drop backend pre-auth anon events, separate share-back project, StrictMode idempotency, session-restore identify, mobile multi-sign-out reset, PII/redaction schema, mobile feature module names, `mobile/app.json` vs `app.config.ts`.
- **2026-04-22 (identity plumbing landed)** — `cli/src/auth.rs` JWT sub extraction + user_id file; `mobile/src/lib/auth/{jwt.ts,sessionStore.ts}` userId field + SecureStore key. 156 + 25 CLI tests pass; mobile typecheck clean.
- **2026-04-22 (second pass)** — Codex consult resumed. Code bugs fixed: CLI save_tokens_for clears stale user_id on parse fail; CLI read_saved_user_id_for prefers JWT (canonical) over file (cache); mobile loadStoredAuthSession is read-only (no backfill race with clear); mobile orphan cleanup covers `user_id` + `expires_at`, not just `refresh_token`. Added CLI test for stale-cleanup. Added security note that JWT `sub` is unverified (telemetry-only, not identity proof).
- **2026-04-22 (doc restructure)** — Stripped daily progress tracker, verification plan, dashboards, review cadence. Doc is now a file-by-file map. Infrastructure + legal gates consolidated into §7 as operator-owned prerequisites.
- **2026-04-22 (third Codex pass — final gate)** — No blockers. Three ambiguities tightened: §1 vs §2 "one project" contradiction reworded to clarify production vs share-back; §3 CLI consent gained a full precedence ladder + config schema + non-TTY behavior; §5.2 taxonomy added `ui.decision_made{decision}` for approve/deny and expanded `ui.destructive_confirmed.action` enum to include `suspend/unsuspend/disconnect/wipe`. One pre-existing (non-telemetry) CLI bug noted in §10. Implementation green-lit.
- **2026-04-22 (utility shapes added)** — §5.1–5.4 now include concrete public-API signatures for every new module: backend `TelemetryEvent` enum + `TelemetryClient` + scrubber + middleware + erasure service; FE/Mobile discriminated-union schema + `capture()` + Zustand consent store; CLI `TelemetryClient` + inline schema + `resolve_consent` / `prompt_if_needed_interactive`. Plus implementation-ordering note (schema first, client second, middleware third, call sites last) to prevent retrofitted allowlists.
- **2026-04-22 (hot-swap contract)** — New §5.0 codifies vendor-neutrality as non-negotiable: module filenames (`telemetry.ts` / `telemetry.rs`, never `posthog.ts`), struct names (`TelemetryClient`), env vars (`*_TELEMETRY_DSN`, never `*_POSTHOG_*`), public API verbs (`init / identify / reset / capture / captureException` — universal across vendors). Renamed throughout doc. CI rule added: vendor SDK imports forbidden outside the four `telemetry.{ts,rs}` wrapper files. Swap path becomes: rewrite 4 files, zero caller changes.
- **2026-04-22 (fourth Codex pass — final-final gate)** — Three blockers caught and fixed: mobile wiring example still had `POSTHOG_KEY`/`POSTHOG_HOST` in `Constants.expoConfig.extra` (renamed to `TELEMETRY_DSN`/`TELEMETRY_HOST`); CLI `ConsentSource` enum was missing the `FirstRunPending` variant the 7-step ladder needs (added, plus `needs_prompt` computed flag on `ConsentState` so the caller doesn't need TTY detection); CLI public API exposed `create_alias(anon, user)` which leaked PostHog wire semantics (folded into `identify(user_id)` — wrapper reads current anon internally and hands merge to vendor invisibly). Also added missing file-map entries (`telemetry/scrub.rs`, `cli/src/telemetry/consent.rs`, `mobile/src/lib/consent.ts`) and resolved the mobile consent-storage ambiguity (AsyncStorage, not SecureStore).
- **2026-04-22 (fifth Codex pass — semantic correctness check)** — Caught one residual bug from pass-4 fix: `ConsentSource::Default` claimed "treat as FirstRunPending" but `needs_prompt` only triggered on literal `FirstRunPending`, so a fresh user with no config file would silently skip the first-run prompt. Collapsed `Default` into `FirstRunPending` (one variant, one semantic: "no user choice yet, prompt on TTY"). Updated §3 ladder steps 5 and 6 to treat "config file doesn't exist" identically to "config exists with asked=false". No blockers remaining. Ship.
- **2026-04-23 (implementation landed)** — Tier A (foundation files), Tier B (wiring), and Tier D (privacy / Dockerfile / mobile app.json / `.env.example`) complete across all four surfaces. Tier C landed the critical subset of backend events (`user.signed_up` + `auth.logged_in` + `auth.logged_out` in `handlers/auth.rs`; `key.created` + `key.deleted` in `handlers/keys.rs`; `user.deleted` + erasure-job enqueue in `handlers/users.rs`) plus CLI identity hooks (`run_login → identify`, `run_logout → reset`) and FE/Mobile identify on login + `reset()` on all sign-out paths. Remaining backend handler emissions (mfa, services/endpoints, catalog, api_keys/agent_bindings, approvals, nodes, channels, mcp/ssh/oauth/notifications/admin, proxy.error, api.rate_limited) + exhaustive FE/Mobile `ui.*` CTA sweep are follow-up PRs — the framework is in place and each new emission is a ~5-line additive edit in the handler/component. **No breaking changes:** with all `*_TELEMETRY_DSN` env vars unset (the default), `TelemetryClient::from_config` returns `None`, every `emit_event` / `capture` / `identify` call is a no-op, and runtime behavior is byte-identical to today. All four surfaces build green: backend `cargo check` clean, CLI 161/161 tests pass, FE `tsc -b` clean, mobile `npm run typecheck` 0 errors.
- **2026-04-23 (post-impl live verification)** — Verified end-to-end against a live PostHog testnet project: backend `user.signed_up`/`auth.logged_in`/`user.deleted` all 200 OK with matching distinct_id; GDPR erasure worker actually called PostHog's `/person/{id}/` DELETE API (visible as `$delete_person` audit events vendor-side); default-off produces zero outbound HTTP and zero log lines; CORS preflight accepts `X-NyxID-Client[-Version]` headers. Caught and fixed one bug live that static review did not surface: CLI `track()`/`identify()` used `tokio::spawn` fire-and-forget inside `#[tokio::main]`, but the runtime teardown on main-return cancelled the spawned tasks before the TCP handshake — ~100% event loss. Switched to bounded-wait `.await` with a 1s timeout; adds ~200-500ms to CLI runtime but events now reliably land.
- **2026-04-23 (host default flipped)** — Default `HOST` flipped from `eu.i.posthog.com` to `us.i.posthog.com` across backend/CLI/frontend/mobile. Rationale: PostHog's own default region for new signups is US, and operators were forced to set both `DSN` and `HOST` to be useful since the DSN came from a US project. After the flip, operators need exactly one env var (`NYXID_TELEMETRY_DSN`) in the common case. EU operators override via `NYXID_TELEMETRY_HOST=https://eu.i.posthog.com`. Community share-back constants (`NYXID_PUBLIC_TELEMETRY_HOST`) were already on US so unchanged.

### Dependencies added

| Surface | Package | Version | Purpose |
|---|---|---|---|
| CLI | `is-terminal` | `0.4` | TTY detection for first-run consent prompt. |
| Frontend | `posthog-js` | `^1.280.1` | Vendor SDK — imported only by `frontend/src/lib/telemetry.ts` (no caller references it, per §5.0 hot-swap contract). |
| Mobile | `posthog-react-native` | `^4.2.0` | Vendor SDK — imported only by `mobile/src/lib/telemetry.ts`. |
| Mobile | `@react-native-async-storage/async-storage` | `^2.1.2` | Persistence for the consent store (AsyncStorage, parity with FE localStorage). |
| Mobile | `zustand` | `^5.0.11` | Consent store state management (matches FE). |

All additions are additive; none replace or conflict with existing production deps. Mobile uses `--legacy-peer-deps` on install because of a pre-existing (unrelated) `react-native-reanimated@4.3.0` ↔ `react-native-worklets@0.7.2` peer-dep conflict that predates this work.

- **2026-04-23 (post-impl Codex pass — blocker fixes)** — Codex review after the implementation landed flagged three breaking-change violations and two correctness bugs. All fixed:
  1. **Unconditional `X-NyxID-Client` headers on FE/Mobile/CLI:** all three clients now gate the header on their respective telemetry-enabled signal (backend's `/public/config` for FE, `expo.extra.TELEMETRY_DSN` for mobile, `NYXID_TELEMETRY_DSN`/share-back for CLI). With everything unset (default), no new headers hit the wire.
  2. **CLI first-run consent prompt ran even with no DSN:** `main.rs` now short-circuits consent resolution and prompt when no DSN is configured. The prompt only fires on machines where telemetry could actually run.
  3. **Backend erasure-worker startup log line:** removed. `spawn_worker` returns silently when `telemetry` is `None`.
  4. **`delete_me` enqueue-failure path emitted dangling events:** now aborts the delete with an internal error if the erasure enqueue fails (when telemetry is on). No user row is deleted and no `user.deleted` event fires without a matching erasure job.
  5. **`auth.logged_in` emitted before session creation:** moved emission into each `client_mode` match arm, after `create_session` / `create_session_and_issue_tokens` returns Ok. A failing token-creation path no longer reports a successful login.
  6. **Mobile sign-in `session.userId` was unreliable:** `signInWithSession` now derives `userId` from the access token's JWT `sub` claim (via the existing `decodeJwtSub` helper) with a fall-back to `session.userId` if the caller set it, so the immediate post-sign-in `identify()` always fires.
  7. **Erasure worker docstring overstated the retry semantics:** updated to describe the actual behavior (uniform 30s re-poll cadence, no per-job backoff schedule, dead-letter after `MAX_ATTEMPTS` uniform ticks). The `MAX_ATTEMPTS × POLL_INTERVAL` envelope is documented.

Four-surface build still green after the fixes: backend `cargo check` + 161 tests pass, CLI 161 tests pass, FE `tsc -b` clean, mobile `npm run typecheck` 0 errors.

- **2026-04-23 (share-back symmetry + banner gate)** — A second post-impl Codex pass surfaced two more blockers. Fixed:
  8. **FE/Mobile share-back asymmetric with CLI:** header guards and init precedence only checked DSN; CLI already honored `DSN OR SHARE_ANALYTICS`. Added compiled-in `NYXID_PUBLIC_TELEMETRY_DSN` constants to `frontend/src/lib/telemetry.ts` and `mobile/src/lib/telemetry.ts` matching the backend/CLI pattern, and updated init to fall back to the public DSN when share-back is on + explicit DSN is empty. Header guards on `frontend/src/lib/api-client.ts` and `mobile/src/lib/api/http.ts` now also activate on share-back. All four surfaces now implement identical precedence: **explicit DSN > share-back → public DSN > off**.
  9. **FE consent banner rendered unconditionally:** `ConsentBanner` now short-circuits when the backend's `/public/config` response reports no `telemetry_dsn` and `telemetry_share_analytics` is not `true`. Default-off FE deploys produce no new DOM that wasn't there pre-telemetry.

After these fixes, Codex's final verdict on the implementation: **Clean. Ship.** — confirmed at the end of the third post-impl review pass.
