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
