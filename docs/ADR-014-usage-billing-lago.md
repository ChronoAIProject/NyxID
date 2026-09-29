# ADR-014: Usage Metering and Billing via Lago

> **Status:** Proposed (draft) — **Rev 2.** Revised after (1) an adversarial Claude pressure-test and (2) independent second/third opinions from Codex GPT-5.5 and OpenCode GLM-5.2 (all code-grounded). Rev-2 deltas are marked inline.
> **Date:** 2026-06-22
> **Canonical home:** this is a NyxID-repo working draft. The authoritative record belongs in
> `ChronoAIProject/chrono-ai-ceo/decisions/NNN-usage-billing-lago.md`. ADR-013 is the latest
> known number, so this is tentatively **ADR-014** — confirm the next free number before landing.
> **Related:** ADR-013 (pure passthrough), CLAUDE.md §8 (streamlined services), CLAUDE.md §9 (agent isolation),
> `backend/src/services/llm_usage_service.rs`, `docs/AI_SERVICES_ARCHITECTURE.md`.

> Accounting/storage update: NyxID 0.31 uses [exact Credits and balanced v2 postings](BILLING_EXACT_ACCOUNTING.md). That normative document supersedes this draft’s integer wallet/rounding assumptions and defines the drained-writer cutover.

## Context

NyxID proxies user/agent traffic to downstream services (OpenAI, Anthropic, custom APIs, SSH, MCP).
We want to **meter usage and charge users** for the services NyxID brokers, and **show users the
cost** of a given service inside NyxID.

What exists today:

- **Usage capture already exists for LLMs.** `backend/src/services/llm_usage_service.rs` extracts
  `prompt_tokens / completion_tokens / total_tokens` (and an optional `reported_cost`) from both
  buffered and SSE-streaming responses, and fires a metadata-only `llm_usage_reported` audit event
  carrying `user_id`, `service_id`, `provider_slug`, `model`, and `api_key_id`. This runs at the
  proxy completion hook in `handlers/proxy.rs`.
- **A catalog/instance split already exists.** Admin-seeded services are `DownstreamService` (the
  read-only catalog). A user's usable instance is a `UserService` ("AI service" on `/keys`), which
  links back to its catalog parent via `catalog_service_id`.
- **Per-agent attribution already exists.** Proxy requests via API key carry `api_key_id` /
  `api_key_name` / `platform` (CLAUDE.md §9).
- **No pricing, wallets, invoicing, or payment rails exist.**

Constraints (project fixed points):

- FI-002 — host facts via config, not hardcoded: *which* services bill and *how* must be admin
  data, not code.
- FI-003 — keep the stable core small: pricing/invoicing logic must not sink into the proxy core.
- FI-004 — cross-process facts need an authoritative record: wallet balance must have a real SSOT;
  an in-process cache must not impersonate it.
- FI-005 — boundaries over convenience: "is this billable" (catalog), "record usage" (proxy),
  "price + invoice + collect" (billing system) are distinct responsibilities.

## Decision

**D1 — Two charge layers: resale (catalog-level) and platform/proxy (plan-level).**
NyxID can charge for two distinct things, and they live in different places:
- *Resale charges* — the value of a downstream NyxID brokers (e.g. tokens on a **NyxID-provided**
  key). Catalog-level: configured per `DownstreamService` via the `ServiceBilling` sub-struct
  (sibling to `ServiceCapabilities`), inherited by the `UserService` via `catalog_service_id`. Only
  services where NyxID supplies the credential carry resale charges.
- *Platform / proxy charges* — the act of proxying itself (requests, bandwidth). These are
  **plan-level**, not per catalog service, and can apply to **any** `UserService` including a user's
  own **custom endpoints**. We never charge for a custom downstream's value, but using NyxID to proxy
  to it is chargeable when the owner's plan says so.

A custom (non-catalog) service therefore carries no resale charge by construction, yet may still
incur platform charges per the owner's plan. The two layers are independent line items.

**D2 — Lago is the metering/pricing/invoicing engine; Stripe collects.**
NyxID pushes usage events to Lago (self-hosted, AGPLv3). Lago holds rate cards/plans, computes cost,
manages wallets, and generates invoices. Lago does **not** move money — it delegates collection to
Stripe. We do **not** hand-roll pricing/invoicing math, and we do **not** charge through Stripe
directly (Stripe's native metering is too thin for our per-model/per-service cardinality and has no
first-class prepaid wallet).

**D3 — Metering is multi-metric, and requires a single new metering choke point (NOT "reuse the existing hook").**
`ServiceBilling.metric ∈ { tokens, requests, bytes }`. Pressure-test correction: only token
extraction exists today, and only on the **direct-HTTP path** for `llm-*` slugs
(`llm_usage_service.rs`, `proxy.rs:2262-2453`). Request counting and byte measurement are **net-new
capture**, and the **node-routed and WS paths have no usage hook at all** (`proxy.rs:1860-2041`,
`3250`, `3365`) — a billable service on those paths would silently bill nothing. Therefore v1 builds
a **single metering choke point** that every proxy path (direct HTTP, node HTTP, direct WS, node WS)
passes through, emitting per-metric usage keyed by the billing owner. No service may be `billable`
until its route flows through that choke point (see Hard requirements). Each metric maps to its own
Lago billable-metric code; one primary metric per service in v1. Typically `tokens` is a *resale*
metric and `requests` / `bytes` suit *platform* charges, but either layer may use any metric.

**D4 — Billing model is plan-driven; the gate splits entitlement (always fail-closed) from funding (conditional fail-open), over cross-instance state.**
The charging model is a Lago plan/wallet configuration, not a NyxID constant: an owner may be on a
**subscription** (recurring fee + optional allowance/overage), **prepaid credit** (wallet burn-down),
or a **hybrid**. NyxID does not encode which. Before forwarding a billable request the gate makes two
*separate* decisions (pressure-test correction — these had different failure semantics conflated):

- **Entitlement** (does the owner's plan include this service?) — **always fails closed**. A cold
  cache or a Lago-unknown state denies a paid-tier service; a free user reaching a paid tier is pure
  leakage with no settlement path.
- **Funding** (does the owner have balance/allowance?) — evaluated against **cross-instance** state
  (MongoDB last-known balance, *not* per-process memory — the platform is stateless multi-instance,
  CLAUDE.md §11), with an atomic decrement on the shared store.

Resolved usage is written to a durable `usage_meter` ledger keyed by a stable per-request, per-layer
`transaction_id`, then pushed to Lago (which dedups on that id) so Lago applies the allowance/overage
or burns the wallet down. Lago is the pricing/invoice SSOT; the `usage_meter` ledger is a genuine
NyxID-owned co-record of un-acked usage until Lago confirms — the honest FI-004 position (see Hard
requirements), not "Lago is the only record."

**Funding uses reserve-then-true-up, not a bare balance check** (Rev 2 — both second-opinion reviewers
showed a bare pre-flight balance check is unsafe under concurrency: N concurrent requests all observe
balance > 0, all forward, all meter *after* the response). At the gate NyxID **reserves a pessimistic
cost** for the request (estimated from the cached read-only Lago rate card, D5; e.g. a per-model
max-token cost), atomically decrements the shared balance by the reservation, forwards, then
**settles/true-ups** against actual metered usage post-response. The reservation row is written
**before** the downstream send (record-before-forward), so a crash after forwarding cannot lose usage
NyxID already paid the provider for.

**Funding fail-open is conditional, not blanket** (revises the earlier blanket fail-open). When Lago
is unreachable, NyxID fails open **only for accounts with a payment instrument on file**
(postpaid/subscription with a card) and **only up to a hard, money-denominated overdraft cap** (priced
via the cached rate card) that auto-suspends the wallet when breached. **Prepaid wallets at or near
zero fail closed** (402) — and "near zero" now means *below the next reservation*, which the
reserve-then-true-up model makes well-defined. The **cap**, enforced atomically on the shared store,
is the real bound on exposure, not "the next request".

**Wallets exist at both org and per-member granularity.** A billable request resolves its wallet via
`org_service::resolve_owner_access(actor, owner)`: org-owned usage draws the org wallet; a member's
usage draws their own wallet or the org's, per an org-level setting. Each wallet maps 1:1 to a Lago
customer.

**D5 — Price/cost is shown in NyxID, conditionally.**
The per-service API response includes a `billing` block **only when** the service is `billable`
*and* NyxID has a live Lago connection *and* a price is configured. Otherwise it is omitted and the
UI falls back to raw usage counts (or nothing). Pricing values originate in Lago and are read back
via Lago's `current_usage`. NyxID never stores its own *authoritative* rate card — but, **per Rev 2,
it may cache Lago's rate card read-only** (clearly labeled approximate, never used for invoicing) for
one purpose: estimating a reservation amount and bounding the overdraft cap in money/credit terms (see
D4). Lago's `current_usage` is treated as provisional billing/display data, **not** a per-request
authorization primitive.

**D6 — A thin stateless Lago *client* + a NyxID-owned billing *service*.**
Pressure-test correction: the original "thin `BillingProvider` trait mirroring `KeyProvider`" framing
was withdrawn — `KeyProvider` is *pure/stateless*, whereas billing's hard parts (durable ledger,
cross-instance balance, reconcile sweep, overdraft cap, gate decision) are stateful and cannot hide
behind a swappable adapter without either leaking into NyxID core or being unsafe. So billing splits
in two:

- A small **stateless `LagoClient`** (I/O only: provision customer/subscription, record event, read
  current_usage/wallet). This is the swappable seam.
- A **NyxID-owned, MongoDB-backed `BillingService`** that owns the `usage_meter` ledger, cross-instance
  balance state, the reconcile sweep, the overdraft cap, and the gate decision.

NyxID implements *no* rating, proration, invoicing, tax, or dunning — those stay in Lago. But the
gate, the ledger, and entitlement *evaluation* live in NyxID by necessity (D7). Boundary (FI-003/005):
NyxID owns "meter + gate + ledger + display", Lago owns "price + wallet + invoice", Stripe owns
"collect".

**D7 — Subscription, prepaid credit, and plan-based enable/disable are supported; charging *config* lives in Lago, but entitlement *evaluation* lives in NyxID.**
Enabling/disabling a service per plan is an **entitlement** decision the gate returns
(`PlanEntitlementRequired`, 402, fail-closed per D4). **Rev 2 correction (verified):** Lago **does**
have a first-class entitlements API (`GET /api/v1/subscriptions/{external_id}/entitlements`, plus a
`subscription.updated` webhook) — the first-draft claim that it has none was wrong and is withdrawn.
So NyxID reads entitlements from Lago as the **source of truth** (cached, webhook-invalidated) and
still **enforces the gate locally** (fail-closed). Charging *configuration* (plan, prices, credit vs
subscription) is Lago data and changes there without NyxID code; the entitlement *gate enforcement* is
NyxID code. The "no NyxID code change for new plan shapes" claim remains partly overstated — new plans
need no code, but moving a service in/out of a tier still touches the gate's entitlement mapping.

## Hard requirements (gating — from the pressure-test)

An adversarial review (billing-correctness, security, architecture; all code-grounded) found the
first draft leaned on the existing best-effort, fire-and-forget *audit* pipeline as if it were a
billing-grade meter. It is not. **No service may be marked `billable` until all of the following hold:**

1. **Unified metering choke point — enumerate ALL entry points (Rev 2).** Beyond the four originally
   listed (direct HTTP, node HTTP, direct WS, node WS), the second-opinion review found more: the
   **`/llm` gateway** (`handlers/llm_gateway.rs`) is a *complete parallel* metering path the first
   draft omitted; **MCP transport** (`mcp_transport.rs`) and **SSH** (`ssh_tunnel.rs`,
   `ssh_web_terminal.rs`) are **connection-shaped, not request-shaped** (per-connection metering with
   periodic flush + settle on disconnect — distinct from per-request HTTP). The natural HTTP seam is
   `proxy_service::forward_request` (shared by direct + `/llm` + public); WS/MCP/SSH need separate
   instrumentation — realistically 2-3 sites, not literally one. (v1 scope decision: **build this
   meter first**, before any billing ships.)
2. **Stable per-request, per-layer `transaction_id`.** Minted once at proxy entry (before failover),
   threaded into the usage context, distinct per layer (`{request_id}:platform` / `:resale`). Today
   no such id exists — audit ids are random per write and the node `request_id` is per-attempt and
   never reaches the hook. Without it, Lago cannot dedup and any retry/SSE-reconnect/replay
   double-bills.
3. **Durable, cross-instance state.** The `usage_meter` ledger and last-known balance live in MongoDB
   (the Oracle-queue precedent), not per-process memory — the platform is stateless multi-instance
   (CLAUDE.md §11). Funding decrements atomically on the shared store.
4. **Hard overdraft cap + auto-suspend**, enforced NyxID-side on the shared store — the real bound on
   fail-open exposure.
5. **Bill the billing owner, not the actor.** The usage context carries a `billing_owner_id`
   (resolved org/member wallet owner) distinct from the actor. Today usage is logged against the
   actor (`proxy.rs:2269`), which mis-bills org spend.
6. **Trust the right number for charges (Rev 2 refinement).** Drive metering off the config-driven
   `ServiceBilling.metric`, never the `llm-` slug prefix or downstream `reported_cost`. Token counts
   are **trusted only for resale on NyxID-provided keys to trusted providers** (OpenAI/Anthropic etc.
   — the provider is not the attacker); they are **not** trusted for user-controlled/custom downstreams,
   which are platform-metered (requests/bytes NyxID measures itself), never resale. A billable request
   with no parseable usage is a flagged exception, not a silent $0.
7. **Integer metric counts to Lago, never computed money** for invoicing (Lago owns authoritative
   pricing; no `f64` currency). The cached rate card (D5) is for reservation/cap *estimation* only.
8. **Resale keys on the FINAL resolved credential, not the catalog flag (Rev 2).** Per-agent bindings
   (`AgentServiceBinding`, `proxy.rs:1287`) and master-credential injection (`proxy_service.rs:1636`)
   change the credential at proxy time; resale charges only when a live `is_nyxid_managed` signal is
   true, regardless of catalog metadata.
9. **`billable=true` is incompatible with anonymous/public-proxy endpoints (Rev 2).** `public_proxy.rs`
   has no `AuthUser`/wallet, so the gate is undefined there. Enforce at `ServiceBilling` write time
   (mirrors the CLAUDE.md §5 anonymous-incompatibility rule), returning a 400.
10. **A real `BillingOwnerResolver`, separate from `resolve_owner_access` (Rev 2).** The ACL function
    decides direct/admin/member/forbidden; it carries no wallet, payment policy, per-member budget,
    suspension, or collection state. Billing needs its own resolver consuming the ACL result.
11. **Bidirectional Lago reconciliation + dead-letter (Rev 2).** Beyond re-pushing un-acked rows,
    periodically compare `sum(usage_meter)` to Lago `current_usage` per customer; dead-letter + alert
    rows Lago persistently rejects (terminated subscription / closed period / recreated customer);
    apply a 422-deadletter / 429-backoff / 5xx-retry ingestion policy. "No loss" requires this, not
    just a durable ledger.
12. **Refuse to serve `billable=true` until the meter + ledger are wired (Rev 2)** — a startup/config
    invariant (fail-closed at catalog read), not an honor-system flag.

## Design sketch

> This ADR records decisions and rationale. A follow-up implementation spec will carry full field
> definitions, indexes, and test plans.

### Model

```text
// on DownstreamService (catalog), sibling to ServiceCapabilities
ServiceBilling {
  billable: bool,                          // default false
  metric: "tokens" | "requests" | "bytes",
  lago_billable_metric_code: Option<String>,
  lago_plan_code: Option<String>,
}
```

`UserService` resolves billing through `catalog_service_id`; no duplicated billing fields on the
instance in v1.

### Wallets and owner resolution

Wallets exist at **org** and **per-member** granularity. A billable request resolves its wallet via
`org_service::resolve_owner_access(actor, owner)`: org-owned usage draws the org wallet; a member's
usage draws their own wallet or the org's, per an org-level setting. Each wallet maps 1:1 to a Lago
customer (`external_customer_id`), so granularity is a NyxID resolution concern, not new Lago
machinery.

### Proxy flow (billable service)

```mermaid
sequenceDiagram
    participant Client
    participant Proxy as NyxID Proxy
    participant Cache as Balance + usage_meter (MongoDB, shared)
    participant DS as Downstream
    participant Meter as unified meter hook (all paths)
    participant Lago

    Client->>Proxy: request via UserService (billable)
    Proxy->>Cache: resolve billing owner + wallet; check entitlement + funding (atomic)
    alt not entitled / prepaid at zero / cold cache
        Cache-->>Client: 402 (entitlement fails closed; prepaid funding fails closed)
    else entitled AND (funded OR card-backed within overdraft cap)
        Proxy->>DS: forward
        DS-->>Proxy: response
        Proxy->>Meter: tokens | request count | bytes (per-layer transaction_id)
        Meter->>Cache: write usage_meter row (durable, lago_acked=false)
        Proxy-->>Client: response (not blocked on Lago latency)
        Meter--)Lago: POST /events (dedup by transaction_id)
        Lago--)Cache: ack + burn-down update shared balance
    end
    Note over Cache,Lago: sweep re-pushes un-acked rows; Lago=pricing SSOT, ledger=usage co-record
```

### Reliability stance

- Usage is written to a durable MongoDB **`usage_meter` ledger first** (durably enough to survive an
  instance crash), keyed by the stable per-request, per-layer `transaction_id` with an `lago_acked`
  flag — then pushed to Lago (which dedups on that id). The ledger, not the audit row, is the replay
  source; the audit row id (random per write) must **not** serve as `transaction_id` (Hard req. 2).
- A reconcile sweep selects `lago_acked = false AND created_at < now - grace` and re-pushes (Lago
  dedups by id → no double-apply). **But a durable ledger alone does not guarantee Lago *applies* a
  replay** (terminated subscription / closed period / recreated customer can reject it), so the sweep
  also **reconciles bidirectionally** (`sum(usage_meter)` vs Lago `current_usage` per customer) and
  **dead-letters + alerts** persistently-rejected rows rather than retrying forever (Hard req. 11).
- If Lago is **unreachable**: balance reads serve the last-known *shared-store* value; entitlement
  fails closed and prepaid-at-zero fails closed; card-backed funding fails open up to the overdraft
  cap (D4). Queued usage replays when Lago returns.

### Error codes

Reserve a new block **11300–11399 (billing/payments)**, mapping to **HTTP 402 Payment Required**:

- `11300 InsufficientCredits`
- `11301 BillingNotConfigured`
- `11302 BillingProviderUnavailable`
- `11303 PlanEntitlementRequired` — owner's plan does not include this service (a 402 upgrade-gate, distinct from out-of-credits)

(Follows the existing reserved-block convention in CLAUDE.md §6.) Per D4 fail-open,
`InsufficientCredits` fires on the request *after* a wallet is known-negative — the request in flight
during a Lago outage is allowed by design. `BillingProviderUnavailable` is reserved for an explicit
fail-closed override, not the default path.

### Surfaces

- **Admin:** `ServiceBilling` editable in the service edit page "Service Metadata" section;
  accepted by `POST/PUT /services`; shown by `nyxid catalog show <slug>`.
- **User:** conditional `billing` block on `/keys` responses. Note (Rev 2): the existing
  `api-key-usage-dashboard.tsx` is an *"Agent Activity"* request-count view backed by audit logs, not
  a billing surface — the billing UI (credits, invoices, per-service cost, Lago state) is **new work**,
  not a render tweak.
- **Config (new env):** Lago base URL + API key, Stripe keys (under Lago), reconcile interval,
  fail-open/closed toggle. Documented in `docs/ENV.md`.

## Consequences

**Positive**

- Direct-path LLM token extraction already exists and is reused — but per D3 / Hard req. 1, request &
  byte metering, the node/WS/`/llm`/MCP/SSH paths, owner attribution, the durable ledger, and the
  reservation gate are **net-new**. This is a build, not "wiring" (Rev 2: corrected the earlier
  "largely wiring" framing that contradicted D3).
- Catalog-driven billing keeps host facts in config (FI-002) and the proxy core neutral (FI-003/005).
- Prepaid wallets fit the AI-product credit model and bound collection risk.
- Lago abstracts the payment processor; Stripe can be swapped/augmented later.

**Negative / costs**

- A new operational dependency (Lago service + its DB) to run, monitor, and upgrade.
- Two billing-side systems (Lago + Stripe), more moving parts than Stripe-alone.
- A balance cache + reconciliation loop is real complexity and a potential source of drift/abuse.
- AGPLv3: run Lago **unmodified** behind its API. Do not fork and expose modified Lago as a network
  service, or copyleft obligations attach. Our integration code stays in NyxID.

**Risks**

- **Mid-stream exhaustion (happy-path, not just outage):** the gate is pre-flight only, but SSE/large
  responses finalize usage after the stream ends (`proxy.rs:2329-2378`), so a near-zero wallet can
  stream one expensive response. Bound it with the per-request overdraft cap (Hard requirement 4);
  in-flight metering with a kill-switch is a possible later hardening.
- **Fail-open exposure** is bounded by the hard overdraft cap, not by "the next request"; prepaid
  fails closed (D4), so residual exposure is card-backed accounts up to the cap.
- The hot path gains owner/wallet resolution + a funding check; `resolve_owner_access` already costs
  1-3 DB round trips, so the gate must cache the resolved wallet + entitlement (short TTL, invalidated
  on membership revocation) and be latency-measured — it is **not** free.

## Alternatives considered

1. **Stripe Billing alone (no Lago).** Rejected as the primary engine: native metering is thin for
   our per-model × per-service cardinality, no first-class prepaid wallet/burn-down, and a billing
   fee on managed revenue. Still used *underneath* Lago for collection.
2. **Build metering + pricing + invoicing in NyxID.** Rejected: re-implements money-critical,
   edge-heavy logic (rate cards, proration, invoices, dunning, tax) and still needs Stripe. Violates
   FI-003/FI-005. Exception: read-only usage *display* (no charging) could be built natively off the
   existing audit events — kept as a possible Phase 0.
3. **Postpaid invoicing instead of prepaid.** Rejected for v1 per product decision (carries
   collection/default risk); prepaid credits chosen.
4. **Synchronous Lago balance check on every request.** Rejected: puts Lago latency/availability on
   the proxy hot path, contradicting NyxID's neutral-relay uptime posture. Cached-balance gate chosen.

## Resolved since first draft

- **Charge trigger:** per-catalog-service admin choice (D1) — no separate global trigger.
- **Lago outage behavior:** *conditional* fail-open (D4) — entitlement always fails closed; funding
  fails open only for card-backed accounts up to a hard overdraft cap; prepaid-at-zero fails closed.
  (Revises the first draft's blanket fail-open after the security review.)
- **Wallet granularity:** both org and per-member wallets (D4), each mapping to one Lago customer.
- **v1 scope:** build the unified metering choke point across all proxy paths *before* any service is
  billable (Hard requirements).
- **Fail-open safety mechanism (Rev 2):** a cached read-only Lago rate card (D5) bounds the cap in
  money terms + reserve-then-true-up funding (D4). A bare pre-flight balance check was rejected as
  unsafe under concurrency.
- **Token metric in v1 (Rev 2):** kept for resale on NyxID-provided trusted-provider keys (the
  provider is authoritative); user-controlled downstreams are platform-metered only.
- **Lago entitlements (Rev 2):** use Lago's entitlements API as source of truth + a local gate; the
  earlier "Lago has no entitlement API" claim was wrong and is withdrawn.

## Open questions

- **Overdraft cap value + suspension policy.** The cap is now a hard requirement; its numeric value,
  per-plan overrides, and how long a suspended wallet stays cut remain to set.
- **Org-vs-member precedence + per-member caps.** When a member acts in an org context, does the org
  or member wallet pay by default, who configures it, and is there a per-member spend cap within an
  org wallet?
- **Metric coverage for SSH / MCP** services, where tokens/requests/bytes map less cleanly (WS bytes
  are covered by the unified meter; SSH/MCP need a metric definition).
- **Lago provisioning & backfill.** Idempotent customer/subscription creation keyed by NyxID owner id
  (`external_customer_id`), plus one-time backfill of existing users — a sync surface to spec.
- **Partial-usage policy.** On client disconnect / truncated stream, is the user charged for
  snapshot-reported-but-undelivered tokens? (Today the code would bill them.)
- **Privacy of the Lago event schema.** Send only minimal billing dimensions (owner id, metric code,
  quantity, txn id, coarse model/service code); exclude `path` and raw `api_key_id` to preserve the
  metadata-only discipline.
- **Platform-charge configuration.** Owner's Lago plan (preferred) vs a global NyxID setting, and how
  it's metered uniformly across custom and catalog services.
- **Dev `LagoClient` stub** for local development without a running Lago instance.
- **Membership-revocation cache invalidation (Rev 2).** `revoke_membership` only updates the membership
  row; the gate's wallet/entitlement cache needs an explicit invalidation (or an accepted bounded
  $-loss TTL window) so a revoked member stops drawing the org wallet promptly.
- **Reservation sizing (Rev 2).** How pessimistic is the per-request reservation (per-model max-token
  cost?), tuned to bound concurrency risk without over-holding prepaid balances?
- **Provisioning-race fail direction (Rev 2).** When Lago is reachable but the customer/subscription
  isn't provisioned yet (backfill / new user), does the gate fail open (card-backed) or closed?
  (Proposed: closed.)
