# Billing UI Glossary

What every term on `/billing` actually means, where the number comes from, and how it is grouped, and which details are expandable.

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
| **Credit micros** | One millionth of a credit — fixed-point, no floating point. Any field ending in `_credits_micros` is divided by 1,000,000 for display, with up to 6 decimals (`lib/billing-display.ts`). 4,200 micros → `0.0042 credits`. Usage costs and funding splits use micros; wallet balances and debits use whole credits. The wallet debit rounds its exact funded cost up to a whole credit, so the Usage cost is not the wallet balance change. |
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

## 1. Billing & Usage tabs and filters

`/billing` is the actual application page. **Billing** contains Wallet, Credit grants &
free usage, and Top-up history. **Usage** contains the filters, Spend / Activity / Tokens
summary, and expandable service records. The page uses live billing APIs and inherits
the application's fonts and theme.

| UI element | Meaning | Source |
|---|---|---|
| **Billing / Usage** | Defaults to Billing. Tab, usage period and service selection persist in the URL and browser history. | `schemas/billing.ts`, `pages/billing.tsx` |
| **Service filter** | Defaults to All active services: services with recorded usage in the selected period. Options use catalog display names. Unused catalog services are excluded. A selection absent from a newly loaded period resets to all. | `lib/billing-usage.ts`, `pages/billing.tsx` |
| **Time filter** (24 hours / 7 days / 30 days / 90 days / All time) | Rolling windows measured back from the server's current UTC time. Filters usage summaries and all usage details. Defaults to 30 days. | `pages/billing.tsx`, `handlers/billing.rs` |
| **Top-up history period** | Independent from Usage, defaults to 30 days. Changing it resets history to page 1. | `components/billing/billing-topup-history.tsx` |
| **All time** | Usage means the last 3,650 days; Top-up history drops the date filter. | `handlers/billing.rs` |

Wallet and benefits always show current balances, independently of either history filter.
Dates use the viewer's local timezone. The personal usage API has no time buckets or arbitrary
date ranges; the time filter does not imply daily or hourly charts.

**Page access and rollout:**
- Capability: `user.capabilities.billing_available` requires billing enabled + Lago configured + the user's billing feature flag.
- Flag precedence is `Default → Global → Org → User`: each explicit value replaces the less-specific value. An active org's disable overrides a global enable, and an explicit user enable overrides that org disable. Conflicting org overrides at the same scope resolve to disabled. Clearing an override restores inheritance; revoked memberships do not contribute org overrides.
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

### Free allowance bundles

Admins choose a service and recipients once, then add one or more units with their
own quantity and recurrence. The Free allowances table groups these rows by
`bundle_id` (legacy singletons use their own id). Each bundle lists every unit and
shows **Active**, **Partially disabled**, or **Disabled**. Edit reviews each unit's
before/after values. Removing a unit disables its row on save, retaining its
consumption history. Editing loads active units only when any are active; disabled
units stay disabled unless explicitly re-added. If all rows are disabled, editing
loads all units so the admin can edit and re-enable them by saving. Saving enables
only the listed units, and the change review shows their re-enablement.
Disable/Enable applies to all rows, including previously removed units. A bundle's service cannot change.

A bundle is only an admin grouping: every unit retains an independent allowance
id, period, and exact-metric funding match. Funding order and user balances are
unchanged; the user balance response exposes the optional bundle id read-only.
Legacy single-row API clients remain supported. Bundle mutations use the same
MongoDB transaction requirement as the repository's other atomic multi-document
mutations, so standalone deployments fail closed instead of writing partial grants.

### Visible balance and expandable breakdown

The Wallet card retains its existing Add credits dialog, balance, freshness and
View breakdown interaction. A help icon beside Wallet explains funding order and
why the available balance can differ from the provider balance. A Suspended badge
appears when applicable. Owner IDs and normal collection-state badges are not displayed.

| Label | Meaning | API field |
|---|---|---|
| **Available** | Credits spendable now, excluding reservations, unsettled charges and expiry holds. Does not include overdraft. | `available_credits` |
| **Updated** | Relative age of the provider-synced balance. Does not describe usage freshness. | `balance_synced_at` |
| **Balance** | Last provider-synced balance; whole credits. | `balance_credits` |
| **Reserved** | Whole-credit holds for in-flight requests. | `reserved_credits` |
| **Pending** | Charged locally, awaiting provider sync. | `pending_lago_debits` |
| **Expiring** | Credits held while expired purchases are removed; shown when nonzero. | `pending_topup_expiry_credits` |
| **Overdraft** | Configured extra capacity, shown when nonzero. Actual eligibility also depends on plan and payment instrument. | `overdraft_cap_credits` |
| **Plan** | Configured plan kind. | `plan_kind` |

All rows after Updated are in the expandable breakdown. Its formula reads
`Available = Balance - Reserved - Pending - Expiring`. The API's
`available_with_overdraft_credits` is not used as the spendable balance.

### Credit grants & free usage

One compact card uses `GET /billing/grants`, `GET /billing/allowances` and catalog names.
Grant rows group by eligible service scope and show available credits (remaining minus
reserved), a used/original gauge and Details. The short amount rounds to two decimals;
hover/focus and Details retain exact microcredit precision.

Free usage groups by service, with separate Tokens, Cache and Other coverage values.
Only identical metrics, recurrence and period boundaries share a subtotal. Unlike units
and different windows never share one balance. A subtle grayscale bar gives each allowance
an equal-width segment; each segment's consumed, reserved and remaining portions are
relative to its own limit. One tooltip explains percentage used, excluding reservations,
and lists each metric's percentage. Zero usage stays zero.

Details retain each grant's original, used, remaining and reserved amount, expiry,
issuance, status and activation; and each allowance's limit, consumed, reserved, remaining,
recurrence, period start and reset/expiry. Expiry dates stay in these expansions.
Help beside Credit grants and Free usage derives scope and cadence from the API response.
Funding remains matching platform allowances first, then eligible grants in expiry order,
then wallet credits. Empty benefits are omitted; loading, retry and rollout states remain.

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

## 4. Add credits dialog

| UI element | Meaning |
|---|---|
| **"Add credits through hosted checkout."** | Payment runs through Stripe *underneath Lago*; NyxID never touches card data. |
| **Credits input** | Whole credits to buy = whole USD. Range **1 to 10,000,000**, step 1, default 100 — enforced on both sides (`billing.tsx:69-75`; `services/billing/provisioning.rs:119-128`). Not an invoice total: no fees or tax shown. |
| **Continue to payment** | `POST /billing/topup` with a fresh browser-generated UUID as `idempotency_key`, then **navigates the current tab** to the hosted `checkout_url` (`openExternal` = `window.location.assign`, `lib/navigation.ts:4-6`). Clicking it does not mean payment succeeded. |

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

## 5. Usage tab

Backed by `GET /api/v1/billing/usage?period=`, aggregated from `usage_meter`.
The API groups by service × layer × metric code × model × API key × ack state × billable state;
the table collapses these into expandable service rows.

**Which rows exist:** a non-null `quantity` and a status of `finalized`, or `dead_letter`
that was actually forwarded. Both wallet-backed and observability-only rows are included.
In-flight work and non-forwarded dead letters are excluded. `billable` is determined solely by
whether `wallet_id` exists. Non-billable meters report zero for every cost/funding field, are
never sent to Lago, and render **Free** with **—** cost.

### Summary and expandable details

| Group | Meaning |
|---|---|
| **Spend** | Estimated credits, covered by benefits (grants + allowances), and wallet-funded cost. Any unknown component makes that total Unavailable; known records remain readable. Empty usage totals are zero. |
| **Activity** | Metered request quantity, services used, images and bytes. These are metric quantities, not unique HTTP request counts. |
| **Tokens** | Total-token metric plus separate input/output and cache-read/write metrics. Missing classes show a dash, not a fabricated count. Token totals and classes may overlap and are never added together. |
| **All metrics & funding** | Exact quantities grouped into Tokens, Cache, and Requests & other units. Funding shows all three sources; allowance-covered units stay separate by metric. All-service API request/byte/event totals remain available here. |
| **Service rows** | Catalog display name, quantities, estimated cost and settlement status. Services are grouped under AI models, Connected apps, or Other services using catalog inference metadata. |
| **Service expansion** | Metered quantities and funding, then Models, agents & billing layers, with every returned aggregate record accessible. |
| **Full metering & funding details** | Per-record costs, funding, allowance-covered units, requests, bytes, events, original provider token breakdown, meter code and agent-key identity. |

Estimated cost is the gross cost of the full finalized quantity, including benefit-covered
units. New settlements use persisted exact gross costs; historical rows use current cached
model/metric rates. Funding is an exact pre-rounding cost, not a whole-credit wallet debit.

**Acknowledged** means Lago accepted a billable event or duplicate, not that an invoice was
paid. **Pending** means a charged row is unacknowledged; forwarded dead-letter rows can stay
pending until operator action. **Free** rows have no charge and display a dash for cost.
Mixed groups say Includes free usage and compute acknowledgement from charged rows only.

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

The default window is 24h; presets are 24h, 7d and 30d. Preset starts round down
to the UTC hour (`hour(now − duration)`), with `to = now`; they may include up
to 59 additional minutes. The footer shows the exact from/to timestamps. Custom
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

Daily operational rollups supply whole UTC days; hourly rollups supply the
remaining edge hours and the partially filled live hour. The worker folds stable
rows older than **60 seconds**, regardless of hour completion; only the most recent one or two minutes ordinarily remain raw. A
journal timestamp bound, published before increments, lets a reader safely use
a partially filled end bucket. Custom windows retain index-supported raw scans
for partial edge hours that contain folded data beyond their boundaries; their
cost scales with those edge rows. One aggregation combines totals, ranking,
service options and live-tail counts. Covering indexes reduce common hourly and
daily summaries without fetching their documents. Rates are read once and joined
through a MongoDB literal lookup map. Legacy per-display-group truncation and
missing-rate masking remain intact through internal cost partitions; API keys
and ack state are not dimensions of either tier’s primary key.

The footer says **Backfilling history · rollups complete through <time>** when
`freshness.rolled_up_through` precedes the window start; otherwise it says
**Live · includes N unfolded rows**. The watermark is an exclusive source-time
bound, no longer hour-aligned: after a gap-free sweep it reaches `now − 60 s`,
or the oldest unfolded terminal timestamp if an unstable row still blocks it.
It starts at the Unix epoch during initial backfill. Exact charged rows fold
after reservation release and funding settlement without waiting for Lago ack,
so a Lago outage does not grow their tail. Legacy charged rows still need ack
or terminal dead-letter status to freeze their display partition. Unacked
charged rows without `forwarded = true` remain live: a later dead-letter
transition could remove them from the dashboard predicate. Forwarding is
monotonic in the meter lifecycle.

The worker uses the billing reconcile interval capped at 60 seconds; zero disables
it. Raw batches contain at most 2,000 rows; hourly-to-daily bootstrap batches
contain at most 200 hourly documents. Both claims are capped at 4 MiB of
serialized BSON, including source IDs and increments. Bootstrap stops at the
largest fitting prefix; oversized raw claims halve their source count and
recompute the increments before publication. Sources outside the claimed prefix
remain pending for subsequent batches. Ticks have a 45-second inter-batch
budget while the watermark is over two hours behind or the daily bootstrap is
incomplete, and 20 seconds once caught up (at most 100 batches either way). Invalid journal state returns an error and
the worker logs a metadata-only warning and retries on the next tick.

Each source batch increments both tiers under the same journal sequence and
`last_batch` fences, then marks its raw sources. Replica-set folds commit both
tiers and source marks atomically. Reporting validates the journal around idle
reads and uses snapshots during active folds. Standalone readers validate the
journal around their aggregation. Contention and snapshot-expiry retries are
bounded to eight attempts; the last completed result (or one ordinary read if
snapshots never completed) is returned with `freshness.validated = false`. The
footer then adds **Updating totals**. Such results can temporarily include an
in-flight fold; they are not presented as validated. The durable ordered journal
and monotonic sequence fences still provide exactly-once persisted folding on
standalone MongoDB. An interrupted write between tiers replays the same batch;
a fence prevents either increment from being applied twice. Existing hourly-only
history is copied to daily summaries automatically in bounded batches through
that same journal, even if its raw events have expired. Old in-flight hourly-only
batches finish first. Readers keep using hourly summaries until the journal
publishes `daily_ready`; new folds then maintain both tiers together. Journal
reads use majority concern so a reader cannot activate the daily tier before
its completed backfill is available to snapshot reads. This needs no operator
migration or new configuration. Every index is additive. Both tiers have no TTL,
while raw rows retain their existing TTL. Historical whole-hour windows remain exact
from rollups after raw expiration. Exact arbitrary partial-hour boundaries need
retained raw rows; expired event timestamps cannot be reconstructed from hourly
summaries.

The existing 20-second server limit and 22-second complete-request guard remain;
timeouts return HTTP 503 and the client does not automatically retry.

---

## 6. Top-up history card

Backed by `GET /api/v1/billing/topups?page=&per_page=&period=` (`handlers/billing.rs:429-558`),
newest first, 10 per page, with its own period selector and server pagination. Loading
failures show a retry action rather than an empty history. On phones, the same fields
stack into purchase rows. NyxID stores only that a checkout was created; the payment outcome is read
live from Lago's credit invoices on each request.

| Column | Meaning |
|---|---|
| **Date** | When the **checkout was created** (`created_at`) — not when payment completed, and not Lago's invoice issuing date, even on a Paid row. |
| **Credits** | Credits requested (= USD). |
| **Credit expiry** | Purchased-credit expiry date, or expired date and exact expired amount. Paid purchases awaiting expiry synchronization show Pending sync. |
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

## 7. Limits to keep in mind

- This is the personal billing page. Organization BYOK usage needs a separate owner-scoped view.
- The personal usage API returns period aggregates, not hourly/daily time buckets. All time
  is capped at 3,650 days for Usage but unbounded for Top-up history.
- Metered requests and bytes are not unique traffic counts. Each metric keeps its own units;
  reported request/byte/event counts remain in expanded details.
- Historical usage can have unknown costs. The page marks affected totals Unavailable instead
  of displaying a partial sum as complete.
- Estimated gross cost, settled funding and rounded wallet debits need not be equal.
- Catalog lookup failures have an explicit retry state; a readable slug fallback remains until
  names load. Catalog names are authoritative even when an administrator chooses a slug-like name.
- Forwarded dead-letter usage can remain Pending until operator action.
- Receipt availability means eligible, not already generated. Receipt generation can require retry.
- When Lago is unreachable, top-up history may use its existing local-session fallback and omit
  invoice/receipt data. The frontend does not infer that missing provider information is payment success.
- The existing wallet provisioning/unconfigured behavior is retained; billing readiness must be
  verified before payment or provisioning actions become available.
