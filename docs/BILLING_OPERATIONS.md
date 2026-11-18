# Billing rollout and merchant taxes

This runbook covers the manual v0 merchant rollout. NyxID owns admission and
exact wallet accounting; Lago owns tax configuration, invoices and prepaid
credits. Billing in Studio and the assistant account panel has a **Billing owner**
selector: Personal and organizations where the signed-in person is an admin.
Balance, setup, top-ups/history, usage, grants/allowances, current bill and finalized
usage invoices follow that selection. Organization wallet management and views
require live organization admin access; members retain their existing consumption
and benefit-read access, without wallet-management access.

## Deployment order

1. Deploy this release to **all** API replicas and workers before assigning any
   Lago taxes. There are no taxed customers before this rollout. Zero-tax
   profiles reproduce the previous accounting behavior.
2. Enable the merchant's billing rollout flag as described below.
3. Provision the relevant owner's wallet, then assign Lago customer taxes.
4. Verify the current bill, gross wallet availability, usage ledger postings and
   receipt ownership using a test merchant and test payments.

New funding rows retain their applied tax snapshot and a tax-accounting marker.
Rows without these additive fields settle pre-tax, as before. There is no tax
backfill, migration fence or ledger rewrite. Once taxes are configured, an older
binary is unsafe for wallet admission.

## Enable a newly signed merchant

In **Admin → Feature Flags**, select `experimental:billing`, add an
**Organization override**, choose the merchant organization and enable it.
This is the supported manual v0 onboarding step; signing up does not enable
billing automatically. `BILLING_ENABLED=true` and an outbound Lago client must
also be configured for charging.

Flag resolution proceeds from the flag definition's default through global,
organization, legacy organization role, legacy organization user, and finally
platform user overrides. More specific decisions win. For a personal wallet,
`billing_rollout_enabled` resolves the person's live active organization
memberships; a disable wins a tie at the same specificity across memberships.
For an organization wallet it resolves the selected organization's member
baseline, then the actor's platform user override. Legacy role/user scopes are
retained in the resolver, though new rollout authoring uses platform scopes.

Ordinary platform-key execution keeps the acting billing principal through
`BillingOwnerResolver::resolve_for_execution`; org members' platform-key usage
bills the person. The explicit org-specialist channel **guest** lane is the
exception: live, admin-enabled guests bill the org wallet for the model and
services, including platform keys, subject to the binding budget and rate limit.
Enabling the billing flag alone never changes an execution's payer. Check that
the Lago customer corresponds to the owner of that execution.

## Organization wallets in the web app

Open `/billing?ownerId=<org UUID>` in Studio or
`/assistant?panel=billing&panelOwnerId=<org UUID>` in the assistant. Owner selection
uses normal URL/history state and resets owner-specific service filters and
pending top-up actions. Only administered organizations appear; an unauthorized
explicit selection is refused, without falling back to a personal wallet.

- `GET /api/v1/billing/wallet?owner_id=<org UUID>` returns the existing wallet or
  JSON `null` for **not provisioned**. It never creates a wallet or subscription.
  Omitting `owner_id` preserves personal GET's existing auto-provisioning.
- **Provision Wallet** calls `POST /api/v1/billing/wallet` with `owner_id`. An
  existing wallet missing its subscription offers **Finish wallet setup**, using
  the same explicit POST to ensure the Lago subscription without changing balances.
- **Add credits** calls the existing `POST /api/v1/billing/topup` with `owner_id`
  and an idempotency key. Lago checkout and payment confirmation remain unchanged.
  Lago's existing `/billing` return restores the selected org and originating shell
  from single-use, account-bound session storage (navigation metadata only, two-hour
  expiry, same tab). Explicit URL selections take precedence. No Lago configuration
  change is needed; a different tab or expired metadata lands on ordinary Billing.
- `GET /api/v1/billing/topups?owner_id=<org UUID>` and
  `GET /api/v1/billing/usage?owner_id=<org UUID>` require the same management ACL;
  existing pagination/period filters still apply. Bills, invoices/receipts and
  grants/allowances use their existing owner-aware APIs. All existing wallet and
  top-up rollout checks still apply; explicit org usage also checks rollout.
- Org-specialist channel guest settings link admins here when wallet/subscription
  setup is missing or the wallet is suspended. Guest admission continues to fail
  closed until ready; enabling a UI view never changes that authority.

CLI parity already exists: `nyxid billing wallet --owner-id <org UUID>` and
`nyxid billing topup --owner-id <org UUID> ...`. Platform-admin credit recipient
pickers already explicitly include organizations.

## Configure Lago taxes

Use the Lago UI or authenticated Lago administration API. Do not put tax rates
or country policy in NyxID configuration.

1. Create a named Singapore GST tax code (for example `sg_gst_9`) at rate `9`.
   Keep `applied_to_organization` **false**: merchant tax policy belongs on each
   customer, not on the default billing entity.
2. Create a named overseas zero-rated code (for example `overseas_zero`) at rate
   `0`. An explicit zero-rated tax is distinct from an empty tax-code list.
3. For each merchant's billing customer, assign the country-appropriate
   `tax_codes`: Singapore → `sg_gst_9`; overseas → `overseas_zero`.
4. Leave plan and charge tax assignments empty. In Lago v1.50 the first nonempty
   scope wins: **charge → plan → customer → customer's billing entity**. Taxes
   within that scope add. An empty customer list inherits defaults; a named 0%
   customer tax overrides them. `applied_to_organization` manages taxes on the
   default billing entity, and other billing entities can have their own taxes.
5. After customer changes, allow the customer webhook to refresh the NyxID tax
   profile. Missing or stale profiles are refreshed once inline before a
   wallet-funded admission. Tax-definition and billing-entity changes have no
   generic v1.50 webhook; their cache refresh follows the existing rate-cache TTL
   and bounded reconciliation sweep. For a rate increase, pause merchant traffic,
   change the tax, wait for the profile refresh, and verify the bill before
   resuming. Do not edit tax assignments while running the demo.

Reconciliation sweeps tax profiles before recovery and again after price
synchronization. Each sweep selects up to 100 due wallets, oldest first.
After selection, it starts refreshes for up to 20 seconds and lets the current
refresh finish. Allow for selection, the 20-second budget, and one final
refresh with up to five seconds of Lago time plus its MongoDB work. The sweep
has no fixed wall-clock maximum. If wallets remain when the budget runs out,
an info log records how many were attempted, completed, and left for the next
sweep. The second sweep can pick them up in the same pass. A refresh error,
including a Lago timeout, still counts as a failed unit in Integrity.

NyxID reads the actual subscription's plan, including customized plan/charge tax
assignments, and the customer's billing entity. Price synchronization round-trips
charge IDs and omits response-only tax fields and `tax_codes` on existing
objects, so Lago preserves its tax assignments.

## Accounting and bill confirmations

- Purchased-credit/top-up invoices are **untaxed** in Lago v1.50. Credits fund a
  later bill; buying them is not the taxable usage event.
- Allowance and grant funding is consumed before the wallet. That portion never
  becomes a Lago usage event and **is not taxed**. It appears in NyxID Usage,
  and is intentionally absent from the Lago bill.
- Wallet funding reserves its pre-tax estimate plus proportional tax, rounded
  upward at one picocredit. Settlement computes `wallet_funded × summed rate /
  100`, rounded half-up at one picocredit. `wallet_funded` remains pre-tax;
  `wallet_charge_credits` and wallet hold/pending-debit counters are gross.
- The same usage settlement entry journals the pre-tax amount to `usage:<row>`
  and tax to `tax:usage:<owner>`, each against `wallet:<owner>`. Existing append,
  dedupe, settlement-lock, account checkpoint and hash-chain rules apply.
- Lago still receives only the pre-tax wallet-funded quantity. Lago rounds fee
  bases and taxes at currency precision, whereas NyxID settles proportional tax
  at pico precision. The remaining bounded cent-level residue uses the existing
  `external:lago` provider-refresh adjustment. The bound scales with the number
  of rounded fees/tax components; it is not a fixed one-cent bound per period.
- A verified empty profile is 0%. Missing/stale tax configuration fails closed
  for wallet-funded admission after one bounded refresh outside the transaction.
  Inline provider reads share a five-second deadline; the complete refresh has
  a six-second ceiling, including capture/publication and failure-backoff writes.
  A persisted provider-failure marker suppresses inline retries for 30 seconds. Provisioning, webhooks and
  reconciliation bypass that backoff. Fully benefit-funded admissions succeed
  even if the best-effort profile refresh fails. Duplicate charges for one
  metric conservatively use the highest additive tax rate, with provider refresh
  reconciling any difference.
- Tax refreshes capture a profile revision, the wallet's tax generation and a
  plan epoch before provider I/O. Existing verified profiles on the same plan
  publish with one conditional update; cold or changed-plan profiles use a
  small guarded transaction. A losing refresh reuses a fresh winner; inline
  admission alone may recapture and fetch once more within the original
  deadline. A second loss fails closed without starting failure backoff, and
  provider failures never retry. Sweep, webhook and post-provision refreshes
  remain single-attempt. Invalidation advances its counter,
  then marks profiles stale and increments their revisions; both statements
  must finish. An interruption between them needs the source to repeat the
  invalidation (#2467 dirty-plan recovery for explicit writers, webhook
  redelivery for webhooks), or later events/TTL refresh. Old binaries ignore
  these additive fields and remain unfenced; complete ordering requires all
  writers upgraded. Rollback needs no data change.
- Current-period subtotal, tax total and total come directly from Lago
  `current_usage`. Per-tax rows are computed from the cached configuration and
  rounded to cents; **Tax rounding** explicitly reconciles them to Lago's tax
  aggregate up to one cent per displayed tax row plus one cent. Beyond this
  tolerance the bill hides cached names/rates and shows only Lago's aggregate
  **Tax**, because the configuration may have changed. There is no retry loop.
- Past invoices are finalized subscription invoices. Their tax rows come from
  `applied_taxes` on the invoice. Invoice total is before prepaid credits; prepaid
  credits applied and amount due are separate, so a wallet-paid bill remains
  visible even when its remaining due is zero. Receipt requests enforce the
  resolved owner and return the same 404 for unknown and foreign invoices.

Implementation reference: [ADR-014](ADR-014-usage-billing-lago.md),
[exact accounting](BILLING_EXACT_ACCOUNTING.md), and
[UI glossary](BILLING_UI_GLOSSARY.md). Provider behavior was verified against
[Lago v1.50.0, 32efd359](https://github.com/getlago/lago-api/tree/32efd35931cca340050c2623d811b14e75d78de9),
particularly `app/services/fees/apply_taxes_service.rb`,
`app/services/fees/paid_credit_service.rb`,
`app/services/invoices/calculate_fees_service.rb`,
`app/services/credits/allocate_prepaid_credits_by_wallets_service.rb`, and
`app/services/{plans,charges}/update_service.rb`.


## Usage delivery capture

The leased `usage_comparison` step compares acknowledged delivery receipts with
one provider current-usage response per owner, customer and subscription. It
uses the immutable emitted event second and exact emitted wallet-funded quantity,
not total metered usage or wallet balance. Prepared/rejected receipts, benefit-only
usage and acknowledgements without a capture sequence do not enter the totals.
Nothing is repaired, charged or sent again by this comparison. Request paths and
Integrity reads make no provider call.

The background usage push prepares an immutable `billing_usage_deliveries` row
before sending. Retries reuse its target, metric, exact quantity representation,
properties and event second (`finalized_at`, falling back to the original
`updated_at`). Acknowledgement advances the target's sequence in
`billing_usage_comparisons` and marks the receipt in the existing meter
acknowledgement transaction. Replay cannot advance it twice. A duplicate provider
transaction id is recorded explicitly; it does not prove an unknown earlier
payload matched this snapshot. Rejections are excluded until a later eligible
attempt is accepted; their count and last-rejection time remain as history.
An eligible retry reopens a rejected receipt as prepared without an expiry,
preserving that history. Receipt state never decides whether to send. Missing, mismatched or unfamiliar
receipt states cannot block the meter acknowledgement or its wallet generation
bump. Unreconciled evidence records a bounded reason and leaves subscription
coverage unknown; only database errors may abort that transaction. Locally
acknowledged benefit-only usage creates no delivery.

Each push pass scans at most 64 prepared receipts using their own
`state/last_checked_at/_id` index. A live unacknowledged meter leaves its receipt
prepared and moves it to the back of that scan. An already acknowledged or
missing meter closes the receipt as acknowledged outside capture, without an
acknowledgement sequence, with a distinct reason and the 100-day expiry. Coverage
uncertainty is published before closing the receipt and survives later successful
captures. Receipt maintenance errors are logged and do not stop usage pushing.

Receipts contain identifiers, quantities, timestamps, delivery state and only the
emitter's property allowlist: `quantity`, `model`, `service_code`, `layer`. They do
not copy meter documents, request/response bodies, error text, actor/key ids,
names or email addresses. Runtime receipts retain the customer and subscription
from existing verification; ordinary receipts retain the memoized wallet target.

A prepared receipt has no expiry while
it is unresolved; acknowledgement or rejection sets `resolved_at` and an
`expires_at` exactly 100 days later, and the `expires_at` TTL index removes it.
Repeated rejection preserves its original deadline; a later accepted delivery
resolves as acknowledged with a new 100-day deadline.
At a steady rate of N acknowledged events per 30-day month, this retains about
100N/30 (3.33N) acknowledged receipts, plus 100 days of rejected receipts and one
row per unresolved transaction, subject to normal TTL-monitor delay. Capture
state has one document per distinct owner/customer/subscription, independent of
event count. Owner erasure and org deletion retain both collections just as they
retain financial usage-meter records; those paths do not shorten either TTL.
Comparison keeps this fixed retention: delayed period discovery cannot safely
shorten it. A period starting 100 or more days ago is unverified even if a cached
projection exists; missing incremental sequences are failed units, never zeroes.

Coverage requires durable activation of `billing:usage-receipts-v1` and a
continuous capture interval beginning strictly before the provider period. Once
per leased comparison pass, a bounded lookup checks retained `security_replicas`
heartbeat rows for an unsupported capability or unknown protocol observed since
that interval began. Rows survive seven days; a successful check older than six
days resets coverage, leaving a one-day evidence-retention margin. Ordinary
deployment gaps and failed database/readiness reads do not reset coverage. A
failed check compares nothing and reports failure without changing the interval.
The readiness heartbeat has no added commands.

Unsupported evidence resets the generation, including a generation that lived
and died between passes. If none is still live and activation exists, the new
interval starts at the successful check; otherwise it remains unset. Rollback to
a visible build without capture invalidates the overlapping period. A later
roll-forward starts a new interval. Builds older than the readiness observation
protocol remain invisible. Lease checkpoints and per-row markers cannot certify
coverage. Initial generation publication succeeds before workers or HTTP serving
start.

A capture-unknown reason advances a durable uncertainty revision and invalid-through
time. Duplicate acknowledgements from normal lost-response retries do not advance
uncertainty: the receipt stores the usage row's creation time in its preparation
write. A duplicate counts normally only when that creation time is after the
continuous-capture boundary. Older or missing creation times keep that period
unverified, remembered in its projection; a new period can recover. Future-dated
events invalidate coverage through their event time, preventing fixed-TTL gaps in
a later period. These markers share the existing sequence update and do not
affect delivery. Legacy uncertainty is adopted conservatively on first comparison.
Each read pins both acknowledgement sequence and uncertainty revision. Before
publishing the pass, the comparator reads the live lease and coverage generation
once. Each target publishes through one atomic update guarded by those pinned
values, the capture reason, one publication per pass and database-time period
bounds. The observation time is the database time of the pass's coverage check.
A lease loss after the read can leave a true observation of pinned evidence;
observations from an old coverage generation are shown as stale.
The documented plan is monthly and NyxID creates calendar-billed subscriptions:
the first eligible period starts at the next calendar month boundary after
continuous support is established (or the next provider-returned anniversary
boundary for an externally configured anniversary subscription). No historical
baseline is inferred. A subscription with continuous acknowledgements is not
compared until it is quiet for ten minutes and shows as waiting. Prepared receipts in the returned period also keep comparison waiting:
an uncertain send may already be included remotely. Integrity shows bounded
prepared/rejected quantity samples separately, explicitly not a complete
historical backlog.

Targets are selected by durable next-visit time, at most 20 per pass. Known current
periods are checked locally first: coverage, ten quiet minutes, then unresolved
prepared receipts. Uncoverable periods wait until their end; busy periods wait
until the last acknowledgement plus ten minutes; prepared deliveries wait ten
minutes. Equal observations wait six hours, pending or confirmed differences ten
minutes, and stable unsupported findings one day. Failed visits use exponential
backoff from five minutes up to six hours. Unknown or elapsed period bounds need
one read to learn the current period. For 100 healthy subscriptions, warming
normally costs 100 reads per period (about 3.33/day for a 30-day period), and equal
steady state costs about 400/day.

Provider reads run at concurrency four with a five-second timeout and no retries;
each publication is one guarded target update with no transaction. It never
writes the lease or coverage control row. Billing keeps the existing shared lease
renewal runner unchanged. Persisted reservations count all attempts against 20
reads per rolling five minutes and 5,760 per UTC day. Restarts do not reset these ceilings.
Discovery reads 100 wallets and existing targets in one batch each, writes only
missing or changed bindings, and avoids an unchanged cursor write. Missing or
changed wallet bindings retire old targets; discovery reactivates a binding if
it becomes current again. This also applies to captured runtime targets. Each
visit folds at most 1,000 receipts; period/layout changes rebuild through the event
index, then acknowledgements fold in sequence order. Incomplete projections stay
visible. Responses are capped at 1 MiB and 256 charges.

A nonzero difference becomes `billing_usage_drift` only after two complete
observations in distinct passes at least ten database-time minutes apart, with
identical local/remote totals and differences. New acknowledgements, changed
periods, changed coverage and failed/incomplete observations break confirmation.
Equality clears immediately on a complete eligible observation. Transport, timeout
and 5xx failures are one failed unit each; other targets continue. HTTP 401/403
also fail a unit with the normal outage backoff because credentials or permissions
may affect every subscription. HTTP 429 is a wait. HTTP 404/410 are
subscription-unavailable findings; other 4xx responses are request-rejected
findings, revisited daily without storing response bodies. No subscription means
no work. Confirmed differences retain the existing
`Billing Lago usage drift detected` warning and increment `drift_alerts` once per
differing metric per published visit.

Integrity separates `billing_usage_coverage` (warming, waits, incomplete or stale
evidence) from `billing_usage_unsupported` and numeric `billing_usage_drift`.
Details are sentences bounded to 4,096 characters. Drift includes the affected
target count, up to five owner/subscription examples, period, first/last observation
and one exact signed local-minus-remote metric difference per example, with a count
of additional differing metrics. Unsupported reasons have counts and up to five
subscription ids. The retained prepared/rejected sample has at most five examples. A stale generation, changed
sequence or elapsed period cannot display a live numeric finding.

The provider contract is taken from lago-api tag `v1.50.0`,
`app/serializers/v1/customers/charge_usage_serializer.rb` and
`app/serializers/v1/customers/usage_serializer.rb`.
`app/services/invoices/customer_usage_service.rb` formats the period bounds with
`iso8601` without fractional seconds. The parser reads the inclusive whole-second
`customer_usage.from_datetime` and `to_datetime` bounds and each
`charges_usage` entry's `billable_metric.code`, `aggregation_type`, and `units`.
The metric code must be bounded and non-blank. For sum metrics, units must be
an exact, non-negative decimal string or integer representable in micro-units.

For `sum_agg` metrics the charge-level `units` is the total over all fees in the
charge. Non-empty `filters` and `grouped_usage` arrays each partition those same
fees: their direct entries' units must add up to the charge total. Nested filters
inside a group are not added again. Malformed breakdowns or entry units produce
`breakdown_mismatch`; missing, null and empty breakdowns need no check.

Other aggregations are omitted from the compared quantities and remembered only
by metric code and a bounded aggregation name in the current read. Their units
and breakdowns are skipped entirely, including absent or unrepresentable units. After local deliveries for the period have been
folded, a delivered metric with a non-sum aggregation produces
`non_sum_aggregation:<name>` instead of a numeric comparison. A non-sum metric
without deliveries in that period does not block the sum metrics. Structural
findings, including invalid sum-charge units and duplicate metric codes across
any aggregations, still block the subscription. Duplicate codes are checked
before aggregation or units, including duplicates across sum and non-sum charges.

Every other key is ignored, whether absent, scalar, object, or array. v1.50.0
already sends `presentation_breakdowns`, `pricing_unit_details`, and
`total_aggregated_units`; none replaces or adds to the charge's `units`.
`backend/src/services/billing/fixtures/lago_current_usage_v1_50_0.json` mirrors
that tag's serializer keys with synthetic values: one charge has two filters,
and one has grouped usage with nested filters. It includes the nested
`presentation_breakdowns`, `invoice_display_name`, `values`, and `grouped_by`
fields, and omits `amount_currency` inside breakdown entries as the serializer
does. Smaller fixtures also cover absent metadata and exact `"12.0"`, `"0.0"`,
`"0.000001"`, and integer units.

Integrity records one bounded reason code per finding; only the code and a
validated aggregation name can come from a response in a stored finding:

| Reason code | Integrity sentence |
| --- | --- |
| `missing_customer_usage` | the provider omitted customer_usage |
| `missing_charges_usage` | the provider omitted charges_usage or returned it in a non-array form |
| `charge_not_object` | a charges_usage entry is not an object |
| `too_many_charges` | the provider returned more than 256 charges |
| `non_sum_aggregation:<name>` | a delivered metric has non-sum provider aggregation '{name}' |
| `breakdown_mismatch` | a provider usage breakdown does not add up to its charge total |
| `duplicate_metric` | the provider returned the same metric more than once |
| `missing_metric_code` | the provider omitted a metric code |
| `unsupported_provider_units` | the quantities cannot be represented exactly |
| `invalid_provider_period` | the provider period is invalid |
| `response_too_large` | the response exceeds the comparison size limit |
| `subscription_unavailable` | the provider no longer exposes the subscription |
| `provider_request_rejected` | the provider rejected the usage request |
| `provider_target_mismatch` | the returned customer or subscription does not match |
| `delivered_metric_missing` | a delivered metric is missing from provider usage |
| `unsupported_usage_shape` (retired) | the previous release recorded an unsupported provider usage shape |

For `non_sum_aggregation`, `<name>` is the provider enum only when it is at
most 32 ASCII characters from `[a-z_]` and is nonempty; malformed or absent names
are stored as `other`. Subscriptions carrying the retired
`unsupported_usage_shape` finding can be re-read before their stored next visit.
The original indexed due query runs first. If it returns fewer than 20 rows, a
second query fills the remaining slots from non-retired, future-dated rows with
that exact legacy reason, excluding rows already selected. The future-date bound
avoids scanning legacy rows that were already due. It uses the named
`usage_comparison_legacy_shape_due` index on `{next_visit_at, _id}`, partial on
`{retired_at: null, comparison.reason: "unsupported_usage_shape"}`. Once those
findings are replaced, the index is empty: the fallback examines no documents or
keys, and the original due query still examines only due rows. Startup creates
the additive index through the existing index setup; no migration or operator
action is required. Old replicas' writes naturally repopulate it during rollout.
If the legacy lookup's named index is missing, that lookup contributes no rows
for the pass. A single warning records only the MongoDB error code; due rows
still proceed. The old findings retain their schedule and become due within a
day. Only IndexNotFound (27) or BadValue (2) with MongoDB's specific missing-hint
message takes this fallback; other errors propagate. The due query is unchanged.

The limits remain 20 subscriptions per pass, 20 reads per rolling five minutes,
and 5,760 per UTC day. Ordinary due rows have priority. Successful publication
replaces the old reason and resumes the ordinary schedule; existing coverage
waits still apply. Both versions read the unchanged observation schema and string
reasons.

## Billing inventories on Integrity

`billing_hold_backlog` and `billing_stuck_requests` are read-only inventories.
They have severity `info` whenever their evidence is readable and complete,
regardless of reservation age or recovery eligibility. An incomplete, truncated
or unreadable scan, invalid rows, or unreadable process-registry evidence makes
the affected inventory `action`; unreadable recovery totals also make the
forwarded-request inventory incomplete. The bounded count of rows with no holds
remains a valid lower bound when its index-key limit is reached, so that count
alone does not require action. Failing recovery steps are reported separately
by `billing_reconcile_steps`, and unexplained holds by the hold observations.

The summaries use “holds” because the inventory includes allowance units as
well as credits. Sentences describe counts, hold kinds and the oldest creation-age
bucket in plain English. Counts of billing rows are not counts of individual
funding reservations: one meter row may reserve from several grants or allowance
periods. The sweep can release unused holds from eligible reservations after
rechecking them; it does not promise completion on the next pass. A request may
have both eligible reservations and reservations belonging to running work, so
these sentences retain that distinction. Live work has no new lifetime limit.

Rows with no holds are not necessarily dead requests or eligible for cleanup.
Their remaining count is a lower bound in sent request rows, with no money tied
up in them. Cleanup's closed-row and request counts are cumulative completions
for that category, not a subset of the currently remaining rows. Request counts
are distinct within each described category; categories and hold kinds overlap.

Information entries say “No action is needed. This inventory is read-only.”
Action entries name the relevant checks: the row limit and backlog, query
performance, stored billing fields and schemas, database access and indexes,
process registry records, recovery counters, or the count query for rows with no
holds. They direct the operator to the stages in Details, database health and
available server logs, then to reload the page for a fresh read. These readers do
not guarantee a dedicated server log line for every failure. Multiple causes
combine fixed checks without repeating them, in at most two sentences.

The exact technical inventory is retained in the additive `technical_details`
field, collapsed under **Details** on the page. The `detail` field contains the
short English sentences. Each diagnostic explicitly supplies `severity` as
`info` or `action`. Older frontends ignore the additive fields and retain their
previous diagnostic coloring; new frontends retain the previous code allowlist
when an older backend omits severity. No operator step is required.

For the hold backlog, each kind is an independent consistent snapshot. Age uses
meter creation and database time. Held amounts exclude recorded returns and
matching applied locks. Resources count positive outstanding holds; billing
rows include zero-hold handoffs. Wallets and owners are distinct row identities.
Allowance units remain separated by metric in Details; groups can overlap in
resources and owners. Incomplete totals are not population totals. Each kind
reads at most 10,000 rows within its existing query and client deadlines.

The forwarded-request inventory classifies potential wallet, grant and allowance
holds in one pinned snapshot, reading at most 10,000 distinct candidates in
128-row pages within four seconds. Incomplete groups mean at least the reported
counts and amounts. Its separate count examines at most 100,001 index keys per
source and may undercount overlapping funding or returned holds. Siblings are
bounded at 129 per row; locks and receipts are deducted in the same snapshot.
Live ownership includes the registry's 45-second window plus a two-minute
margin. Legacy eligibility requires an extra hour before every live generation's
registration. Missing or unreadable registry evidence authorizes no cleanup.
Reading either inventory changes no counters, quantities or charging decisions.

## Reconcile ownership and verification status

The periodic billing maintenance pass acquires the MongoDB lease
`billing:reconcile`, including its immediate startup tick. A contender skips its
tick. The holder uses the shared `CLUSTER_LEASE_TTL_SECS` (default 30 seconds)
and `CLUSTER_LEASE_RENEW_SECS` (default 10 seconds), renews throughout the pass,
and releases only its exact token. Crash takeover is eligible at lease expiry;
work resumes on the next `BILLING_RECONCILE_INTERVAL_SECS` tick (default 300
seconds), up to 330 seconds after the last successful renewal, excluding outages
and backlog. No interval or billing admission setting changes.

On renewal loss the pass finishes its current write protocol and stops before
starting another unit. Sweep rows, price components, and schedule chunks have
cooperative stop points. A stopped credit-schedule walk leaves its existing
300-second period lease to expire: that progress row has no exact ownership token
to release safely, and deterministic grant IDs make chunk replay safe. A graceful
shutdown stops future ticks and requests the same cooperative stop, then awaits
the current unit and exact-token release. Renewal continues while that unit
finishes. If the hard shutdown deadline aborts the pass, shutdown deliberately
leaves its lease to the shared TTL (30 seconds by default); bulk shutdown release
never transfers this ownership under an uncertain provider operation. An already
sent provider request cannot be recalled by a MongoDB lease, and an in-flight
unit may finish after expiry. Existing row claims, financial locks and dedupe
remain necessary. This supervisor does not serialize request-time plan writers,
wallet/rate/tax snapshots, or purchased-credit expiry against other entry points.

After exact-accounting readiness, a failed maintenance step records its stable
error code and the pass continues with independent later steps. The caller still
receives one failed-pass result listing failed steps. Integrity's
`billing_reconcile_steps` diagnostic shows each step's consecutive failed passes
and database-time first/last failure; a successful attempt clears that step,
while a drain/configuration skip preserves it. Status uses the existing
`billing:reconcile` lease checkpoint and is published once with its fenced release;
a clean pass adds no database commands. Old lease holders ignore and retain the
checkpoint and keep their earlier stop-on-error behavior, so counts cover passes
observed by upgraded holders. No rolling-update action is required. A hard abort
or lost lease cannot publish a completed pass; leaf recovery markers remain the
authority for unfinished work. Per-document failures already handled inside a
step retain their existing retry and diagnostic behavior. Same-schema failure
entries with unknown step names are preserved and displayed. A newer checkpoint
version is retained as `status_newer_version` without failing an otherwise healthy
pass. Malformed or older formats are replaced with fresh status on release and
reported once in that pass's aggregate log; they do not fail subsequent passes.

Money recovery decodes and processes each selected resource or request separately.
A failed unit retains its financial claims, locks and evidence while other selected
units continue. The pass diagnostic includes `failing_units` for the latest batch;
a query/cursor error has no invented count. Selections retain their existing filters,
sorts, indexes and caps. Claim loops exclude failed identities only within that pass;
existing leases, retry times and expiry rotation remain authoritative. More poison
rows than a selection cap can still defer healthy work across passes and are reported
as failures. No retry fields or indexes are added. Live claims, pending journals,
expiry quiet periods and retry backoff are waits, not failing units; provider read
outages and malformed documents are failures. Old replicas ignore the additive status
count and retain their existing batch behavior; rolling updates and rollback need no
operator action. No amount, financial fence, receipt or evidence rule changes.

Provider maintenance uses the same per-unit isolation for tax profiles, service and
machine prices, usage emission/comparison, wallet balances and rate-cache publication.
Provider outages fail their units after existing retry state is saved; plan contention,
readback still pending, event rate limits and stale snapshot publication are waits.
Rate publication retries a definitively rejected row's remaining chunk against the
same captured epoch, never a freshly read epoch. The clean command paths and existing
selection bounds are unchanged; no new durable marker or index is introduced.

Automatic and manual chain verification discard losing `Ok` scans, always log a local
`Broken` at error level and retry its publication up to three times (on conflict,
keeping the stored break at an earlier or equal sequence, or returning an error on
exhaustion), so a late break after repair may restore an alarm that the next scan
clears by rechecking its break cursor.
The optional status `revision` distinguishes publications even within one
millisecond; missing revisions remain readable. `CHAIN_VERIFY_INTERVAL_SECS=0`
still disables both automatic chain verification and the separate account
reconciliation runner.

Deploy all replicas before relying on these guarantees. Build `a8bb6c617` does
not take the maintenance lease and still replaces chain status unconditionally.
During a mixed rollout an old pass can overlap the new holder and an old status
write can overwrite a new result. The new writer detects a changed starting
snapshot, including an old replacement without a revision, but cannot fence an
old writer's subsequent write. These are the pre-existing mixed-fleet races;
no financial format, movement or per-row fence changes in this release.
