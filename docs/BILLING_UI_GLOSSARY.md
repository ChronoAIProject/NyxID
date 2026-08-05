# Billing UI Glossary

What every term on `/billing` actually means, where the number comes from, and where the label
lies to you.

**Scope:** the logged-in Billing page (https://nyx.chrono-ai.fun/billing). Source of truth is the
code in this worktree — `frontend/src/pages/billing.tsx` + `backend/src/handlers/billing.rs`,
identical to `main` at time of writing (last billing UI commit `f0691e23`).

**Design background:** [ADR-014](./ADR-014-usage-billing-lago.md) (decisions) and
[USAGE_BILLING_LAGO_SPEC.md](./USAGE_BILLING_LAGO_SPEC.md) (implementation). Lago is the billing
engine; NyxID owns the meter, the wallet cache, and the spend gate. Lago owns pricing and invoices.
Where the ADR's intent and the shipped code disagree, this doc describes **the code**.

> Written by two independent passes (Claude + GPT/Codex) over the same source, then reconciled;
> every claim below was verified against the code.

---

## 0. The three words everything else is built on

| Term | Meaning |
|---|---|
| **Credit** | The billing unit. **1 credit = 1 USD.** NyxID creates every Lago wallet in USD with `rate_amount: "1"`, so credits are 1:1 with the wallet currency (`services/billing/lago_client.rs:93-95`, `:272-278`). Wallet amounts are always whole integers. |
| **Credit micros** | One millionth of a credit — fixed-point, no floating point. Any field ending in `_credits_micros` is divided by 1,000,000 for display, with up to 6 decimals (`billing.tsx:584-591`). 4,200 micros → `0.0042 credits`. Cost *estimates* use micros; wallet balances never do. |
| **Layer** | Which of two independent charges produced a usage row. One request can produce one of each. |

| Layer | What is being charged |
|---|---|
| **Platform** | NyxID's own fee for brokering the request — the proxy hop itself. Opt-in per service via the admin `platform_billable` flag; services default to free (`models/service_billing.rs:15-19`). |
| **Resale** | The downstream vendor's value, resold. Charged **only** when NyxID supplied the master credential (`CredentialClass::NyxidManagedMaster`), the catalog service sets `resale_billable` with a Lago metric code, and the operator switch `BILLING_RESALE_ENABLED` is on. Bring your own key — or have an agent binding swap yours in, or keep the credential on a node — and there is no resale line (`services/billing/route_context.rs:52-58`). |

---

## 1. Page header and period selector

| UI element | Meaning | Source |
|---|---|---|
| **"Wallet balance, credits, and service usage."** | Page subtitle. Static copy. | `billing.tsx:107-109` |
| **Period dropdown** (24 hours / 7 days / 30 days / 90 days / All time) | Rolling windows measured back from the server's current UTC time — not calendar periods. Filters **both** the Usage card and the Top-up history card. Defaults to **30 days**; an unrecognized value also falls back to 30 days. | `billing.tsx:56`, `:111-125`; `handlers/billing.rs:706-715` |
| ↳ **"All time"** | Means two different things. For **Usage** it is the last **3,650 days** (~10 years). For **Top-up history** it genuinely drops the date filter. | `handlers/billing.rs:712`, `:445-450` |

Changing the period remounts the top-up history card (`key={period}`), resetting it to page 1.

**Page access is gated three times:**
- Capability: `user.capabilities.billing_available` requires billing enabled + Lago configured + the user's billing feature flag.
- Frontend: `BillingRouteGuard` redirects to `/dashboard` if that capability is false (`components/billing-route-guard.tsx`).
- Backend: every billing endpoint calls `ensure_billing_rollout()`, a staged-rollout feature flag. Owners outside it get **403 "Billing is not enabled for this account"** (`handlers/billing.rs:304-313`).

**Whose money is this page about?** Always **your personal wallet**. Wallet, top-up, and history all
call `resolve_for_wallet_management(actor, None)`, which passes your own id and resolves to
`PaysFrom::Personal` (`services/billing/owner_resolver.rs:50-58`); usage keys directly on
`auth_user.user_id` (`handlers/billing.rs:188`). Org-billed usage is recorded under the *org's*
owner id and is **not visible anywhere on this page** — see §7.

---

## 2. Banner: "Billing is not available on this deployment."

Shown when the usage response's capability block says charging is off (`billing.tsx:66-68`,
`:135-139`). Backed by `BillingReadOnlyBlock` (`handlers/billing.rs:74-80`, `:294-299`):

| Field | Meaning |
|---|---|
| `charging_enabled` | Already `BILLING_ENABLED && lago_configured` — despite the name, not just the master switch. False means nothing is charged, only metered. |
| `lago_configured` | A Lago API URL **and** key are present and the client constructed. **Not** a live health check (`services/billing/mod.rs:33`, `:82`). |
| `source: "usage_meter"` | Numbers come from NyxID's own durable ledger, not Lago's rating engine. Never rendered. |
| `rates_are_approximate: true` | **Every cost on this page is an estimate.** Always true. Never rendered. |

While it shows, the Top Up input, Checkout button, and Provision Wallet button are all disabled
(`billing.tsx:70-75`, `:176`, `:382`). Because the route guard already blocks the same conditions,
this banner is mostly a defense against stale capability state rather than the normal experience.

---

## 3. Wallet card

Backed by `GET /api/v1/billing/wallet` → `BillingWalletResponse` (`handlers/billing.rs:94-114`;
model at `models/billing_wallet.rs`).

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

Backed by `GET /api/v1/billing/usage?period=` (`handlers/billing.rs:183-301`), aggregated from the
`usage_meter` ledger and grouped by service × layer × metric × ack state.

**Which rows exist at all:** only charged usage — a non-null `quantity` and `wallet_id`, and a status
of `finalized` (or `dead_letter` that was actually forwarded). In-flight `reserved`/`forwarded` work
and observability-only metering are excluded (`handlers/billing.rs:191-207`).

### Header

| UI element | Meaning |
|---|---|
| **"Per-service quantity and estimated cost."** | The only place on the page that says *estimated*. |
| **Unlabeled number beside the ↻ icon** | The **total estimated cost** for the period, `totals.estimated_credits_micros / 1e6`. `-` when no estimate exists at all. It is not a refresh button. |

### Totals strip

| Label | Meaning | Caveat |
|---|---|---|
| **Quantity** | Sum of every row's `quantity`. | Rows use **different units**; tokens, requests, and bytes are added together. |
| **Requests** | Sum of quantity **for rows whose metric is `requests`**. | A token-metered LLM service contributes 0, even though it served requests. |
| **Bytes** | Sum of quantity **for rows whose metric is `bytes`**. | Same shape of undercount. |
| **Events** | Count of underlying ledger documents. | Not a request count: one request can write a platform *and* a resale row, and WS/SSH sessions write one per flush. |

### Table columns

| Column | Meaning | Source |
|---|---|---|
| **Service** | `service_slug`, else `service_id`, else **Unknown** (neither present on the row). | `UsageMeterRow.service_slug` |
| **Layer** | `Platform` or `Resale` — see §0. | `UsageMeterRow.layer` |
| **Metric** | The unit counted: **Tokens** (LLM tokens), **Requests** (one per call), **Bytes** (payload volume). The admin's `platform_metric` override wins; otherwise the heuristic is WebSocket/SSH → bytes, `llm-` slug → tokens, everything else → requests (`handlers/proxy.rs:3136-3158`). | `UsageMeterRow.metric` |
| **Quantity** | Metered units for that group, in the unit named by Metric. | `$sum: quantity` |
| **Cost** | `credits_per_unit_micros × quantity` from the cached rate card (`billing_rate_cache`), in credits to 6 decimals. **Recomputed at read time from the *current* rate** — not a rate captured with the usage — so a repricing retroactively changes the cost shown for old usage. | `handlers/billing.rs:250-256` |
| **Status → Acked** | Lago accepted the usage event, *or* reported its transaction id as an already-applied duplicate. Says nothing about whether an invoice was paid. | `lago_acked: true` |
| **Status → Pending** | Lago has not acknowledged the row. Usually a new or retrying row — but forwarded `dead_letter` rows are included by the query and stay unacked **permanently** without operator action, so Pending is not always transient. | `lago_acked: false` |
| **Status → Free** | Metered without cost, never pushed to Lago. | `billable: false` — unreachable, see gap 4 |

Empty state: **"No usage in this period."** — which does not prove there was no traffic, only no
finalized wallet-backed rows. On a usage error the page still passes an empty array down, so the
error banner and this text can appear together.

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

4. **The "Free" badge is unreachable.** The usage query filters to `wallet_id != null`, then derives
   `billable` from the presence of that same field (`handlers/billing.rs:196`, `:223`) — so
   `billable` is always `true`. The `Free` badge and the `—` cost path (`billing.tsx:511-521`) are
   dead code. Either drop the branch or stop excluding observability rows.

5. **Org-billed usage is invisible here.** Usage keys on `auth_user.user_id` and the wallet resolves
   to `PaysFrom::Personal`, but an org member's requests are metered under the *org's*
   `billing_owner_id`. That spend appears on nobody's billing page, and the org wallet has no UI at
   all. There is no owner switcher.

6. **"Quantity" totals add incompatible units.** Tokens + requests + bytes summed into one unitless
   number (`billing.tsx:477`, `handlers/billing.rs:282`). Bytes dominate by orders of magnitude, so
   for any mixed account the number is meaningless.

7. **"Requests" and "Bytes" totals undercount.** They sum only rows whose *metric* is that unit
   (`handlers/billing.rs:264-273`). The Requests tile is not "requests you made".

8. **Cost is an estimate at the current rate, and the table never says so.** `rates_are_approximate`
   is always true and never rendered; the estimate is recomputed from today's cached rate, so a
   repricing rewrites history. Worse, `sum_optional` silently skips rows with no cached rate
   (`handlers/billing.rs:754-762`) — the total can be **partial with no warning**, and only shows `-`
   when *every* row lacks a rate. Header should read "Est. cost" and flag partial totals.

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
