# CMA allocation billing through existing NyxID prices

This integration is disabled by default and does not activate the pricing
proposal in PR #2151. Prices are authored through the existing service billing
editor/API, normalized by `billing::pricing`, synchronized by the existing Lago
price sync, and selected by `BillingRouteContext`. The platform lane must have
exactly these three synchronized metrics, one primary and two components:

| Metric | Quantity unit |
| --- | --- |
| `sandbox_cpu_millis_seconds` | Allocated CPU millicores × seconds |
| `sandbox_memory_mib_seconds` | Allocated RAM MiB × seconds |
| `sandbox_disk_gib_seconds` | Retained disk GiB × seconds |

There is no second price editor, price table, wallet, top-up path or startup
seed. Quote snapshots preserve the accepted ordinary prices and provider
binding as evidence. They are not an authoring authority. Regular HTTP proxy
calls cannot create allocation-time usage; these units require dedicated window
admission. Zero-price trials stay outside paid admission.

## Authority and account eligibility

Configure the ordinary catalog service UUID in `CMA_BILLING_SERVICE_ID` and one
General-purpose service account UUID in `CMA_BILLING_SERVICE_ACCOUNT_ID`. Its
issued JWT and live account both need `billing:sandbox:write`. AuthUser verifies
the JWT/token row; each money/evidence transaction also fences account activity,
purpose, scope, credential generation, token expiry and revocation. Human,
delegated, relay and API-key authentication is rejected.

Only this trusted producer can assert an owner and lifecycle operation. CMA must
establish owner authorization and price consent before requesting a window;
these routes must never be exposed as an arbitrary-owner public billing proxy.
Personal owners need the existing `experimental:billing` rollout, active account,
provisioned wallet, good collection state and ordinary service entitlement.
The APIs do not provision any provider objects.

The effective subscription must be active, bound to the accepted customer and
configured plan, and denominated in USD. Readback verifies exactly one standard
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
`maximum_allocation`, the three exact decimal `credits_per_unit` strings and
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

The receipt returns `window_id`, all input identities, whole UTC `covered_from`
and exclusive `covered_until`, allocation, and each component's usage row ID and
reserved/finalized quantity. First coverage starts prospectively. Coverage is
at most 300 seconds and shortens at a UTC month boundary; use the returned
cutoff. A renewal names the immediate prior window, keeps its generation,
accepted quote and operation, and starts exactly at its cutoff. Renewal is
admitted only in the final 30 seconds before that cutoff. An expired generation
cannot be revived by a late renewal.

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

Catalog edits continue through existing authoring. A price mismatch blocks new
windows and provider emission for accepted usage. Before repricing an active
Sandbox service, stop new admission, drain evidence and emission, and verify
the provider billing period. No automatic migration rewrites accepted quotes.

The new collections contain lifecycle/evidence state only:

- `sandbox_billing_quotes`: accepted service-price and provider snapshots.
- `sandbox_billing_windows`: atomic request receipts and usage-row references.
- `sandbox_billing_cursors`: ordered, nonoverlapping coverage per producer/Sandbox.
- `sandbox_billing_evidence`: original retained revision quantities and digests.

Existing wallets, allowances, grants, rate cache, usage rows, exact settlement,
ledger and Lago emission remain authoritative for accounting. Runtime rows are
excluded from generic abandonment/release; absent evidence retains holds.

## Rollout and remaining launch work

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
NyxID. Neither this implementation nor the proposed rates authorizes activation.
