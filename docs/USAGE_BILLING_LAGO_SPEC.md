# Implementation Spec: Usage Metering & Billing via Lago

> **Status:** v3 — implementation-ready. Implements [ADR-014 Rev 2](./ADR-014-usage-billing-lago.md).
> **Date:** 2026-06-23
> **Read first:** ADR-014 (decisions + the 12 Hard Requirements that gate "billable"). This spec turns
> those decisions into concrete modules, collections, signatures, insertion points, and a test plan.
> All file:line references are against the current tree and may drift — re-grep before editing.

> **Revision history.** v1 = first draft. v2 = pressure-tested by Claude + Codex GPT-5.5 + OpenCode
> GLM-5.2 (unanimous: architecture sound, not start-P1-ready). **v3 = those review resolutions
> (R1–R12) promoted inline** — the spec now reads as one coherent doc. Material corrections from the
> review: a metadata-only resolution phase precedes the gate (the old "gate in `execute_proxy_inner`"
> was not implementable); resale keys on a `CredentialClass` enum (not `has_server_credential`); settle
> debits locally and is idempotent (the repo has no Mongo transactions); `Forwarded` rows are never
> auto-freed; connection-shaped metering uses per-flush `transaction_id`s; Lago 422 is branched by
> sub-code; there is no `billable` field (it is `resale_billable` + plan-level platform).

## 0. Guiding constraints (from the ADR + reviews)

- **Build the meter before billing.** The unified meter ships and is verified *before* any service can
  charge (Hard req. 1, 12).
- **Money correctness over convenience.** Durable-before-forward, stable per-request/per-layer
  `transaction_id`, reserve-then-true-up funding with an immediate local debit, bidirectional Lago
  reconciliation, idempotent settle (no DB transactions exist in this repo).
- **Thin Lago client, NyxID-owned gate/ledger.** No rating/invoicing/dunning in NyxID. NyxID may cache
  Lago's rate card *read-only* (approximate) only to size reservations and bound the cap.
- **Cross-instance, no per-process state.** All billing state in MongoDB (Oracle-queue precedent,
  CLAUDE.md §11). Atomic ops via `find_one_and_update`.
- **Two layers.** *Resale* (catalog-level, only when NyxID supplies the credential) vs *platform*
  (plan-level, any proxied request). Independent line items, independent `transaction_id`s.
- **Metadata before secrets.** Entitlement and owner resolution run on a metadata-only route context
  that does **not** decrypt credentials; credential decryption + reservation happen just before the
  send, per path.

## 1. Phasing

Each phase is independently shippable and leaves the system correct. Nothing charges until P3.

| Phase | Deliverable | Hard reqs | Self-contained? |
|---|---|---|---|
| **P1 — Unified meter** | Metadata route context (§4.0) + one meter per path writing a durable `usage_meter` ledger (capture only, **no wallet, no gate, no Lago**). `billing_owner_id`, `billing_request_id`, per-layer/per-flush `transaction_id`, request/byte counters, `CredentialClass` resale signal. `ServiceBilling` catalog field + write surfaces + anonymous-incompatibility validation. | 1,2,3,5,6,7,8,9,12 | Yes — `wallet_id`/`reserved_credits` are `Option`, unused in P1 |
| **P2 — Lago sink + display** | `LagoClient` + idempotent (GET-or-create) provisioning + event push + bidirectional reconcile + dead-letter. A Lago-compatibility spike (§11) precedes freezing the client signatures. Read-only `billing` block in API/UI. | 11 | Yes — builds on the P1 ledger |
| **P3 — The gate** | `billing_wallet` + entitlement (fail-closed, on route context) + reserve-then-true-up funding (local debit, idempotent settle) + cached rate card + overdraft cap + conditional fail-open. `BILLING_ENABLED` flips charging on. | 4 + D4 | Needs P2 |
| **P4 — Surfaces + backfill** | Billing UI (credits, invoices, per-service cost), CLI, top-up, one-time Lago backfill of existing owners. | — | Needs P3 |

## 2. Module map

```
backend/src/
  models/
    service_billing.rs        # ServiceBilling sub-struct (on DownstreamService) + BillingMetric
    usage_meter.rs            # UsageMeterRow + BillingLayer, UsageStatus, CredentialClass
    billing_wallet.rs         # BillingWallet (P3)
    billing_rate_cache.rs     # BillingRateCache (P2/P3)
  services/
    billing/
      mod.rs                  # BillingService facade
      route_context.rs        # metadata-only resolution → BillingRouteContext (§4.0)
      meter.rs                # MeteredProxyContext + open/settle/fail (§4.3)
      reservation.rs          # atomic reserve / idempotent settle against BillingWallet (§5.1) (P3)
      owner_resolver.rs       # BillingOwnerResolver (§6)
      lago_client.rs          # LagoClient — stateless I/O (P2)
      reconcile.rs            # bidirectional sweep + dead-letter (§7) (P2)
  handlers/billing.rs         # GET usage/wallet, POST top-up (P2/P4)
  errors/mod.rs               # 11300–11399 block (§8)
  db.rs                       # indexes (§3)
  config.rs                   # env (§9)
```

`AppState` gains `pub billing: Arc<BillingService>` (constructed before the `AppState` literal in
`main.rs`, mirroring `node_ws_manager`/`encryption_keys`).

## 3. Data model

### 3.0 Shared types

```rust
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BillingMetric { #[default] Tokens, Requests, Bytes }

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BillingLayer { Platform, Resale }

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UsageStatus { Reserved, Forwarded, Finalized, Failed, Abandoned, DeadLetter }

/// The FINAL resolved credential class (computed AFTER agent override + node/direct resolution).
/// Resale charges apply ONLY to `NyxidManagedMaster`. (R2 — replaces `has_server_credential`.)
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialClass {
    NyxidManagedMaster,        // catalog master credential NyxID owns/pays for → resale-eligible
    UserOwned,                 // user-stored BYO key (UserApiKey)        → platform only
    AgentOverrideUserOwned,    // per-agent binding swapped in a user key  → platform only
    NodeManaged,               // credential lives on the node            → platform only
    NoAuth,                    // no credential                           → platform only
}

pub struct PlatformUsage { pub requests: i64, pub bytes: i64 }
pub struct ResaleUsage   { pub metric: BillingMetric, pub quantity: i64 }
pub struct ResaleSpec    { pub metric: BillingMetric, pub lago_metric_code: String }
```

### 3.1 `ServiceBilling` (resale layer; sub-struct on `DownstreamService`)

Mirrors the `ServiceCapabilities` pattern (`models/downstream_service.rs:14-30, 266-268`). **There is
no `billable` field** — resale is `resale_billable`; the platform layer is plan-level (Lago plan), not
catalog config.

```rust
// models/service_billing.rs
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ServiceBilling {
    /// Resale (downstream value) charges — only honored when the FINAL CredentialClass is
    /// NyxidManagedMaster (§3.0). Default false.
    #[serde(default)]
    pub resale_billable: bool,
    #[serde(default)]
    pub resale_metric: BillingMetric,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lago_resale_metric_code: Option<String>,
}
```

On `DownstreamService`, sibling to `capabilities`:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub billing: Option<ServiceBilling>,
```

Platform metric codes are global constants: `platform_requests`, `platform_bytes`. `UserService`
inherits resale config through `catalog_service_id`.

**"Billing-active" rollup (R7).** A request is *billing-active* iff `ServiceBilling.resale_billable`
(and the resolved credential is `NyxidManagedMaster`) **OR** the resolved billing owner is on a
platform-metered Lago plan. The `BILLING_ENABLED` startup invariant (§9) refuses to serve a
billing-active request unless the meter+ledger (P1) and, for charging, the gate (P3) are wired —
fail-closed at catalog read.

**Write surfaces (R10) — not just the model field.** `handlers/services.rs` enumerates fields
manually, so P1 must also add `billing` to `CreateServiceRequest` (`services.rs:37`),
`UpdateServiceRequest` (`:233`), `ServiceResponse` (`:135`); set it in the field-by-field create
(`:977`), the manual `set_doc` update (`:1333`), and `service_to_response_with_viewer`
(`services_helpers.rs:240+`); and surface it in the catalog + CLI.

**Anonymous incompatibility (Hard req 9) is bidirectional (R10).** Reject `resale_billable=true` when
the service has enabled anonymous endpoints (in `services.rs`), AND reject enabling an anonymous
endpoint when `resale_billable` (in `admin_anonymous_endpoints.rs`) — extend
`validate_*_anonymous_compatibility` (`anonymous_endpoint_service.rs:104/113`). Returns 11304 (HTTP
400) at write time. Public/anonymous proxy (`public_proxy.rs`, no `AuthUser`) can therefore never be
billing-active.

### 3.2 `usage_meter` — durable ledger + reservation lifecycle

One row per `(billing_request_id, layer)` for request-shaped paths; per
`(billing_request_id, layer, flush_seq)` for connection-shaped paths (R4 — otherwise Lago dedups all
flushes after the first). Written **before** the downstream send (record-before-forward).

```rust
// models/usage_meter.rs  — COLLECTION_NAME = "usage_meter"
pub struct UsageMeterRow {
    #[serde(rename = "_id")] pub id: String,            // uuid
    pub transaction_id: String,                          // "{billing_request_id}:{layer}[:{flush_seq}]"
    pub billing_request_id: String,                      // stable; NOT the node wire request_id (R8)
    pub layer: BillingLayer,
    #[serde(skip_serializing_if = "Option::is_none")] pub flush_seq: Option<i64>, // connection-shaped only
    pub billing_owner_id: String,                        // wallet owner (NOT actor)
    #[serde(skip_serializing_if = "Option::is_none")] pub wallet_id: Option<String>,   // None in P1
    pub actor_user_id: String,                           // attribution only
    #[serde(skip_serializing_if = "Option::is_none")] pub api_key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub service_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub service_slug: Option<String>,
    pub metric: BillingMetric,
    pub lago_metric_code: String,
    pub credential_class: CredentialClass,               // resale only when NyxidManagedMaster
    #[serde(skip_serializing_if = "Option::is_none")] pub model: Option<String>,
    #[serde(default)] pub reserved_credits: i64,         // 0/unused in P1 (no wallet); set in P3
    #[serde(skip_serializing_if = "Option::is_none")] pub quantity: Option<i64>, // actual metered (settle)
    pub status: UsageStatus,
    pub forwarded: bool,                                  // true once the downstream send fired (R3.3)
    pub released: bool,                                   // hold released? (idempotent settle guard, R3.2)
    pub lago_acked: bool,
    pub attempt: i32,
    pub settlement_attempts: i32,
    pub settlement_next_retry_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")] pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")] pub updated_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")] pub finalized_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")] pub expires_at: Option<DateTime<Utc>>, // TTL, set by sweep on terminal+acked
    #[serde(skip_serializing_if = "Option::is_none")] pub last_error: Option<String>,
}
```

**Lifecycle.** `Reserved` (hold placed, before send) → set `forwarded=true` immediately before the
downstream send → `Forwarded` → on response, meter actual `quantity`, idempotent settle → `Finalized`
→ push to Lago → `lago_acked=true`. Error: downstream failure with `forwarded=false` → `Failed`
(release hold). **Crash recovery (R3.3):** the sweep may release/abandon `Reserved` rows
(`forwarded=false`, never sent — safe). It must **NOT** auto-free `Forwarded` rows (`forwarded=true`):
downstream cost may already be incurred, so those are charged at the reservation or held pending Lago
reconciliation / dead-lettered — never silently freed.

**Indexes** (`db.rs`, mirroring oracle patterns at `db.rs:1715-1743`):
- unique `{ transaction_id: 1 }` — idempotency / no double-row.
- `{ status: 1, lago_acked: 1, updated_at: 1 }` — reconcile/dead-letter sweeps.
- `{ billing_owner_id: 1, created_at: -1 }` — per-owner usage queries / display.
- TTL `{ expires_at: 1 }` expire-after 0 (the reconcile sweep sets `expires_at` on terminal+acked rows).

### 3.3 `billing_wallet` — cached balance + cap + Lago customer mapping (P3)

```rust
// models/billing_wallet.rs — COLLECTION_NAME = "billing_wallet"
pub struct BillingWallet {
    #[serde(rename = "_id")] pub id: String,             // uuid
    pub owner_id: String,                                 // NyxID person/org user_id (unique)
    pub lago_customer_id: String,                         // == external_customer_id (unique)
    #[serde(skip_serializing_if = "Option::is_none")] pub lago_subscription_id: Option<String>,
    pub plan_kind: PlanKind,                              // Prepaid | Subscription | Hybrid
    pub balance_credits: i64,                             // last value SYNCED from Lago
    pub reserved_credits: i64,                            // open holds (NyxID-owned)
    pub pending_lago_debits: i64,                         // finalized-but-not-yet-synced burns (R3.1)
    pub has_payment_instrument: bool,                     // gates conditional fail-open
    pub overdraft_cap_credits: i64,                       // money-denominated, via rate cache
    pub suspended: bool,
    pub collection_state: CollectionState,               // Good | PastDue | Suspended
    #[serde(with = "...chrono_datetime_as_bson_datetime")] pub balance_synced_at: DateTime<Utc>,
    #[serde(with = "...chrono_datetime_as_bson_datetime")] pub created_at: DateTime<Utc>,
    #[serde(with = "...chrono_datetime_as_bson_datetime")] pub updated_at: DateTime<Utc>,
}
```

**Availability (R3.1):** `available = balance_credits − reserved_credits − pending_lago_debits`.
`balance_credits` is authoritative-from-Lago (synced by sweep); `reserved_credits` are open holds;
`pending_lago_debits` is the sum of finalized-but-unsynced burns so availability drops *immediately* at
settle (not only at the next sync — without this, the next request reserves the same money). The
periodic sync sets `balance_credits` from Lago and zeroes the `pending_lago_debits` it has accounted
for. **Indexes:** unique `{ owner_id: 1 }`, unique `{ lago_customer_id: 1 }`.

### 3.4 `billing_rate_cache` — read-only Lago rate mirror (P2/P3)

```rust
// COLLECTION_NAME = "billing_rate_cache"
pub struct BillingRateCache {
    #[serde(rename = "_id")] pub id: String,             // "{lago_metric_code}:{model|*}"
    pub lago_metric_code: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub model: Option<String>,
    pub credits_per_unit_micros: i64,                    // integer, scaled; APPROXIMATE — never invoicing
    #[serde(with = "...chrono_datetime_as_bson_datetime")] pub synced_at: DateTime<Utc>,
}
```

Refreshed by a sweep from Lago plan/charge config. Used only for reservation sizing + cap
denomination. Authoritative pricing stays in Lago.

## 4. Metering — route context, per-path map, emit API (P1)

### 4.0 Metadata-only route context (R1)

Entitlement and owner resolution need the service/owner/credential-class **without** decrypting
credentials, and must run at a point that exists on every path. A metadata-only resolver yields:

```rust
// services/billing/route_context.rs
pub struct BillingRouteContext {
    pub billing_request_id: String,    // stable; minted once at the outer handler (R8)
    pub billing_owner_id: String,      // resolved billing owner (NOT actor) — see §6
    pub actor_user_id: String,
    pub api_key_id: Option<String>,
    pub user_service_id: Option<String>,
    pub catalog_service_id: Option<String>,
    pub service_slug: Option<String>,
    pub node_intent: NodeIntent,       // Direct | Node | NodeWithFallback
    pub auth_method: String,
    pub credential_class: CredentialClass,  // FINAL class, computed AFTER agent override (R2)
    pub resale: Option<ResaleSpec>,    // Some iff resale_billable && credential_class==NyxidManagedMaster
    pub platform_enabled: bool,        // owner's plan meters platform usage
}
```

Today resolution decrypts before returning direct targets (`proxy_service.rs:1750`); split a
metadata pass out of it (or compute the context from the already-resolved `pre`/`ProxyTarget` fields
**after** the agent-credential override at `proxy.rs:1289-1300`, where the final credential is known —
do **not** read the pre-override `has_server_credential`).

### 4.1 `MeteredProxyContext`

The emit-side handle, derived from `BillingRouteContext` + measured usage. `billing_request_id` is
minted ONCE in the outer wrappers (`proxy.rs:422/630` region) and threaded into `*_inner`,
`execute_proxy_inner`, and every spawned stream/bridge closure (`proxy.rs:2308` SSE, `1956` node
stream, `3326/3594` WS). It is **separate** from the node wire-protocol `request_id` (`proxy.rs:1816`),
which is deliberately regenerated per failover attempt (opposite lifecycle, R8).

### 4.2 Per-path resolve → gate → reserve → dispatch → settle (R1, R6)

There is **no single shared seam.** Each entry path owns its sequence; all call the same
`meter::{open, settle, fail}` (§4.3) and, in P3, the same `gate` (§5).

| Entry path | Resolve + gate/reserve site | Dispatch / send | Settle (meter) | Shape |
|---|---|---|---|---|
| `/proxy` (`execute_proxy_inner:1132`) | single fan-in ~`proxy.rs:1352` (after owner+credential-class+target resolved, **before** the WS/node/direct arms at `1662/1720/2236`) | `forward_request:2236` / node `send_proxy_request:1849` / WS bridges | buffered `2455`; SSE task `2308`; node Complete `1869`, Streaming `1893` | request (HTTP), connection (WS) |
| Codex/ChatGPT direct branch | same fan-in | `send_to_chatgpt` (`proxy.rs:2150/2187`) — bypasses `forward_request` | in the ChatGPT translator finalize | request |
| `/llm` gateway (`llm_gateway.rs:64/330`) | in `llm_proxy_request` ~`:265` (after resolution `:125`, before send) | `forward_request:273/644` AND Codex branch `:224/634` | its existing extraction `:827-933` → meter | request |
| MCP (`mcp_service::execute_tool`) | **inside** `execute_tool` after target resolution, before node/direct dispatch (`mcp_service.rs:2372+`) | execute_tool's own node/direct forwarding | post-dispatch within execute_tool | request (per tool call) |
| SSH exec (`ssh_exec.rs:91`) | before the exec call | `ssh_service` exec | on response (`metric=requests` or raw stdout/stderr bytes pre-truncation) | request |
| WS direct (`proxy.rs:3250`) / node (`3365`); SSH tunnel (`ssh_tunnel.rs:173`) / web terminal | at upgrade (reserve per-connection cap, R6) | bridge loop | periodic flush + settle on disconnect | connection-session |

**Metric capture.** Tokens reuse `llm_usage_service` (buffered + SSE accumulator). Requests = 1/call;
bytes = request/response length (HTTP) or `ConnectionUsageStats { frames_in/out, bytes_in/out,
duration }` (WS). **WS bridges today return only duration** (`proxy.rs:3036/3343/3617/3656`) → change
them to return/flush `ConnectionUsageStats`. **SSH tunnel + web terminal already keep byte counters**
(`ssh_tunnel.rs:375/529`, `ssh_web_terminal.rs:434/508`) — reuse them.

Drive metering on `ServiceBilling`/owner plan, never `slug.starts_with("llm-")` (Hard req. 6).

### 4.3 Emit API

```rust
// services/billing/meter.rs — writes are DURABLE (awaited), not fire-and-forget
pub async fn open(db, ctx: &BillingRouteContext) -> AppResult<()>;            // writes Reserved row(s); P3 also reserves §5.1
pub async fn mark_forwarded(db, billing_request_id) -> AppResult<()>;          // set forwarded=true, status=Forwarded, before send
pub async fn settle(db, ctx, platform: PlatformUsage, resale: Option<ResaleUsage>) -> AppResult<()>; // idempotent §5.1
pub async fn fail(db, billing_request_id, reason) -> AppResult<()>;            // only frees never-forwarded rows
```

`open()` and `mark_forwarded()` are awaited before the downstream send. `settle()` on a stream runs
inside the (already detached) stream task; durability for streams is **open-before-send + reconcile on
crash**, not a synchronously-durable settle (the `Forwarded`-not-auto-freed rule, R3.3, covers the
crash window). Keep a metadata-only audit event in parallel for the existing dashboards.

## 5. The gate (P3)

For a billing-active request, the gate runs on the `BillingRouteContext` at each path's resolve site
(§4.2). Two separate decisions:

```
gate(ctx):
  if not (ctx.platform_enabled or ctx.resale.is_some()): return Allow      # free request
  wallet = owner_resolver.resolve(actor, owner)                            # §6
  if wallet.suspended: return Deny(11307 WalletSuspended)
  # 1. ENTITLEMENT — always fail-closed (runs on metadata; no credential decryption needed)
  ent = entitlements.is_entitled(wallet, ctx.service)        # Lago entitlements API, cached + webhook-invalidated
  if ent == Unknown (cold cache / Lago down) or NotEntitled: return Deny(11303 PlanEntitlementRequired)
  # 2. FUNDING — reserve-then-true-up
  est = rate_cache.estimate(ctx)                            # pessimistic; per-model max for tokens
  if reservation.try_reserve(wallet, est): return Allow     # §5.1 atomic
  if wallet.plan_kind == Prepaid: return Deny(11300 InsufficientCredits)
  if wallet.has_payment_instrument and reservation.try_reserve_overdraft(wallet, est): return Allow
  return Deny(11300)
```

Lago **unreachable**: entitlement fails closed; prepaid funding fails closed; card-backed funding
fails open up to the cap (D4). `BillingProviderUnavailable` (11302) only when an operator forces
fail-closed.

**Connection-shaped gate (R6):** for WS/SSH sessions, reserve a per-connection byte-cap up front
(== the owner's overdraft-cap slice), flush + true-up periodically, and kill the stream when
cumulative flushed bytes exceed `available`.

### 5.1 Atomic reserve + idempotent settle (no DB transactions — R3.2)

The repo has **no** `start_session`/`with_transaction` (only a TODO at `services.rs:2127`), so
multi-doc atomicity is achieved with idempotent guards, mirroring `oracle_task_service.rs:959`.

**Reserve (prepaid):**
```rust
db.collection::<BillingWallet>("billing_wallet").find_one_and_update(
    doc!{ "owner_id": &owner, "suspended": false,
          "$expr": { "$gte": [ { "$subtract": [ { "$subtract": ["$balance_credits","$reserved_credits"] },
                                                 "$pending_lago_debits" ] }, est ] } },
    doc!{ "$inc": { "reserved_credits": est }, "$set": { "updated_at": now } },
).with_options(FindOneAndUpdateOptions::builder().return_document(ReturnDocument::After).build()).await?
// Some(_) == reserved. MongoDB serializes writes on the single wallet doc → no over-commit. (Wallet docs
// must default reserved_credits/pending_lago_debits to 0, else $subtract on a missing field → null → deny.)
```

**Overdraft reserve (card-backed):** same op with the cap-inclusive filter
`{$gte:[{$subtract:[{$add:["$balance_credits","$overdraft_cap_credits"]}, {$add:["$reserved_credits","$pending_lago_debits"]}]}, est]}`
AND `has_payment_instrument: true` — atomic so concurrent card-backed requests cannot exceed the cap.

**Settle (idempotent, two steps):**
```rust
// 1. atomically claim the row's terminal transition (idempotent: a re-run finds status already Finalized)
let claimed = usage_meter.find_one_and_update(
    doc!{ "_id": &row_id, "status": { "$in": ["reserved","forwarded"] } },
    doc!{ "$set": { "status":"finalized", "released": false, "quantity": actual,
                    "finalized_at": now, "updated_at": now } },
).with_options(FindOneAndUpdateOptions::builder().return_document(ReturnDocument::After).build()).await?;
// 2. only if WE made the transition, move the wallet (release hold + local debit so availability drops now)
if claimed.is_some() {
    billing_wallet.update_one(doc!{ "owner_id": &owner },
        doc!{ "$inc": { "reserved_credits": -est, "pending_lago_debits": actual } }).await?;
    usage_meter.update_one(doc!{ "_id": &row_id }, doc!{ "$set": { "released": true } }).await?;
}
```
A recovery sweep handles `status=Finalized && released=false` (crash between step 1 and step 2) by
re-applying the wallet move. The `pending_lago_debits` is cleared by the balance sync once Lago
confirms the burn (so the cap bounds total exposure across one full `BILLING_RECONCILE_INTERVAL_SECS`
window of inflated balance, not just one request).

## 6. `BillingOwnerResolver` (consumes `OwnerAccess`) (§6)

`org_service::resolve_owner_access` (`org_service.rs:1242`) is ACL-only — it returns
`OwnerAccess::{Direct, AsOrgAdmin, AsOrgMember, Forbidden}` and carries no wallet/policy. The resolver
maps that to a wallet:

```rust
pub struct ResolvedBillingOwner { pub owner_id: String, pub wallet: Option<BillingWallet>, pub pays: PaysFrom }
pub enum PaysFrom { Personal, OrgWallet { org_id: String } }  // MemberWallet DEFERRED (R9)
```

- `Direct` → personal wallet.
- `AsOrg*` → **org wallet** (P1/P3 default). Per-member wallets + per-member spend caps are DEFERRED
  (no org-billing-policy field exists yet — ADR open question).
- **Legacy `DownstreamService` path (R9):** `effective_owner_for_approval` is `None` there (set only in
  the `pre_resolved` arm, `proxy.rs:1187`). Fall back to the **actor's personal wallet**, and treat
  legacy-path requests as platform-metered only (never resale). Document this; the legacy path is still
  active during migration (CLAUDE.md §8).

`billing_owner_id` is the wallet owner chosen here, **distinct from the actor** (today usage logs the
actor at `proxy.rs:2269`). Cache the resolution short-TTL; **membership revocation must invalidate it**
— `revoke_membership` (`org_service.rs:975`) needs an invalidation hook (else a revoked member keeps
drawing the org wallet for the TTL). Required for P3.

## 7. `LagoClient` (stateless I/O) + provisioning + reconcile (P2)

```rust
// services/billing/lago_client.rs — base_url + api key + http client; NO state
pub trait LagoApi {
    async fn ensure_customer(&self, owner: &OwnerProvisionInput) -> AppResult<String>;  // GET-or-create (R11)
    async fn ensure_subscription(&self, customer_id, plan_code) -> AppResult<String>;     // GET-or-create
    async fn record_event(&self, ev: &LagoEvent) -> AppResult<LagoAck>;                    // POST /events
    async fn record_events_batch(&self, evs: &[LagoEvent]) -> AppResult<Vec<LagoAck>>;     // fall back to loop if unsupported
    async fn current_usage(&self, customer_id, sub_id) -> AppResult<LagoUsage>;
    async fn wallet_balance(&self, customer_id) -> AppResult<i64>;
    async fn entitlements(&self, sub_external_id) -> AppResult<Vec<Entitlement>>;          // endpoint path version-dependent (R11)
}
```

**Idempotent provisioning (R11):** Lago `POST /customers`/`/subscriptions` conflict on an existing
`external_id` (not upsert), so `ensure_*` is GET-or-create (treat conflict as "exists"). Verify
`record_events_batch`, `wallet_balance`, and the entitlements endpoint path against the **installed**
Lago via the §11 spike before freezing these signatures.

**Event schema (privacy allowlist):** send only
`{ transaction_id, external_customer_id, code, timestamp, properties: { quantity, model?, service_code? } }`.
**Never** `path`, raw `api_key_id`, bodies, or downstream URLs.

**Ingestion failure policy — branch on the Lago error `code`, not the HTTP status (R5):**
- `transaction_id_taken` (422) → **SUCCESS** (`lago_acked=true`; the event was already applied — do
  NOT dead-letter).
- `billable_metric_not_found` / subscription / closed-period (422) → `DeadLetter` + alert.
- `429` → backoff + retry. `5xx`/timeout → retry; row stays `lago_acked=false`.

**Reconcile sweep** (`reconcile.rs`, spawned like the OAuth sweep `main.rs:732`):
1. Re-push `lago_acked=false AND updated_at < now-grace` rows.
2. `Reserved && forwarded=false && updated_at < now-abandon_grace` → `Abandoned` (release holds).
   **`Forwarded` rows are charged/held/dead-lettered, never auto-freed** (R3.3).
3. `Finalized && released=false` past the settle grace, or a due retryable `Failed` row → re-apply
   the wallet move through the same idempotent settlement path. Settlement retries use bounded
   exponential backoff and end in `DeadLetter`.
4. **Bidirectional** per Lago **customer/subscription**: compare `sum(usage_meter finalized)` vs Lago
   `current_usage`; alert on drift.
5. Sync `balance_credits` from Lago, clear accounted `pending_lago_debits`, refresh `billing_rate_cache`.

## 8. Error codes (11300–11399 → HTTP 402, except 11304 → 400)

Add to `AppError` (`errors/mod.rs`), map status in `status_code()`, assign in `error_code()`, add keys
in `error_key()` (oracle 11000-block is the template).

| Code | Variant | Key | HTTP | Notes |
|---|---|---|---|---|
| 11300 | `InsufficientCredits` | `insufficient_credits` | 402 | prepaid below next reservation |
| 11301 | `BillingNotConfigured` | `billing_not_configured` | 402 | billing-active, no wallet/plan |
| 11302 | `BillingProviderUnavailable` | `billing_provider_unavailable` | 402 | explicit fail-closed override only |
| 11303 | `PlanEntitlementRequired` | `plan_entitlement_required` | 402 | plan excludes service (incl. fail-closed unknown) |
| 11304 | `AnonymousIncompatibleBilling` | `anonymous_incompatible_billing` | **400** | `resale_billable` + anon endpoint (write time) |
| 11307 | `WalletSuspended` | `wallet_suspended` | 402 | cap breached / collection suspended |

## 9. Config / env (`config.rs` + `docs/ENV.md`)

```bash
BILLING_ENABLED=false                 # refuse to SERVE a billing-active request unless meter+ledger(+gate for charging) are wired
LAGO_API_URL=                         # e.g. http://lago-api:3000
LAGO_API_KEY=                         # bearer for NyxID->Lago; network-isolate Lago
LAGO_PLAN_CODE=starter                # default owner subscription plan created during provisioning/backfill
LAGO_WEBHOOK_SECRET=                  # verify subscription.updated / wallet webhooks
BILLING_RECONCILE_INTERVAL_SECS=300   # reconcile + balance sync sweep (0 disables); bounds fail-open drift window
BILLING_RATE_CACHE_TTL_SECS=900
BILLING_RESERVATION_ABANDON_SECS=600  # grace before Reserved(forwarded=false) -> Abandoned
BILLING_DEFAULT_OVERDRAFT_CAP_CREDITS=0
BILLING_FAIL_CLOSED=false             # operator override: force fail-closed everywhere (-> 11302)
BILLING_RESALE_ENABLED=false          # explicit opt-in; platform-only by default
# Stripe lives UNDER Lago (Lago's payment-provider config), not in NyxID env.
```

Parse with the existing `env::var().ok().and_then(parse).unwrap_or(default)` / `parse_bool_env`
(`config.rs:530`) pattern. The `BILLING_ENABLED` invariant asserts at catalog read: a resolved
billing-active request (`resale_billable` + `NyxidManagedMaster`, or platform-metered owner) is refused
unless the required modules are wired — fail-closed, not honor-system (Hard req 12).

## 10. Surfaces

- **Admin (P1):** `ServiceBilling` in the service edit "Service Metadata" section; full write-path
  plumbing per §3.1 (R10); `nyxid catalog show <slug>` displays it.
- **User API (P2/P4):** `GET /api/v1/billing/wallet`; `GET /api/v1/usage?period=` (per-service usage +
  cost from Lago `current_usage`, conditional on `BILLING_ENABLED` + live Lago + configured price, D5);
  `POST /api/v1/billing/topup` (P4, → Stripe via Lago).
- **Frontend (P4):** a **new** billing surface (credits, invoices, per-service cost). The existing
  `api-key-usage-dashboard.tsx` is an "Agent Activity" request-count view, not a billing UI — treat
  billing UI as new work.
- **CLI:** `nyxid billing wallet`, `nyxid billing usage`.

## 11. Migration / backfill + Lago spike

- **P2 Lago-compatibility spike (R11):** before freezing `LagoClient`, verify against the installed
  Lago's OpenAPI: customer/subscription create conflict semantics, `events` batch support, wallet
  balance read, entitlements endpoint path, and the 422 error `code` sub-values.
- Idempotent `ensure_customer`/`ensure_subscription` keyed by NyxID owner id (`external_customer_id`).
- One-time, re-runnable backfill provisions a Lago customer + wallet per existing person/org owner,
  behind `BILLING_ENABLED`.
- Provisioning race (Lago reachable, customer not provisioned yet): gate **fails closed** except
  card-backed within cap.

## 12. Test plan (TDD; real Mongo for integration)

**Unit:** `ServiceBilling`/enum serde round-trip; `available = balance − reserved − pending` math;
rate-cache reservation sizing; entitlement decision table (incl. Unknown → fail-closed);
`CredentialClass` derivation (BYO/override/master/no-auth/node).

**Integration (money-correctness):**
- **Concurrency drain:** N parallel reserves against balance B → total ≤ B; prepaid never over-commits.
- **Post-settle double-spend:** finalize a request, immediately fire another → second sees reduced
  `available` (pending_lago_debits applied), cannot reserve the freed amount before sync.
- **Crash-after-forward:** kill between `mark_forwarded` and settle → sweep leaves the `Forwarded` row
  charged/held (NOT auto-freed); kill between `Reserved`(not forwarded) and send → hold released.
- **Idempotent settle:** re-run settle → no double wallet move (`released` guard); crash between step 1
  and step 2 → recovery sweep completes it.
- **Idempotent replay + dedup:** re-push same `transaction_id` → Lago dedups; `transaction_id_taken`
  422 → `lago_acked=true`, not dead-letter; per-layer + per-flush ids never collide.
- **Connection-flush:** a WS session with K flushes emits K distinct `transaction_id`s → all K billed.
- **Owner attribution:** org-member request bills the org wallet (`billing_owner_id`), not the actor;
  legacy-path request bills the actor's personal wallet, platform-only.
- **Resale classification:** BYO key (and agent-override BYO) on a resale-billable catalog service →
  `CredentialClass != NyxidManagedMaster` → platform-only, no resale charge.
- **Path coverage:** a billing-active service over each path (`/proxy` direct/node/WS, `/llm`,
  Codex/ChatGPT, MCP, SSH) is metered AND gated; anonymous/public path with `resale_billable` rejected
  at write (11304).
- **Fail modes:** Lago down → entitlement denies, prepaid denies, card-backed allowed within cap; cap
  breach → `WalletSuspended`.
- **Failover stability:** a node failover (new wire `request_id` per attempt) keeps ONE stable
  `billing_request_id` → exactly one platform line.

**E2E:** top-up → proxy burns credits → balance drops → exhaustion 402 → top-up restores.

## 13. Acceptance gate (maps to ADR Hard Requirements)

`BILLING_ENABLED` charging stays off until P1+P2+P3 land and §12 passes. Per requirement:

1. Unified meter + gate across ALL paths (`/proxy`, `/llm`, Codex, MCP, SSH, WS) → path-coverage test.
2. Stable per-request/per-layer (and per-flush) `transaction_id`, stable across node failover →
   idempotent-replay + connection-flush + failover-stability tests.
3. Durable cross-instance state → crash-after-forward + concurrency + idempotent-settle tests.
4. Money-denominated overdraft cap + auto-suspend (cached rate card) → fail-modes test.
5. Bill the billing owner, not the actor → owner-attribution test.
6/8. Right number / final credential class → resale-classification test (BYO + override → platform).
7. Integer counts to Lago (the `LagoEvent.properties.quantity` is i64, sourced only from integer
   counts; `reported_cost` is never an input) → schema test.
9. Anonymous incompatible (bidirectional, 11304) → write-time test on both `services.rs` and
   `admin_anonymous_endpoints.rs`.
10. `BillingOwnerResolver` separate from ACL, with legacy + org defaults → unit + owner-attribution.
11. Bidirectional reconcile + dead-letter + 422 sub-code branch → reconcile/dead-letter tests.
12. Refuse to serve billing-active until wired → `BILLING_ENABLED` startup-invariant test.

## 14. Deferred / open (tracked in ADR §Open questions)

Overdraft cap value & suspension duration; org-vs-member payment precedence + per-member caps +
`MemberWallet`; SSH/MCP metric definitions (no SSH/MCP service can be billing-active until defined);
reservation pessimism tuning; membership-revocation cache-invalidation hook (required for P3);
in-flight mid-stream kill-switch tuning; native Phase-0 display (dropped).
