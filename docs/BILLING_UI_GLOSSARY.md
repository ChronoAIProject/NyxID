# Billing UI Glossary

What every term on `/billing` actually means, where the number comes from, and where the label
lies to you.

**Scope:** the logged-in Billing page (https://nyx.chrono-ai.fun/billing). Source of truth is the
code in this worktree — `frontend/src/pages/billing.tsx` + `backend/src/handlers/billing.rs`.
The usage descriptions include personal platform-key billing, free meters, and exact funding costs.

**Design background:** [ADR-014](./ADR-014-usage-billing-lago.md) (decisions) and
[USAGE_BILLING_LAGO_SPEC.md](./USAGE_BILLING_LAGO_SPEC.md) (implementation). Lago is the billing
engine; NyxID owns the meter, wallet cache, spend gate and per-service lane prices. Lago synchronizes those prices and owns invoicing.
Where the ADR's intent and the shipped code disagree, this doc describes **the code**.

> Written by two independent passes (Claude + GPT/Codex) over the same source, then reconciled;
> every claim below was verified against the code.

---

## 0. The three words everything else is built on

| Term | Meaning |
|---|---|
| **Credit** | The billing unit. **1 credit = 1 USD.** NyxID creates every Lago wallet in USD with `rate_amount: "1"`, so credits are 1:1 with the wallet currency (`services/billing/lago_client.rs:93-95`, `:272-278`). Wallet amounts are always whole integers. |
| **Credit micros** | One millionth of a credit — fixed-point, no floating point. Any field ending in `_credits_micros` is divided by 1,000,000 for display, with up to 6 decimals (`billing.tsx:584-591`). 4,200 micros → `0.0042 credits`. Usage costs and funding splits use micros; wallet balances and debits use whole credits. The wallet debit rounds its exact funded cost up to a whole credit, so the Usage cost is not the wallet balance change. |
| **Layer** | Which of two independent charges produced a usage row. One request can produce several platform component rows and one resale row. |

| Layer | What is being charged |
|---|---|
| **Platform** | NyxID's fee for the request. In lane mode, the final credential selects either Your own key or NyxID platform key pricing. A missing lane is free. With no lanes, the legacy `platform_billable` / `platform_pricing` settings apply. |
| **Resale** | The downstream vendor's value, resold. Charged **only** when NyxID supplied the master credential (`CredentialClass::NyxidManagedMaster`), the catalog service sets `resale_billable` with a Lago metric code, and the operator switch `BILLING_RESALE_ENABLED` is on. Bring your own key — or have an agent binding swap yours in, or keep the credential on a node — and there is no resale line (`services/billing/route_context.rs:52-58`). |

A **lane** selects the platform-layer price; it is not an extra billing layer.
**Your own key** includes BYOK, agent credential overrides and node-managed keys.
**NyxID platform key** means NyxID supplied the catalog master credential. The
connect dialog and service detail show every component's exact credits per unit,
including input/output/cache-read/cache-write tokens and generated images. Each component
has its own sync state. A pending/failed primary substitutes legacy billing (or free)
for the whole lane, ignoring all extras. With a synced primary, it and synced extras
charge independently; unsynced extras are free and never add a legacy charge.
Resale may add its independent charge to platform-key traffic. Usage continues
grouping by service/model/agent/layer; lane charges retain these dimensions and use the existing wallet/allowance/grant funding display.

Unit prices allow 12 fractional digits (`PRICE_FRACTIONAL_DIGITS = 12`), with exact
integer picocredit rates and truncated legacy micro rates for compatibility. Gross
costs truncate to micros after multiplication; wallet debits ceil the exact remaining
cost to whole credits. These are different amounts, even for a sub-microcredit cost.
An allowance covers only rows with its exact component metric, before grants and wallet.
The server's computed `allowance_metrics` list supplies the dialog's units: configured
primary/components plus legacy while a primary is unsynced or no lanes exist.
Stable primary Lago codes remain `platform_svc_{slug}_{byok|pk}`; components append
`_{metric}` and retain independent durable cleanup. Upgrade ALL replicas before
configuring component prices, new-metric allowances, or prices beyond six decimals;
old binaries cannot read those enums or charge picocredit rates.

Priced input/cache classes do not overlap: OpenAI/Gemini caches are subtracted from
input, while Anthropic's separate cache counts are not. The displayed provider token
breakdown retains the original accounting. Successful image responses count generated
images and may also carry token components. A zero component releases reservations
and generates no Lago event.
Without provider-reported usage, input/output/cache token classes are zero while
legacy `tokens` still uses the byte estimate, so per-class pricing requires
providers that report usage.
Reservations use the request-byte token estimate for `tokens`, `input_tokens`,
and `output_tokens`; cache-read/cache-write reserve one unit because input already
covers cache quantities. Images reserve request `n` (default one); requests and
bytes reserve one unit. Standalone legacy metrics and resale keep their one-unit gate.
See [the lane contract](PLATFORM_KEYS_AND_INFERENCE.md#billing-lanes-and-durable-accounting).

The admin service setting **Charge only NyxID-provided credentials**
(`platform_charge_nyxid_credentials_only`, default off) restricts enabled platform
billing to NyxID master keys and shared OAuth apps, including X. BYO and legacy
untagged OAuth connections, agent overrides, node-managed credentials and no-auth
traffic still produce observability meters but no platform wallet charge. This is
independent of resale, which continues to require a NyxID master key. The restriction
also applies when pricing lanes are configured; shared OAuth tokens retain their
previous user-token price lane.

---

## 1. Page header and period selector

| UI element | Meaning | Source |
|---|---|---|
| **"Wallet balance, credits, and service usage."** | Page subtitle. Static copy. | `billing.tsx:107-109` |
| **Period dropdown** (24 hours / 7 days / 30 days / 90 days / All time) | Rolling windows measured back from the server's current UTC time — not calendar periods. Filters **both** the Usage card and the Top-up history card. Defaults to **30 days**; an unrecognized value also falls back to 30 days. | `billing.tsx:56`, `:111-125`; `handlers/billing.rs:706-715` |
| ↳ **"All time"** | Means two different things. For **Usage** it is the last **3,650 days** (~10 years). For **Top-up history** it genuinely drops the date filter. | `handlers/billing.rs:712`, `:445-450` |

Changing the period remounts the top-up history card (`key={period}`), resetting it to page 1.

**Page access and rollout:**
- Capability: `user.capabilities.billing_available` requires billing enabled + Lago configured + the user's billing feature flag.
- Frontend: `BillingRouteGuard` redirects to `/dashboard` if that capability is false.
- Backend: wallet, top-up, receipt, and benefit reads retain their existing `ensure_billing_rollout()` gates. **Usage has no rollout gate**: authorized readers can see metered but uncharged traffic.

**Whose money is this page about?** The page shows the signed-in person's usage and wallet.
Platform-key usage always bills the requesting person, including when an organization grant
provides access and the resolved service belongs to that organization. The person's billing
rollout flag applies. `BillingOwnerResolver::resolve_for_execution` selects this payer after
final credential selection; an agent override using an org-owned credential still bills the org.
Org BYOK traffic retains org-wallet billing and the org's rollout flag. It does not appear on
this personal page. There is no owner selector. Grants and allowances retain their separate
API support for authorized org reads.

---

## 2. Banner: "Billing is not available on this deployment."

Shown when the usage response's capability block says charging is off (`billing.tsx:66-68`,
`:135-139`). Backed by `BillingReadOnlyBlock` (`handlers/billing.rs:74-80`, `:294-299`):

| Field | Meaning |
|---|---|
| `charging_enabled` | Already `BILLING_ENABLED && lago_configured` — despite the name, not just the master switch. False means nothing is charged, only metered. |
| `lago_configured` | A Lago API URL **and** key are present and the client constructed. **Not** a live health check (`services/billing/mod.rs:33`, `:82`). |
| `source: "usage_meter"` | Numbers come from NyxID's own durable ledger, not Lago's rating engine. Never rendered. |
| `rates_are_approximate: true` | Compatibility flag, always true. New settled rows use persisted exact gross costs; older rows use current cached rates. The UI labels the cost **Est. cost**. |

While it shows, the Top Up input, Checkout button, and Provision Wallet button are all disabled
(`billing.tsx:70-75`, `:176`, `:382`). Because the route guard already blocks the same conditions,
this banner is mostly a defense against stale capability state rather than the normal experience.

---

## 3. Wallet card

Backed by `GET /api/v1/billing/wallet` → `BillingWalletResponse` (`handlers/billing.rs:94-114`;
model at `models/billing_wallet.rs`).

For mixed billing lanes, the allowance unit selector follows [the metering and allowance rules](USAGE_BILLING_LAGO_SPEC.md#40-metadata-only-route-context-r1).

### Credit-benefit recipients

The admin grant, credit schedule, and allowance dialogs share four choices:

| Recipients | Wallet receiving the benefit |
|---|---|
| **All billing owners** (`all_users`) | Every active person's and organization's wallet. |
| **Selected owners** (`selected_users`, `target_user_ids`) | Each selected person's or organization's wallet. Selecting an organization funds its shared wallet. |
| **Organization members** (`org_members`, `target_org_ids`) | Each active person's personal wallet when they have a non-revoked membership in any selected organization, including viewers. The organization wallet receives nothing. |
| **Group members** (`groups`, `target_group_ids`) | Each active person's personal wallet through direct group membership. Parent/child groups are not expanded; organization accounts are excluded. |

Selected lists contain 1–500 unique ids, with only the list matching the recipient kind populated. Organizations must be active; groups must exist. Overlapping memberships pay a person once. One-shot org/group grants reject more than 100,000 resolved recipients; larger populations use schedules.

Grants snapshot recipients when issued. Schedule periods freeze the recipient policy when claimed and page by person id, excluding later signups and later organization joins. Revocations/deactivation can remove people ahead of the cursor. Group membership has no join timestamp: existing people's group changes can affect an unfinished period, while later-created people wait for the next period. Allowances follow live membership in both the balance display and funding path. Removing membership stops new matching immediately; existing consumption-period rows are retained and reservations already admitted can settle. Reading with an organization `owner_id` never applies member benefits to its wallet.

Old rows default the new id lists to empty. Older replicas do not understand the new enum values: upgrade all readers/writers before using member targets (rollback requires migrating every persisted new-kind row).

### Header

| Label | Meaning | API field |
|---|---|---|
| **Owner `<uuid>`** | The billing owner id — on this page, always your own person UUID. Rendered raw, with no display name. | `owner_id` |
| **Status badge** ("Good" / "Past Due" / "Suspended") | The wallet's collection state, title-cased. The badge has no heading, so nothing on screen says it is a *collection* state. | `collection_state` (text) / `suspended` (color) |

| Badge value | Meaning |
|---|---|
| **Good** | Normal. Every new wallet starts here. |
| **Past Due** | An unpaid invoice, service continues. **Nothing in the codebase ever writes this value** — the enum exists in the model and the Zod schema and nowhere else. Aspirational today. |
| **Suspended** | Requests are refused (`WalletSuspended`, error 11307). Set when the overdraft cap is breached (`services/billing/reservation.rs:1194`). |

### The three big numbers

| Label | Meaning | Formula / source |
|---|---|---|
| **Available** | What you can spend right now, excluding overdraft. The number the prepaid gate checks. | `balance − reserved − pending_lago_debits`, saturating (`models/billing_wallet.rs:56-60`) |
| **Balance** | Your Lago wallet balance **as last cached locally** — not a live read at render time. The client prefers Lago's `credits_ongoing_balance` and rounds decimals to whole credits. Because OSS Lago's balance clock job is premium-gated, the reconciler *itself* subtracts the current period's accrued usage, rounding partial credits up against you (`services/billing/reconcile.rs:166-180`). | `balance_credits` |
| **Reserved** | Whole-credit holds for in-flight requests. NyxID reserves a pessimistic estimate *before* forwarding, then trues it up on settle. Money not yet spent but not spendable twice. | `reserved_credits` |

The card does **not** display two fields the API returns and Available depends on:

| Hidden field | Meaning |
|---|---|
| `pending_lago_debits` | Charges settled locally but not yet reflected in a trusted Lago balance refresh. Subtracted immediately so the next request cannot reserve the same money (spec §3.3, R3.1). **This is why `Balance − Reserved` often does not equal `Available` on screen.** The CLI shows it as "Pending Debits" (`cli/src/commands/billing.rs:278`); the web page hides it. |
| `available_with_overdraft_credits` | Available plus the full overdraft cap. Returned by the API, parsed by the Zod schema, never rendered. |

### The three details

| Label | Meaning | Reality check |
|---|---|---|
| **Plan** (`Prepaid` / `Subscription` / `Hybrid`) | *Prepaid* = hard stop when Available hits zero. *Subscription* / *Hybrid* = the non-prepaid gate branch, which may reserve into overdraft. | **Provisioning always writes `Prepaid` and nothing ever updates it** (`services/billing/provisioning.rs:73` is the only production writer). `Subscription` and `Hybrid` cannot currently appear. |
| **Overdraft** | The configured cap on extra spend — a *capacity*, not current debt, and not included in Available. Requires the hidden `has_payment_instrument = true` and a non-prepaid plan. Defaults to `BILLING_DEFAULT_OVERDRAFT_CAP_CREDITS` = 0. | **Inert today.** `has_payment_instrument` is only ever written `false` (`provisioning.rs:77`); no production code sets it true. Combined with the always-`Prepaid` plan, the overdraft branch is unreachable. |
| **Synced** | When `balance_credits` was last refreshed from Lago — by webhook or by the reconcile sweep (`BILLING_RECONCILE_INTERVAL_SECS`, default 300s). | Applies to **Balance only**. Reserved and every usage number are NyxID-local and live. |

### Empty / error states

| State | Trigger |
|---|---|
| **"No wallet provisioned." + Provision Wallet** | Only when the wallet request fails with error code **11301 `BillingNotConfigured`** (HTTP 402). The button calls `POST /billing/wallet`, which idempotently ensures the Lago customer, subscription, wallet, and local row. Toast on success: "Billing wallet provisioned" — shown whether or not anything was actually created. |
| **Error banner + Retry** | Any other wallet failure. Message is the server's, falling back to "Failed to load billing wallet." |

**About 11301.** Across the subsystem it means *Lago client absent* (`services/billing/mod.rs:98-106`),
*rate-cache entry missing or stale* (`reservation.rs:1073`), or *no Lago client for a receipt*
(`handlers/billing.rs:583`). On the wallet request specifically it means **Lago is unconfigured** —
not "you have no wallet", since `GET /billing/wallet` auto-provisions when Lago works
(`handlers/billing.rs:334-342`). See gap 3.

---

## 4. Top Up card

| UI element | Meaning |
|---|---|
| **"Add credits through hosted checkout."** | Payment runs through Stripe *underneath Lago*; NyxID never touches card data. |
| **Credits input** | Whole credits to buy = whole USD. Range **1 to 10,000,000**, step 1, default 100 — enforced on both sides (`billing.tsx:69-75`; `services/billing/provisioning.rs:119-128`). Not an invoice total: no fees or tax shown. |
| **Checkout** | `POST /billing/topup` with a fresh browser-generated UUID as `idempotency_key`, then **navigates the current tab** to the hosted `checkout_url` (`openExternal` = `window.location.assign`, `lib/navigation.ts:4-6`). Clicking it does not mean payment succeeded. |

| Concept | Meaning |
|---|---|
| **Idempotency key** | Stops a double-click from creating two payments. Reusing a key with a *different* amount is a 409 Conflict; reusing it with the same amount returns the existing checkout (`reused: true`). |
| **Paid vs granted credits** | A top-up sends `paid_credits` only; `granted_credits` (Lago's free/promotional bucket, which is *additive*) is forced to `"0"` so a purchase never mints double (`lago_client.rs:302-317`, bug #1050). |

**The creation-status enum you never see.** `POST /billing/topup` returns its own lifecycle state,
distinct from the history table's (`models/billing_topup_session.rs:7-13`):

| Value | Meaning |
|---|---|
| `pending` | Local idempotent session stored; the provider call has not completed. |
| `checkout_created` | Lago produced the wallet transaction, finalized invoice, and hosted URL. Payment still unpaid. |
| `failed` | Creating the Lago transaction or checkout failed — surfaces as an error toast. |

---

## 5. Usage card

Backed by `GET /api/v1/billing/usage?period=`, aggregated from `usage_meter`.
The API groups by service × layer × metric code × model × API key × ack state × billable state;
the table collapses these into expandable service rows.

**Which rows exist:** a non-null `quantity` and a status of `finalized`, or `dead_letter`
that was actually forwarded. Both wallet-backed and observability-only rows are included.
In-flight work and non-forwarded dead letters are excluded. `billable` is determined solely by
whether `wallet_id` exists. Non-billable meters report zero for every cost/funding field, are
never sent to Lago, and render **Free** with **—** cost.

### Totals strip

| Label | Meaning |
|---|---|
| **Est. cost** | Sum of visible row costs in microcredits, displayed to 6 decimals. The same row costs are summed for each service. Missing estimates are skipped by `sum_optional`; a known zero counts, and all-unknown/empty costs show `-`. |
| **Tokens / Requests / Bytes / component units** | Quantities summed separately by metric. A token-metered LLM call contributes tokens, not a request count. |
| **Funding line** | Appears when grants or allowances funded usage: **Funded by grants … · Funded by allowances … · Charged to wallet …**. These are exact pre-rounding costs for new settlements, not whole-credit wallet debits. |

### Table columns

| Column | Meaning |
|---|---|
| **Service** | `service_slug`, else `service_id`, else **Unknown**. Expand for model, agent and layer details. |
| **Usage** | Quantity with its metric. A service spanning multiple metrics says how many metrics; expanded rows show each quantity and provider-reported token breakdown. |
| **Est. cost** | **Gross cost of the full finalized quantity**, including allowance-covered units and grant-funded costs. New settlements use `funding.total_charge_micros`, persisted at settlement time. Historical rows without that field use `quantity × current cached rate`, selecting the row's model-specific rate before the generic metric rate. Repricing affects historical fallback estimates only. |
| **Funding beneath cost** | **grants … · allowance … (1,200 tokens) · wallet …** when any non-wallet funding exists. Uses persisted `grant_funded_micros`, `allowance_funded_micros`, `wallet_funded_micros`, and `allowance_funded_quantity`. Allowance units appear only when the row/service has one metric, avoiding addition of unlike units. |
| **Status → Acked** | Lago accepted the charged usage event or reported a duplicate. It does not mean an invoice was paid. |
| **Status → Pending** | A charged row has not been acknowledged. Forwarded dead-letter rows can remain pending until operator action. |
| **Status → Free** | Metered without cost, never pushed to Lago. A service with charged and free rows labels **Includes free usage** and computes Acked/Pending from its charged rows only. |

For pre-change funded rows, grant funding is the sum of `grant_consumptions.amount_micros`;
allowance units are the sum of `allowance_consumptions.quantity`, valued at the current rate.
Wallet funding is `max(0, estimated gross cost − grant funding − allowance funding)`.
Rows without funding metadata use the same current-rate estimate, funded entirely by the wallet.
Missing rates leave unknown estimates null, including groups mixing exact settlements with
historical usage that cannot be priced. Grant micros remain known from consumption records.
MongoDB aggregates the consumption arrays before responses are built. Costs are summed
consistently from the API rows into both totals and service rows; non-billable rows contribute zero.

Settlement stores this display metadata atomically with `funding.settled = true`. Retries reuse
the stored settlement. Funding order (allowances → grants → wallet), rounded wallet debit,
wallet-funded Lago quantity, usage identity and ledger encoding are unchanged.

Empty state: **No usage in this period.** This means no finalized or forwarded-dead-letter meters
with a known quantity for this person in the selected period. The backend can expose free meters even when
the person has no chargeable wallet.

---

### Admin usage

`/admin/usage` (API: `GET /api/v1/admin/usage`) is available to platform admins
and read-only operators. It reports all credential classes, including free BYOK,
agent overrides, node credentials and no-auth traffic. **`BILLING_ENABLED` must
be on when traffic occurs** for meters to exist; turning it on does not backfill
past usage. No Lago connection or per-user billing rollout is required to read it.

The default window is the last 24 hours; presets are 24h, 7d and 30d. Custom
RFC 3339 windows use `[from, to)` and must be positive and at most 31 days;
inverted or longer windows are rejected. The user picker searches names/emails
and matches either the actor or billing owner, so selecting an organization shows
its members' usage of that organization's wallet. Ranking attributes each row to
the actor and separately identifies a differing billing owner. Deleted identities
say **Unknown user**; their IDs are available only in a tooltip.

Only non-null quantities in finalized rows or forwarded dead letters count.
Requests and provider token classes come exclusively from the unique primary
`{billing_request_id}:platform` row; historical component copies and resale rows
cannot inflate them. New component rows no longer store `token_breakdown`.
Events and per-metric quantities include primary, component and resale rows;
quantities are billing units, not a second count of distinct requests. Cache-read
and cache-write retain provider accounting and can overlap input. **Total tokens**
is derived as prompt + completion; cache counts are displayed separately because
no provider-independent non-overlapping grand total can be reconstructed from
historical `token_breakdown` alone. Estimated tokens without a breakdown remain
visible in metric quantities, not in provider token-class totals.

Costs follow the personal Usage card: exact persisted settlements first, then
current model-specific/generic rates for legacy billable groups; free events cost
zero. Known costs are summed, unknown groups remain null and are skipped by totals.
A **Partial estimate** notice exposes missing historical rates. Grant micros stay
known even without a rate. Funding amounts are pre-rounding costs, not wallet debits.
Ranking is paged by actor × billing owner × service, descending by requests, cost,
a selected metric's quantity, or a token class, with stable identity tie-breakers.
A quantity ranking never adds unlike metrics. Expanding a user shows all their
services in the exact response window. The service picker retains all options in
the selected window/user scope when a service is selected.

All reductions, rate joins, ranking sorts and paging run in MongoDB with a 20-second
server limit (and a 22-second complete-request guard); timeouts return HTTP 503.
One additive non-unique status/date index bounds the reporting scans, at the cost
of another index update on meter inserts and status transitions. No existing index,
meter lifecycle, charging rule, or ledger format changes.

---

## 6. Top-up history card

Backed by `GET /api/v1/billing/topups?page=&per_page=&period=` (`handlers/billing.rs:429-558`),
newest first, 10 per page. NyxID stores only that a checkout was created; the payment outcome is read
live from Lago's credit invoices on each request.

| Column | Meaning |
|---|---|
| **Date** | When the **checkout was created** (`created_at`) — not when payment completed, and not Lago's invoice issuing date, even on a Paid row. |
| **Credits** | Credits requested (= USD). |
| **Invoice** | Lago's human-facing invoice number. `—` while unresolved — invoice attachment is asynchronous, and the handler backfills the link through Lago's wallet transactions when it can. |
| **Status** | See below. |
| **Actions** | **Resume payment** (pending only; **navigates the current tab** to the stored checkout URL). **Download receipt** (paid only; resolves a signed Lago URL and opens it in a **new tab** — `window.open(_blank)`, `use-billing.ts:103`). `—` otherwise. |

### Status values (computed at read time, `handlers/billing.rs:519-535`)

| Status | Meaning |
|---|---|
| **Paid** | The Lago credit invoice reports `payment_status = succeeded`. The only status that enables a receipt. |
| **Pending** | No decisive Lago outcome, the local session is not failed, and it is under 24h old. Covers both local `pending` and `checkout_created`. Resumable when a URL was stored. |
| **Expired** | No decisive Lago outcome and over 24h old. Stripe checkout sessions expire after 24 hours and Lago returns the same cached session per transaction, so it can no longer be completed — start a new top-up. Computed by NyxID; **not** a Lago status. |
| **Failed** | Lago reports `payment_status = failed`, or the local session itself failed before checkout existed. |
| **Voided** | The Lago invoice lifecycle status is `voided`. Checked before payment status. |

**`receipt_available` means eligible, not generated.** It is set `true` for every Paid row
(`handlers/billing.rs:546`). Clicking asks Lago to produce the PDF and briefly retries; if generation
is incomplete the backend returns "The receipt is still being generated; try again shortly".

**Degraded mode:** if Lago is unreachable the endpoint does not fail — it falls back to local session
state (`handlers/billing.rs:470-476`). Everything then reads Pending/Expired/Failed with no invoice
numbers and no receipts.

> **Two status enums share the same words.** The *history* status above is derived per-request from
> Lago. The *session* status in MongoDB (§4) is `pending` / `checkout_created` / `failed` and describes
> only checkout creation. Usage `Pending` (§5) is a third, unrelated meaning: no Lago ack.

---

## 7. Gaps and naming issues

Ranked by how likely a user is to be misled.

1. **Overdraft is inert, but the card shows it as a live capability.** `has_payment_instrument` is
   only ever written `false` (`provisioning.rs:77`) and `plan_kind` is only ever written `Prepaid`
   (`provisioning.rs:73`) — the two conditions the overdraft branch requires
   (`reservation.rs:162-179`). The Overdraft row is a number that can never be spent. Either hide it
   until a payment instrument exists, or label it "not available".

2. **Plan and collection state advertise values the system cannot produce.** `Subscription` /
   `Hybrid` have no writer; `past_due` has **no writer anywhere in the tree** (only the enum and the
   Zod schema). The UI implies a state machine that is not implemented.

3. **The "Provision Wallet" empty state is a dead end — its button is always disabled.** It renders
   only on 11301 (`billing.tsx:604-606`). `GET /billing/wallet` auto-provisions on miss, so 11301
   there means Lago is unconfigured — which is exactly what makes `billingReady` false and disables
   the button (`billing.tsx:382`). The one screen offering "Provision Wallet" is the one where
   provisioning cannot work. It needs an explanation, not a disabled button.

4. **Resolved: metered free usage is visible.** Observability-only meters are included, carry
   zero cost and funding values, and render Free / —. Mixed services retain meaningful charged status.

5. **Platform-key usage is personal; org BYOK usage remains outside this page.** Platform-key
   grants to organizations authorize their members to use NyxID's key; each requesting person
   pays and sees that usage here. Org-owned BYOK credentials still bill the org wallet, whose
   usage is not exposed on this personal page. There is no owner switcher.

6. **"Quantity" totals add incompatible units.** Tokens + requests + bytes summed into one unitless
   number (`billing.tsx:477`, `handlers/billing.rs:282`). Bytes dominate by orders of magnitude, so
   for any mixed account the number is meaningless.

7. **"Requests" and "Bytes" totals undercount.** They sum only rows whose *metric* is that unit
   (`handlers/billing.rs:264-273`). The Requests tile is not "requests you made".

8. **Totals can be partial when historical rates are missing.** New settlements persist exact
   gross cost and funding splits; older rows are recomputed from the current model/metric rate.
   A group containing historical usage with no cached rate has null gross, wallet, and allowance
   costs even when some of its settlements have exact figures; known grant micros are retained.
   `sum_optional` skips unknown costs, includes known zeroes, and shows `-` only if all are
   unknown. The table and totals consistently say Est. cost and sum the same visible rows.

9. **`Balance − Reserved ≠ Available` on screen.** `pending_lago_debits` is subtracted but never
   shown. The CLI displays it; the web page should too, or explain the gap in a tooltip.

10. **Usage "Pending" can be permanent.** Forwarded `dead_letter` rows are included in the query and
    never become Acked without operator action (`handlers/billing.rs:199-207`). They look like
    normal in-flight rows.

11. **Top-up history has no error state.** The component never checks `historyQuery.isError` and
    retries are disabled, so a failed load renders **"No top-ups yet."** (`billing.tsx:216-266`).
    A payment history that silently reads "empty" on failure is the worst possible default.

12. **History status can regress to "Expired".** `credit_invoices` fetches `per_page=100` with no
    pagination and turns any fetch error into an empty list (`lago_client.rs:496-502`,
    `handlers/billing.rs:470-476`). With no matching invoice, age alone relabels an old **paid**
    top-up as Expired.

13. **Inconsistent navigation.** Checkout and Resume payment **replace the current tab**
    (`window.location.assign`); Download receipt opens a **new** tab (`window.open(_blank)`). Leaving
    the app mid-session to pay should at minimum be consistent, and probably a new tab.

14. **"All time" means two different things** — 10 years for Usage, unbounded for Top-up history
    (`handlers/billing.rs:712`, `:445-450`) — from a single shared selector.

15. **"Synced" reads as if it covers the whole card.** It sits beside Plan and Overdraft but applies
    only to Balance.

16. **Owner is a raw UUID** with no display name.

17. **The total cost is unlabeled and looks like a control** — it sits next to a `RefreshCw` icon
    with no caption (`billing.tsx:470-473`). Label it, and either wire up or remove the icon.

18. **`lago_metric_code` is fetched but never shown** (`schemas/billing.ts:28`) — the one field that
    would let a user reconcile a line against their Lago invoice.
