# CMA allocation billing through NyxID

Implementation scope selected on 2026-11-03: CMA usage must enter NyxID's usage
ledger and use prepaid controls before paid launch. Prices remain proposals in
[PR #2151](https://github.com/ChronoAIProject/NyxID/pull/2151); selecting this
architecture does not activate them.

User constraint: follow NyxID's existing pricing implementation. Do not create a
parallel price engine, administrative price screen, wallet or direct database
seed. Extend the current pricing normalization, exact Credits representation,
Lago synchronization/readback, rate cache and reservation/settlement machinery.
Any Sandbox lifecycle contract stores evidence and accepted pricing references;
it is not another source of prices. The existing Machine fixed-profile rate
contract is not copied as a new Sandbox pricing system. The three allocation
units use existing `LanePricing`/`LanePriceComponent`, normal price
normalization, service-owned Lago metric codes and the ordinary price sync/rate
cache. Runtime evidence supplies quantities; ordinary HTTP calls supply zero
allocation time. No startup seed or hard-coded production rate is introduced.

## Accounting and authority

NyxID owns versioned rates, account admission, benefit/wallet reservations,
settlement, the usage ledger and Lago emission. CMA owns Sandbox allocation
observations, the retained interval ledger, runtime lifecycle and the provider
deadline. Only one path may emit each billable interval to Lago. The existing CMA
direct exporter stays disabled when the NyxID path is selected.

Use three distinct allocation-time metrics, matching CMA's existing integer
units. Do not reuse `MachineRunningSeconds`, whose profile and paused-storage
contract differ. Reuse the exact funding machinery so Sandbox windows compete
with all other usage for the same wallet, grant and allowance balances.

A trusted CMA service principal may submit observations and request windows only
for an owner/Sandbox contract already bound to the accepted quote. A public
caller cannot choose a billable owner, price or settlement quantity. Changed
payloads under an existing idempotency key conflict. Rate versions and provider
subscription bindings are immutable once accepted.

## Finite windows and recovery

Each window reserves at most five minutes of resource allocation. Reserve all
three metrics and the durable window receipt in one majority Mongo transaction;
abort the entire reservation on any error. Persist returned funding coverage in
CMA before changing provider desired state. A retry after an ambiguous response
recovers the same receipt and holds. Never extend a runtime from an unconfirmed
reservation or a response for another allocation/generation.

Renew before the current cutoff. A failed renewal prevents further allocation
and requests a pause; native `shutdownTime` is also set to the funded cutoff.
Existing NyxID code explicitly treats this as desired Kubernetes control-plane
state, not a guaranteed physical stop under a node/control-plane partition.
Track any unreserved residual separately as a platform cost. Do not debit the
customer for work outside accepted funding coverage.

Paused storage reserves disk-only windows. Exhausted credit must not delete
customer data: stop compute admission and retain storage while billing is
suspended. Unfunded retained storage is a platform cost, not unbounded debt or a
future surprise backcharge. A separate retention/deletion policy would require
explicit product policy and customer disclosure before launch.

Settlement consumes only verified CMA allocation intervals intersecting a funded
window. Verify exact owner, Sandbox, runtime generation, allocation, integer
arithmetic, UTC boundaries and monotonic ledger evidence. Preserve partial
lifecycle markers and never synthesize missing intervals. Stop/release evidence
can settle less than the reserved capacity; unused holds are released through
the existing exact settlement path. Unknown outcomes keep their recoverable
liability until reconciled, and do not become automatically free usage.

## Rollout contract

The feature starts disabled. Old retained usage stays visible but is not charged;
the accepted contract has a prospective effective timestamp. All readers must
understand the new metric units before any writers create those rows. Validate
provider metrics and effective subscription prices, preserve unrelated Lago
charge IDs, and verify current bill and finalized invoice behavior on a controlled
test subscription before nonzero activation.

Required checks include concurrent reservations against one account, mixed
benefit/wallet funding, tax accounting, idempotent replay and mismatched replay,
expired/wrong-owner contracts, provider outages, renewal versus pause races,
crash recovery on both sides, active-to-paused disk accounting, release before
the cutoff, late residual usage, historical cutoff exclusion, retained history,
and exactly one Lago emission path.

## Existing implementation to reuse

- `backend/src/services/billing/funding/machine.rs`: finite reservations sharing
  wallet, grants, allowances and service ceilings; currently restricts the metric
  to machine running seconds and needs a carefully bounded generalization.
- `backend/src/services/billing/machine_runtime.rs`: immutable rate/owner/
  subscription binding, accepted quotes, durable window replay.
- `backend/src/services/billing/reservation.rs`: exact settlement and recovery.
- `backend/src/services/billing/reconcile.rs`: the sole NyxID Lago emitter.
- `backend/src/services/remote_machine_adapter.rs`: native shutdown-time mapping
  and its explicit partition limitation.
- CMA `layer3-adapter/src/agent_sandbox`: native claim/member authority checks;
  any funded-deadline mutation must preserve these existing identities and CAS
  guards instead of modifying arbitrary Kubernetes resources.
