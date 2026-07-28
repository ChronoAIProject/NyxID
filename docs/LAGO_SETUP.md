# Lago Billing Setup

Practical guide for wiring NyxID's usage billing to a real Lago instance
(v1.48.x, self-hosted OSS). Companion to `docs/ENV.md` (variable semantics)
and `docs/USAGE_BILLING_LAGO_SPEC.md` (architecture). Everything in this
document was validated against a live deployment; the Caveats section lists
the failure modes that are not obvious from Lago's docs.

## 1. NyxID environment

```bash
BILLING_ENABLED=true
LAGO_API_URL=https://billing.example.com/api   # see URL caveat below
LAGO_API_KEY=<lago api key>
LAGO_PLAN_CODE=starter                          # must exist in Lago
LAGO_WEBHOOK_SECRET=<shared secret>             # only useful if Lago can reach NyxID
LAGO_PAYMENT_PROVIDER_CODE=<stripe connection code>  # enables top-up checkout
```

URL caveat: `LAGO_API_URL` must point at the Lago **API**, not the dashboard.
The client appends `/api/v1/` unless the URL already ends in it. Reverse
proxies that serve the dashboard at the root and strip a leading `/api`
before forwarding to the API container need the extra prefix (the example
above resolves to `.../api/api/v1/...`, which is correct for that layout).
Verify with `curl <LAGO_API_URL>/health` — it must return the Lago API
version JSON, not HTML.

## 2. Lago-side configuration

1. **Billable metrics** (Settings -> Billable metrics), all SUM over the
   `quantity` event property:
   - `platform_tokens`
   - `platform_requests`
   - `platform_bytes`
   - `resale_tokens` / `resale_requests` (only if resale is used)

   Events also carry `model`, `service_code`, and `layer` properties for
   grouping and charge filters.
2. **Plan** (code = `LAGO_PLAN_CODE`): monthly, $0 base fee, paid in
   arrears, with a standard charge per metric you intend to price. Metrics
   without a charge aggregate but cost nothing.
3. **Entitlement feature**: create a Feature with code `all_services` and
   attach it to the plan. The reservation gate requires a subscription
   entitlement of `*`, `all_services`, or the specific service slug;
   without one, every billable request fails with 402
   `plan_entitlement_required` (code 11303).
4. **Stripe** (for top-ups): connect Stripe under Integrations and note the
   connection **code** (not the display name) — that is the value for
   `LAGO_PAYMENT_PROVIDER_CODE`. Set the connection's success redirect URL
   to `<FRONTEND_URL>/billing` so checkout returns to NyxID.
5. **Webhooks** (optional): point Lago at
   `<BASE_URL>/api/v1/webhooks/lago` with `LAGO_WEBHOOK_SECRET`. When Lago
   cannot reach NyxID (local development), skip this — the reconcile sweep
   pulls the same state on `BILLING_RECONCILE_INTERVAL_SECS` (default 300s).

Customers created by NyxID are linked to the Stripe connection only when
`LAGO_PAYMENT_PROVIDER_CODE` is set at creation time. Customers provisioned
before that must be linked manually (Lago UI: customer -> payment provider).

## 3. Rate cache (required, manual for now)

The reservation gate sizes credit holds from the `billing_rate_cache`
MongoDB collection. The sweep that should mirror Lago's plan charges is not
implemented, so rows must be seeded manually and kept "fresh" by a long TTL:

```javascript
db.billing_rate_cache.insertOne({
  _id: "platform_tokens:*",
  lago_metric_code: "platform_tokens",
  credits_per_unit_micros: NumberLong(5),   // 0.000005 credits per token
  synced_at: new Date(),
})
```

Seed one row per metric you price (unpriced metrics can use `0`, which
reserves nothing but keeps the gate open). Set
`BILLING_RATE_CACHE_TTL_SECS` high (e.g. `31536000`); with the default 900s
the rows go stale in 15 minutes and billable requests are rejected with
"billing rate cache is stale". Keep `credits_per_unit_micros` in sync with
the Lago charge price manually. 1 credit = 1 USD (wallets are created with
`rate_amount: "1"`).

## 4. What gets charged

Charging is **opt-in per catalog service**. Everything is metered for
observability, but credits are only reserved and Lago events only pushed
for services whose billing config has `platform_billable: true` (admin
Services page -> Edit -> Billing, or `PUT /api/v1/services/{id}` with
`{"billing": {"platform_billable": true}}`).

- User-added keys (BYOK) and custom endpoints are never charged.
- Token metering applies to `/llm` routes and to proxied services whose
  slug starts with `llm-`; other services meter requests/bytes.
- Token counts use the provider-reported `usage` object (Chat Completions
  and Responses API shapes, JSON and SSE), falling back to bytes/4.
- The resale layer (charging for the platform's own upstream key at a
  markup) is separate and additionally gated by `BILLING_RESALE_ENABLED`,
  `resale_billable` on the service, and a final credential class of
  `nyxid_managed_master`. See the spec for details.

## 5. OSS Lago limitations

- **Wallet ongoing balance never updates.** Lago's
  `RefreshWalletsOngoingBalanceJob` is premium-licensed
  (`return unless License.premium?`); on OSS, `credits_ongoing_balance`
  stays equal to `credits_balance`, which itself only moves when a period
  invoice settles. NyxID compensates: the reconcile sweep subtracts the
  period's `current_usage` (which works on OSS) from the synced balance, so
  the local balance reflects accrued usage within minutes.
- **Balances are whole credits.** Sub-credit usage shows in the usage
  panel's cost column before it moves the balance; availability rounds
  accrued usage up to whole credits.
- **Month-end transient**: when the period rolls over, accrued usage resets
  slightly before the invoice debits the wallet, so the balance can briefly
  read high until the invoice settles.

## 6. Troubleshooting

| Symptom | Cause |
|---|---|
| `Billing provider unavailable: Not Found` on startup backfill | `LAGO_API_URL` points at the dashboard or the wrong path prefix (section 1) |
| Wallet creation fails with `rate_amount` validation | Lago requires wallet and credit amounts as decimal strings; fixed in the client — update NyxID |
| 402 `plan_entitlement_required` (11303) on billable requests | Plan has no entitlement feature (section 2.3) |
| 402 only on some services | Expected: those services have `platform_billable` enabled and the wallet cannot cover the reservation |
| Top-up fails `no_linked_payment_provider` | Customer not linked to the Stripe connection (section 2.4) |
| Events visible in Lago but usage aggregates to zero | Events pushed without `external_subscription_id`; fixed in reconcile — update NyxID |
| Balance never changes | See section 5; also confirm the Lago `clock` scheduler and a worker consuming the `clock` queue are running |
| Billable requests rejected with stale/missing rate | `billing_rate_cache` row missing or older than `BILLING_RATE_CACHE_TTL_SECS` (section 3) |

Lago error bodies (`error_details`) are surfaced in NyxID's
`Billing provider unavailable` messages, so backend logs identify the
failing field directly.
