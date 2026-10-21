# Billing rollout and merchant taxes

This runbook covers the manual v0 merchant rollout. NyxID owns admission and
exact wallet accounting; Lago owns tax configuration, invoices and prepaid
credits. The Billing page shows the acting person's current bill and finalized
usage invoices. The bill and invoice APIs also accept an authorized organization
`owner_id`; managing an organization wallet requires organization admin access.

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

Platform-key execution, including NyxBot usage, always bills the **acting
person** through `BillingOwnerResolver::resolve_for_execution`. Enabling the
demo organization's flag enables personal billing through membership resolution;
it does not redirect that traffic into the organization's wallet. The web Billing
page remains personal. Check that the actual Lago customer corresponds to the
owner who will execute the demo, rather than assuming it is the organization.

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
  The inline provider timeout is five seconds total; a persisted failed-attempt
  marker suppresses inline retries for 30 seconds. Provisioning, webhooks and
  reconciliation bypass that backoff. Fully benefit-funded admissions succeed
  even if the best-effort profile refresh fails. Duplicate charges for one
  metric conservatively use the highest additive tax rate, with provider refresh
  reconciling any difference.
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
