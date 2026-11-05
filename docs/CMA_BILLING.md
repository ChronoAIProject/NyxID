# CMA allocation billing through NyxID

Billing is disabled by default. Deployment seeds the first immutable rate card
from the approved resource prices in PR #2151. Admins can publish subsequent
versions at runtime on **Admin → Billing → Settings**, without redeploying.
Prices live in `sandbox_rate_cards`; quote snapshots, exact usage settlement and
version-specific Lago codes preserve historical prices. The implementation
reuses NyxID's decimal validation, Credits arithmetic, funding, tax-aware Lago
charge synchronization, rate cache and chained audit infrastructure.

| Metric | Quantity unit | Initial credits per unit (USD, pre-tax) |
| --- | --- | --- |
| `sandbox_cpu_millis_seconds` | Allocated CPU millicores × seconds | `0.000000010000` |
| `sandbox_memory_mib_seconds` | Allocated RAM MiB × seconds | `0.000000001000` |
| `sandbox_disk_gib_seconds` | Retained disk GiB × seconds | `0.000000040000` |

The seed is idempotent, audited, and never overwrites runtime changes. It does
not enable billing. There is no separate wallet or top-up path. Ordinary HTTP
proxy calls cannot create allocation-time usage; these units require dedicated
window admission. Zero-price trials stay outside paid admission.

## Authority and account eligibility

Configure a catalog identity for Sandbox entitlements and funding in
`CMA_BILLING_SERVICE_ID` and one
General-purpose service account UUID in `CMA_BILLING_SERVICE_ACCOUNT_ID`. Its
issued JWT and live account both need `billing:sandbox:write`. AuthUser verifies
the JWT/token row; each money/evidence transaction also fences account activity,
purpose, scope, credential generation, token expiry and revocation. Human,
delegated, relay and API-key authentication is rejected.

Only this trusted producer can assert an owner and lifecycle operation. CMA must
establish owner authorization and price consent before requesting a window;
these routes must never be exposed as an arbitrary-owner public billing proxy.
Personal owners need both `experimental:billing` and
`experimental:cma-sandbox-billing` rollout flags, an active account,
provisioned wallet, good collection state and ordinary service entitlement.
The APIs do not provision any provider objects. Keep ordinary CMA control-plane
proxy services (such as `cma`/`cmaeg`) on their ordinary pricing. The CMA rate card
is the authority for allocation prices; editing a service credential lane does
not edit the CMA rate card.

The effective subscription must be active, bound to the accepted customer and
configured plan, monthly calendar billing, explicit customer timezone `UTC`, and
USD denomination. Readback verifies exactly one standard
charge per metric, the accepted exact price, no charge filters/minimums/proration,
and nonrecurring `sum_agg(quantity)` definitions. Missing proof, stale cache,
partial sync and provider failures deny new admission.

## Producer API

All paths below are relative to `/api/v1/billing/sandboxes`. JSON rejects unknown
fields, prices and amounts. Identities and operation/request keys are canonical
UUID v4; Sandbox IDs use CMA's `sbx_` plus 32 lowercase hexadecimal characters.

`POST /quotes` accepts:

```json
{
  "owner_id": "<owner UUID>",
  "sandbox_id": "<CMA Sandbox ID>",
  "allocation": {"cpu_millis": 2000, "memory_mib": 8192, "disk_gib": 64}
}
```

The response contains `quote_id`, owner/Sandbox/catalog identities,
`maximum_allocation`, `rate_card_id`, `rate_card_version`, the three exact decimal
`credits_per_unit` strings and
`accept_before` (15 minutes). The quote alone reserves no money. Each dimension
of the full profile must be positive.

`POST /windows` accepts:

```json
{
  "request_id": "<new request UUID>",
  "quote_id": "<quoted UUID>",
  "owner_id": "<owner UUID>",
  "sandbox_id": "<CMA Sandbox ID>",
  "runtime_generation": 1,
  "operation_id": "<durable owner-authorized lifecycle operation UUID>",
  "previous_window_id": null,
  "seconds": 300,
  "allocation": {"cpu_millis": 2000, "memory_mib": 8192, "disk_gib": 64}
}
```

All resource holds, existing `usage_meter` rows, quote acceptance, the window
receipt and the per-Sandbox cursor commit in one majority transaction. Funding
uses the existing allowance → grant → wallet order, service limits, taxes and
exact Credits arithmetic, with no overdraft. Any failure rolls back all holds.
A paused window may specify zero CPU/RAM and positive retained disk, within the
accepted maximum profile. All-zero windows are rejected.

The receipt returns `window_id`, `request_id`, `quote_id`, `owner_id`,
`sandbox_id`, `runtime_generation`, whole UTC `covered_from` and exclusive
`covered_until`, `allocation`, `components`, `evidence_accepted`, and nullable
`last_evidence_revision`. Each component has `usage_row_id`, `metric`,
`reserved_quantity`, and nullable `finalized_quantity`. The request UUID binds
the original operation and predecessor; those two fields are not echoed. First coverage starts prospectively. Coverage is
at most 300 seconds and shortens at a UTC month boundary or a scheduled rate
change; use the returned cutoff. A renewal names the immediate prior window, keeps its generation,
accepted quote and operation, and starts exactly at its cutoff. Renewal is
admitted only in the final 30 seconds before that cutoff. An expired generation
cannot be revived by a late renewal. After a rate cutoff, an old-quote renewal
returns 409 `conflict`. Complete prior evidence, obtain a fresh quote and new
owner-authorized operation, then start the next generation. This prevents an
old authorization from silently accepting a higher tariff.

The next generation requires the next integer generation, a fresh unaccepted
quote and unique accepted operation, ended prior coverage and complete evidence
for all earlier windows. This conservative boundary does not yet support early
generation replacement inside existing coverage. An owner/Sandbox binding cannot
be transferred by changing a request.

Persist the receipt in CMA before changing provider desired state. After a lost
response, retry the exact request or call `GET /receipts/{request_id}`. The same
key and payload recover the same receipt even when new admission or the provider
is unavailable. A changed payload conflicts. A receipt is finite coverage, not
permission to restart an already stopped or superseded lifecycle operation.

`POST /windows/{window_id}/evidence` accepts a bounded retained ledger range:

```json
{
  "owner_id": "<owner UUID>",
  "sandbox_id": "<CMA Sandbox ID>",
  "runtime_generation": 1,
  "intervals": [{
    "revision": 42,
    "started_at": 1793690000,
    "ended_at": 1793690060,
    "allocation": {"cpu_millis": 2000, "memory_mib": 8192, "disk_gib": 64},
    "quantity": {"cpu_millis_seconds": 120000, "memory_mib_seconds": 491520, "disk_gib_seconds": 3840}
  }]
}
```

The request body is limited to 512 KiB and 1,024 intervals. Only retained CMA
events qualify; projected read-time tails are not evidence. Revisions and UTC
intervals must be contiguous, original allocation arithmetic must match, and
every funded second needs explicit evidence. The service intersects intervals
with funded coverage, excludes historical/unfunded time and rejects allocation
above the reservation. Missing tails and future observations cannot imply a
refund. An explicit zero-use interval may release unused holds after that period
has elapsed; there is no early-cancel or inferred-zero shortcut.

Finalization atomically pins original ledger revision digests across windows
and all component quantities, then uses existing settlement/journal recovery.
Identical evidence replay is idempotent; changed evidence conflicts. The receipt's
`evidence_accepted` means evidence was persisted, not that every journal/provider
operation has finished. Receipt recovery and evidence submission continue when
new admission is disabled; owner deletion/suspension does not erase an admitted
liability.

`GET /cursors/{sandbox_id}` returns the durable consumer checkpoint:

```json
{
  "owner_id": "<owner UUID>",
  "sandbox_id": "<CMA Sandbox ID>",
  "runtime_generation": 1,
  "last_window_id": "<latest admitted window UUID>",
  "covered_until": 1793690300,
  "last_evidence_revision": 42,
  "evidence_covered_until": 1793690060,
  "last_evidence_window_id": "<latest evidenced window UUID>"
}
```

Evidence fields are null until the first acceptance. Submit windows in increasing
`covered_from` order: an earlier window missing evidence blocks a later one.
Adjacent windows must continue at the last revision or its immediate successor;
the same boundary revision may straddle windows only with identical retained
payload. The cursor, original evidence, window receipt and finalized usage rows
commit atomically. A restart reads this checkpoint and resumes from CMA's durable
outbox/retained per-Sandbox events, retaining any boundary revision until all
intersecting windows are acknowledged. It must not use a global maximum revision,
a bounded inventory/UI listing, or an Agent conversation `through_seq`.
`last_evidence_revision` in an individual window response is that window's last
accepted revision; the cursor endpoint gives the Sandbox-wide checkpoint.
Evidence ingestion is this authenticated push consumer, not a second poller or
Lago exporter. There is no unbounded historical import or backcharge.

## Errors, idempotency and retries

All successful calls above return HTTP 200. The `request_id` JSON field is the
window idempotency key; there is no additional idempotency header. Persist its
exact payload before the first POST and reuse it for every retry. Quotes have no
monetary effect and may be requested again after a lost quote response. Evidence
idempotency is `(window_id, exact parsed payload)` with cross-window retained
revision digest checks. Unknown JSON fields are rejected.

Application errors use the existing envelope `{"error":"...","error_code":1004,"message":"..."}`.
Axum JSON/body rejections may instead return plain text: malformed JSON is 400,
an unsupported content type is 415, invalid field types or unknown fields are
422, and an oversized body is 413. Correct those requests before retrying.
The authoritative numeric registry is `backend/src/errors/mod.rs`; clients must
branch on both HTTP status and `error`, not human message text:

| HTTP | Error | Retry rule |
| --- | --- | --- |
| 400 | `validation_error` | Fix malformed IDs, arithmetic, coverage, or evidence before retrying. |
| 401 | `unauthorized` / token error | Obtain a fresh service-account JWT, retaining the original request key/payload. |
| 403 | `forbidden` | Stop admission; operator must repair the configured producer/live scope or owner authority. |
| 404 | `not_found` | Receipt/cursor absent at this read. An in-flight transaction may still commit; retry the **same** request, never allocate a replacement key for an ambiguous attempt. |
| 409 | `conflict` | Recover receipt/cursor first. Changed-payload or generation conflicts are terminal for that payload; earlier missing evidence must be delivered before a later window. |
| 402 | `insufficient_credits`, `wallet_suspended`, `plan_entitlement_required` | Do not extend funded capacity. Retry only after funds/account/entitlement changes. |
| 402 | `billing_provider_unavailable` / `billing_not_configured` | Admission remains closed; recover the receipt first, then retry the same request with bounded exponential backoff. This repository currently maps provider billing errors to 402. |
| 429 | `rate_limited` | Honor `Retry-After` when present; use bounded backoff. |
| 5xx or transport timeout | database/internal/unknown outcome | Treat as ambiguous. GET the receipt and retry the identical POST with bounded backoff and jitter. |

Never extend a provider deadline without an accepted matching receipt. Retry
receipts/evidence even when admission is disabled. On evidence timeout, GET the
window receipt by its admission `request_id`; `evidence_accepted=true` acknowledges
its retained payload, while settlement/provider completion may still be pending.
A 512 KiB body limit also applies; oversized JSON receives HTTP 413. Do not split
an incomplete window into independent finalization calls; use smaller future
windows if a full retained range would exceed 1,024 intervals.

## Ledger and provider recovery

The existing NyxID reconciler is the only Lago emitter. It verifies the accepted
quote, exact usage row, frozen settlement price and effective subscription before
emission. It never redirects historical usage through the wallet's current
subscription or silently reprices it through the current cache. Only the shared
wallet-funded quantity goes to Lago; grants and allowances use existing accounting.

The meter row's `finalized_at` is the last covered second, placing delayed
evidence in its usage month. The window's `finalized_at` records receipt time.
Provider late-event limits and finalized invoice behavior must be verified on
a controlled subscription before activation. A provider rejection follows the
existing retry/dead-letter path and retains the pending debit.

Each rate version owns separate Lago metric codes (for example
`cma_sandbox_cpu_millis_seconds_v1`). New versions add charges through the existing
full-plan update machinery, preserving unrelated charge IDs and prior versions.
Historical rate cards, cache prices and provider charges are retained; no edit
reprices accepted usage. Missing/unsynchronized current versions deny admission
instead of falling back to a previous price. Provider readback still proves the
owner's effective subscription before admission and delayed emission.

The new collections contain lifecycle/evidence state only:

- `sandbox_rate_cards`: immutable configured prices, effective-from timestamps, actor and provider synchronization state.
- `sandbox_billing_quotes`: accepted rate-card and provider snapshots.
- `sandbox_billing_windows`: atomic request receipts and usage-row references.
- `sandbox_billing_cursors`: ordered, nonoverlapping coverage and atomically committed evidence checkpoints per producer/Sandbox.
- `sandbox_billing_evidence`: original retained revision quantities and digests.

Existing wallets, allowances, grants, rate cache, usage rows, exact settlement,
ledger and Lago emission remain authoritative for accounting. Runtime rows are
excluded from generic abandonment/release; absent evidence retains holds.

## Enablement and runtime rate configuration

`experimental:cma-sandbox-rate-sync` independently controls CMA rate-card writes
to Lago. It defaults to **true** and accepts **global overrides only**, because
the provider plan is shared across owners. Admins can change it at runtime on
the existing Feature Flags page or with
`PUT /api/v1/admin/feature-flags/experimental:cma-sandbox-rate-sync` and
`{"target_kind":"global","enabled":false}`. Set `enabled` to true to resume.
The reconciler reads the live flag before each sync batch; no redeploy or new
environment variable is required. Disabling prevents new sync batches, leaves
already-synced cards and provider charges untouched, and retains pending/failed
versions for a later retry. A sync batch already in flight may finish.

CMA sync errors and the bounded batch timeout are logged with metadata only.
Pending/failed cards remain retryable; an ambiguous timeout retains the existing
lease until expiry. Ordinary tax refresh, usage emission and wallet/rate refresh
continue in the same reconciliation pass. The sync flag does not enable or
disable admission: the existing admission gates below and synchronized-price
proof still apply.

The existing admin Feature Flags page and
`PUT /api/v1/admin/feature-flags/experimental:cma-sandbox-billing` control the
runtime rollout. No separate editor or end-user self-activation API is added.
Use `{"target_kind":"user","target_key":"<person UUID>","enabled":true}` for one
person, or `{"target_kind":"global","enabled":true}` for the global baseline.
The default is false. The Billing Settings environment table shows the independent
fleet gate and configured producer/catalog identities. Existing resolution applies global, matching organization
cohorts, then a personal override; an organization cohort targets its members'
personal wallets and does **not** authorize an organization payer. Set the same
override to false to disable it. `CMA_BILLING_ENABLED=false` is the independent
fleet-wide admission kill switch, even if a per-user flag is true.

With the flag off, no new quote/window or charged coverage is admitted. Previously
funded windows retain their contractual liability: receipts, evidence ingestion,
and settlement remain recoverable after disablement. Disabling does not refund
unproven usage, discard evidence, or backcharge unfunded intervals. Both rollout
flags, `BILLING_ENABLED`, configured producer/service, approved synchronized prices,
and account/provider eligibility must permit admission.

`GET /api/v1/admin/billing/cma-rate-cards` lists the latest 100 versions for
admin/operator inspection. `POST` on the same path is admin-only and accepts:

```json
{
  "expected_version": 1,
  "effective_from": "2026-12-01T00:00:00Z",
  "rates": {
    "sandbox_cpu_millis_seconds": "0.000000010000",
    "sandbox_memory_mib_seconds": "0.000000001000",
    "sandbox_disk_gib_seconds": "0.000000040000"
  }
}
```

All three rates must be positive decimal strings with at most 12 fractional
digits and within the existing per-unit price cap. Unknown/missing resources,
negative/zero rates, exponent notation and excess precision are rejected.
`effective_from` must be a whole UTC second at least 301 seconds in the future
and later than the latest published version. `expected_version` prevents a
stale editor or lost-response retry from publishing an unintended second version;
a 409 requires reloading the history. Versions cannot be edited or deleted.

Publication and its chained audit entry commit together. The existing billing
reconciler synchronizes pending versions in bounded leased batches while quote admission verifies
both the local version and effective Lago subscription. Publication does not
enable billing, and a sync failure does not erase a published configuration.
Failed provider readback schedules durable synchronization repair for the same
immutable version. A shared transaction fence serializes price publication with funded admission;
the notice period exceeds the maximum funded window and windows stop at the
next price cutoff. Existing receipts remain valid at their original prices.

The initial seed asset is `backend/assets/cma-sandbox-rates.json`. Its exact
rates correspond to $0.036/vCPU-hour, $0.0036864/GiB RAM-hour and
$0.000144/GiB retained disk-hour, as specified in #2151. Lifecycle requests never
contain client-supplied rates. The seed is deployment configuration only;
charge paths always read the persisted effective version.

## Rollout and remaining launch work

Finance's rollout condition ([PR #2204](https://github.com/ChronoAIProject/NyxID/pull/2204#issuecomment-6651534655))
allows merging/deploying and an internal-team pilot with the current card.
Before enabling any external user, publish a revised rate card aligned with the
final pricing policy, confirm its Lago synchronization and effective date, and
align the website and policy with that card. Seeding or deploying does not satisfy
this external-rollout prerequisite; Finance supplies the pricing detail privately.

Keep `CMA_BILLING_ENABLED=false` until every API reader and billing worker knows
the three new enum units. Disabling admission after writing these units does not
make rollback to old readers safe. No historical usage is backcharged.

The CMA lifecycle adapter is a separate required change: persist generation and
owner intent, consume these receipts, fence provider `shutdownTime` to coverage,
checkpoint/send retained intervals, recover ambiguous responses, and pause
compute on failed renewal. Paused retained storage must not be deleted when
funding fails; unfunded residual allocation is platform cost under the pending
retention policy. Native shutdown deadlines are desired control-plane state,
not a physical stop guarantee during a node/control-plane partition.

Paid launch additionally requires controlled subscription/invoice verification,
end-to-end crash/race tests across both systems, and exactly one enabled Lago
emission path. CMA's existing direct exporter must remain disabled when using
NyxID. The user enables billing after deployment; seeding prices never activates charging.
