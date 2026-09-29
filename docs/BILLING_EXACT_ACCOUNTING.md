# Exact credit accounting

NyxID 0.33.0 implements exact credit accounting for issue #1672.

## Invariants

All accounting uses credits with twelve decimal places. A billed settlement
satisfies `allowance_funded + grant_funded + wallet_funded = quantity × rate`.
Every money movement has balanced debit and credit postings. Wallet and grant
balances agree with their journal accounts. Compatibility display fields never
feed accounting.

## D1. Exact money and storage

`models::credits::Credits` holds checked i128 picocredits (10⁻¹² credits), bounded
to ±(10³⁴−1) picocredits so every accepted amount round-trips through Decimal128.
Arithmetic errors fail closed. Money never saturates, truncates or rounds inside
accounting. JSON uses canonical decimal strings; BSON uses Decimal128 credits.
Finite scientific notation and excess trailing zeroes are accepted; nonzero
sub-picocredit precision, NaN, infinity and overflow are rejected.
Canonical Credits use direct IEEE 754 BID conversion for BSON, with the same
bytes as the BSON library's decimal parser. Rollup serialization and model wire
adapters preserve binary BSON values instead of taking Extended JSON round trips.
Other incoming Decimal128 exponents retain the library fallback.

Wallet credit fields retain their existing unit-bearing whole-credit names.
Other exact money keys are unit-free: grant `amount`, `remaining`, `reserved`,
`terminal_amount`; funding `total_charge`, `allowance_funded`, `grant_funded`,
`wallet_funded`; allocation `amount`; rollup `gross_cost`, `wallet_cost`,
`grant_cost`, `allowance_cost`, `legacy_grant_cost`; expiry `amount` and
`expired_credits`; schedule and period `amount`. Rust exact money identifiers use
these names. When a new key is absent, readers normalize its legacy integer
`*_micros` key (or rollup `legacy_grant`) from microcredits. A present new key
wins, including when both keys exist. New writes use only unit-free money keys.
Legacy whole-credit wallet integers retain their original scale.

Rates use capped i64 picocredits and a truncated legacy micro-rate mirror.
An absent pico rate scales the legacy rate exactly. A Lago plan refresh excludes
only invalid metrics, removes their stale cache entries, logs them at error
level and publishes an Integrity diagnostic. Valid metrics continue refreshing;
an invalid metric cannot fall back to a free default.

## D2. Funding conservation and recovery

Reservation applies allowance quantities, computes the remaining exact cost,
reserves grants in expiry order, then derives the wallet hold by subtraction.
A fully grant-funded reservation passes with a prepaid wallet at zero.
Availability subtracts exact holds, pending Lago debits and pending expiry.

Settlement freezes its rate before funding mutations, applies actual allowance
quantities, consumes exact grant amounts and derives wallet funding by
subtraction. Wallet debit equals wallet-funded cost. A zero wallet share emits
no wallet posting or Lago quantity. Benefit operation locks retain their existing
idempotency and journal-confirmation protocol.

Rows with no wallet are metering-only: settlement charges zero and never reads
or consumes benefits. Legacy rows with a wallet but no funding snapshot retain
wallet-only semantics. Their durable `wallet_only` marker prevents retries from
claiming newly available benefits. Their wallet charge is exact quantity × rate.

Final funding amounts, `settled=true`, the Lago quantity, the diagnostic
`lago_carry_id`, and the carry cohort commit in one majority transaction, fenced
by the settlement claim. An aborted final row write rolls back the carry. A
retry either observes the entire committed split or recomputes both together.
A worker that outlives its claim returns the committed wallet share together
with its committed Lago quantity, never its stale computed share.

## D3. Balanced journal and account verification

New entries use `event_type: accounting_v2`, a `movement`, and ordered positive
postings with equal debit and credit totals. Transfers debit the destination and
credit the source. The asset accounts are debit-normal.

| Account                  | Meaning                                            |
| ------------------------ | -------------------------------------------------- |
| `wallet:{owner_id}`      | Provider balance minus pending Lago debits         |
| `grant:{grant_id}`       | Remaining promotional credits                      |
| `usage:{row_id}`         | Destination of grant and wallet-funded usage       |
| `platform:promotions`    | Source of grant issuance                           |
| `external:lago`          | Provider funding and explicit provider adjustments |
| `writeoff:grant_expired` | Expired grant credits                              |
| `writeoff:grant_revoked` | Revoked grant credits                              |
| `writeoff:topup_expired` | Expired purchased credits                          |
| `opening_balance`        | Cutover balances and absorbed legacy deltas only   |

Reservations do not move account balances. Nonzero cutover balances get opening
postings; zero balances get no entry. New wallets and any initial provider
funding against `external:lago` commit atomically. Checkout/invoice notifications
are memo-only, best-effort entries; provider refresh journals the actual money.

A usage wallet lock remains until its posting is durable. Grant issuance is
unspendable until journaled, consumption retains its lock until confirmation,
and terminal markers retry. Purchased-credit expiry retains its operation until
all entries are confirmed. Provider balance updates and postings commit together.
Dedupe keys, charge IDs and v1 canonical bytes remain unchanged.

### Canonical encoding

v1 bytes are unchanged. v2 HMAC-SHA256 input starts with
`nyxid:billing-ledger:v2\0`. Each following UTF-8 field has an unsigned u64
big-endian byte-length prefix, in this order:

1. Sequence, previous hash, entry ID, owner ID, reference ID, movement.
2. Transaction ID, layer suffix, metric, service slug, model, quantity.
3. Legacy amount credits, legacy amount micros, legacy balance credits.
4. Dedupe key, wallet ID, timestamp milliseconds, posting count.
5. For each posting in stored order: account, `debit`/`credit`, canonical amount.

Absent optional values encode as empty strings. Verification checks sequence,
linkage, hashes, balanced v2 postings and the latest audit-chain head anchor.
Tail truncation detection retains the unanchored interval window.

### Account checkpoints

`billing_account_balances` stores each reconciled asset account's signed
`balance` and `through_seq`. Every wallet or grant posting updates its
checkpoint in the same transaction as the ledger append, including openings
and provider adjustments. Journal-only usage, provider, promotion, opening,
and writeoff accounts have no checkpoint rows. This invariant lets
reconciliation compare stored balances with indexed checkpoint point reads,
independent of the asset account's history length. Checkpoints are
HMAC-authenticated, and the account/sequence index resolves the newest posting
to reject replayed checkpoints. Appends validate the preceding checkpoint
before advancing it.
The checkpoint HMAC input starts with `nyxid:billing-account:v1\0`, followed by
u64 big-endian length-prefixed UTF-8 account, sequence and canonical balance.
Rolling HMAC verification separately checks historical postings for edits or
deletion.

A billing-enabled runner leases the reconciliation job across replicas and checks
up to 1,000 wallets and 1,000 grants in one bounded snapshot. It runs at the
chain-verification cadence, holds the lease for the active pass, and releases it
when the pass reaches both lexical cursors; deployments with billing disabled
spawn no runner. Durable lexical cursors make collections
that finish early wait for the other collection. Active money locks and
incomplete journaling defer accounts; a deferred cycle never certifies a full
pass. Mismatches retain the cursor and report account, stored balance and journal
balance at error level. The one-second deferred-refresh worker uses a per-request
lease before calling Lago, so replicas cannot duplicate a provider fetch.

For N accounts in either collection, a pass runs its
`ceil(N/1000) + 1` bounded transactions back-to-back, then idles until the next
`CHAIN_VERIFY_INTERVAL_SECS` tick. With a five-second per-batch budget, 10,000
accounts take at most 55 seconds, 100,000 take 505 seconds, and 1,000,000 take
5,005 seconds (83 minutes 25 seconds), excluding database delays. The operational
full-pass objective is 24 hours, leaving retry headroom. Each transaction reads
at most 2,000 current accounts and their checkpoints; it never scans historical
posting lists. Recovery delays or database outages remain visible through the
last full-pass timestamp.

Append requests on a replica are coalesced into bounded group commits: one
tail read assigns consecutive sequences and hashes, one `insert_many` writes
the batch, and one transaction updates all affected asset checkpoints. Callers
wait for that transaction before releasing a usage lock; a crash therefore
leaves the durable lock for recovery. Across replicas, the unique sequence index
and transaction conflicts serialize the global chain. Per-key workers retry
sequence conflicts against a new tail with a thirty-second deadline instead of
eight attempts. Majority journaled commits authorize lock release.
Entries are validated before enqueue. If a group fails,
its members retry individually, isolating a corrupt account or overflow from
unrelated settlements. A failed send evicts only the matching closed worker and
retries once through a fresh worker.

Integrity preserves the existing `chains` array values (`audit_log` and
`billing_ledger`). Account results are an additive `accounts` field so cached
older clients still parse the response. Hash-chain breaks explicitly warn of
possible tampering; account mismatches have distinct recovery guidance.

## D4. Provider boundaries and residue

`billing_lago_carry` retains a cohort keyed by SHA-256 of the JSON tuple
(owner ID, metric code, canonical rate). It stores cumulative wallet money,
emitted microquantity as an integer string and a rational remainder numerator.
For cumulative wallet money M and rate R, both in picocredits:

```
E' = floor(M' × 10⁶ / R)
q = E' − E
remainder_numerator = (M' × 10⁶) mod R
```

The residue is `remainder_numerator / 10⁶` picocredits and may be finer than a pico.
It is never rounded into Credits. The row's `lago_carry_id` identifies this cohort
for diagnosis. Cohorts persist so later usage at the same rate can spend residue.
Only wallet-funded quantities go to Lago or drift comparison.

Provider JSON decimal tokens are preserved before floating-point conversion.
Webhook, reconciliation and expiry share one effective-balance calculation:
settled balance less exact accrued, uninvoiced usage. Invalid accrued usage fails
closed. A refresh blocked by a money lock creates a durable request and returns
`wallet_refresh_deferred`, never `ignored`. A one-second worker fetches a fresh
provider snapshot, commits it and deletes only the request token it serviced.
If a newer deferral changed that token, the worker releases only its own lease
so the newer request can run on the next tick; a successor's lease is preserved.

Purchased credits expire independently after 365 days. The Lago void boundary
floors at five decimal places. Residue remains spendable in the provider purchase
and is reconsidered from provider history on every sweep; there is no local
write-only expiry carry. Action limits apply after flooring so tiny residues
cannot starve actionable purchases. History increments and `history_applied`
commit together, fenced by the operation token. Provider receipts recognize
legacy history writes. Dedupe confirmation retains the expiry lock until every
transactional expiry posting is durable.

## D5. Cutover and operations

Existing installations must drain all pre-v2 billing writers before cutover:
disable billed admission, drain requests, stop old servers/reconcilers, and set
`BILLING_EXACT_CUTOVER_DRAINED=true` on the new version. The acknowledgement is
persisted so new replicas and restarts can resume it. Fresh databases need no
acknowledgement. Never restart old binaries against migrated data.

Cutover never blocks serving or panics the process, including when billing is
disabled. A background task waits for acknowledgement and migrates bounded batches.
Billing cutover and analytics normalization have independent durable markers:
`billing_migrations/exact-v2.completed_at` admits exact billing, while
`billing_migrations/exact-v2-rollups.completed_at` certifies that derived hourly
and daily mirrors have been normalized. Until the billing marker exists, billed
admission and money mutations/sweeps return `BillingProviderUnavailable` (503).
They never wait for the analytics marker. Auth, SSO, nonbilling proxy traffic,
metering-only settlement and existing wallet/grant UI reads continue. Every
replica reads the billing marker; completion requires no restart. The background
migration starts rollup normalization after billing is ready and records its own
bounded progress, state and Integrity diagnostic; a delayed rollup marker leaves
analytics on the legacy-tolerant reduction path.

Integrity reports `billing_exact_cutover` while the billing marker is pending,
including the acknowledgement requirement or failed money-document identifiers.
It independently reports `billing_rollup_normalization` while the rollup marker
is pending, with the current operation or last normalization error. The rollup
marker records `running`, `error`, or `complete`, plus separate lexical
`rollup_hourly_after` and `rollup_daily_after` cursors. A successful completion
clears that marker's diagnostic; a rollup error never reopens the billing gate.

Each document conversion, absorbed-operation marker and nonzero opening posting
commits atomically. Malformed documents are isolated in `billing_migration_errors`,
logged and surfaced through the Integrity cutover diagnostic; both conversion
and delta absorption continue with other rows in bounded lexical batches.
Empty, lock-free historical grants with valid integer amounts
convert by bulk pipeline with no journal append. Failed rows retry in subsequent
background passes. No normal reconcile tick or request performs migration or
legacy-delta absorption after completion.

Grant balances and locks remove old required micro keys. Already-applied legacy
wallet/expiry operations are marked absorbed so recovery cannot post them twice.
Unapplied legacy grant locks apply once in the opening transaction. Cutover absorbs
any remaining legacy deltas before marking completion. Old wallet/grant readers
and new ledger variants fail old deserializers; old verification cannot certify
new history. Historical usage stays readable through explicit scale fallbacks.

Rollup normalization processes historical hourly and daily roots, nested
partitions and query mirrors in bounded resumable batches after billing cutover.
Each batch is a MongoDB server-side update pipeline, so each document is changed
atomically with concurrent fold updates; it preserves an explicitly present exact
value (including null), converts an absent legacy microcredit key to Decimal128
credits, and removes the legacy mirror keys. Progress is durable and failed
batches remain diagnostic for the next pass. Before
`exact-v2-rollups.completed_at`, admin usage and analytics use a legacy-tolerant
expression that handles mixed documents. After that marker, readers use the
plain exact paths and covering indexes.
Old `*_micros` query-mirror values are Decimal128 microcredits; unit-free mirror
values are Decimal128 credits. A missing mirror falls back to the root without
rescaling an exact root. New folds mirror all five money measures, including
legacy grant cost. Normalization and every fold also write flat
`query_{money_field}` Decimal128 aliases from those mirrors in the same atomic
update. Once every exact key exists, scalar v4 covering definitions need no
presence inspection or nested traversal. Before completion, document reads
preserve the missing-versus-null distinction. Startup creates replacement indexes
before retiring their known obsolete definitions; unrelated indexes remain
unchanged.

## D6. API and clients

Exact API amounts are additive Credits fields encoded as decimal strings. Wallets
expose balance, reserved, pending debits/expiry, available, available with overdraft
and overdraft cap. Usage exposes estimated, wallet, grant and allowance credits.
Grants/schedules expose exact amounts; expiry exposes exact expired credits.
Admin usage exposes exact costs, and analytics exposes `exact_value`/`exact_total`.
Legacy integer response fields truncate once after aggregation and clamp only for
display range. Whole-credit authoring inputs remain compatible.

Frontend calculation/sorting uses BigInt picocredits. Missing exact fields fall
back to safe legacy integers; null stays unknown. Normal display rounds half away
from zero to six decimals and shows a threshold for smaller nonzero values.
Compact headlines compute their displayed digits with integer arithmetic. Chart
geometry may use Number projections; money labels retain exact decimals. CLI
output prefers exact amounts, including top-up verification arithmetic.

## Deviations

- Decimal128 cannot represent the full i128 picocredit range; the accepted range
  is constrained to exact Decimal128 round trips rather than silently rounding.
- A drained first cutover is required because old workers can write through
  previously loaded raw documents. Deserialization fences alone cannot guarantee
  mixed-version safety. Pending cutover gates billing rather than server startup.
- Legacy funding-less rows retain wallet-only semantics; applying new benefits
  retroactively would consume grants/allowances that were never reserved.
- Account checkpoints are maintained transactionally with appends rather than
  reconstructed by repeatedly scanning historical tails. This keeps account
  verification bounded even for the busiest wallet; the independent hash-chain
  verifier retains historical tamper detection.
- Superseded rollup covering indexes are retired after their replacements exist.
  Keeping both sets doubled transaction dirty-page pressure and repeatedly
  exhausted the benchmark replica’s 256 MiB WiredTiger cache. The replacement
  indexes cover normalized exact mirrors; the transitional legacy fallback
  fetches documents until its separate completion marker. This changes only
  known obsolete definitions.
- Covered reductions use normalized exact money paths only after the rollup
  marker. During the bounded migration window, the marker-aware reduction keeps
  missing and explicit-null semantics correct across mixed legacy/exact mirrors.
- Checkpoints are limited to wallet and grant asset accounts. Journal-only usage,
  provider, promotion, opening and writeoff accounts do not create durable
  checkpoint rows; the rolling verifier owns the historical posting checks.
- Billing and rollup readiness are separate markers. Wallet/grant cutover gates
  money immediately after its own durable marker; analytics normalization is a
  bounded background job and never creates a billing outage. A process-local
  latch avoids repeating each marker query after its durable completion is seen.
- Expiry residue is retained by the provider purchase, whose current remainder
  is authoritative; a duplicate local write-only carry is unnecessary.
- Normalized reductions use flat `query_{money_field}` Decimal128 aliases instead
  of dotted `query_costs` paths. MongoDB 8.0 reconstructs and traverses a nested
  object for each covered dotted-path index row; flat fields feed the group
  directly from index slots. Both mirror forms retain identical exact amounts,
  including explicit null, and commit atomically with each fold/normalization.
  Replacement v4 indexes retire only matching old definitions after creation.
- Rollup normalization uses an explicit presence check instead of `$ifNull` for
  exact fields, because a present null must continue to supersede legacy values.

## Test coverage

`Credits` tests cover exact parsing, JSON/BSON representations, arithmetic bounds,
legacy scale fallback and randomized conservation. `exact_tests` covers the
zero-wallet grant regression, fractional mixed funding, legacy/metering-only
semantics, aborted carry commit recovery with a new grant, migration isolation,
zero openings, legacy locks, mixed-version fences, concurrent carry and journal
idempotency, 200 simultaneous wallet settlements, bounded account checks and
balance corruption. Provider tests cover mixed valid/invalid rates, deferred
top-up refresh, consistent accrued usage, expiry residue and history idempotency.
Rollup/admin tests cover mixed legacy/exact buckets before and after the rollup
marker, atomic normalization interleaved with a fold increment, exact numeric
cost ranking, covered sums against a Decimal128 oracle across sub-micro and
Int64 overflow cases, and the 472-microcredit loss regression. Ledger tests cover batch
failure isolation and replacement of a closed append worker. Frontend and CLI
tests cover additive
schemas, exact formatting, compact headlines and arithmetic; billing browser
specs cover responsive meters and analytics.

Named regressions include:

- `metering_only_and_legacy_wallet_rows_never_consume_benefits`.
- `aborted_carry_and_split_retry_with_new_grant_conserve_rational_residue`.
- `pending_cutover_gates_money_but_allows_metering_and_legacy_reads`.
- `deferred_topup_refresh_uses_effective_balance_without_flip_flop`.
- `deferred_refresh_services_newer_request_on_next_pass`.
- `deferred_refresh_preserves_successor_lease`.
- `two_hundred_concurrent_wallet_settlements_leave_no_locks`.
- `account_check_uses_constant_work_after_many_postings`.
- `covered_credit_mirrors_preserve_legacy_scale_and_explicit_null`.
- `benchmark_daily_fixture_scales_decimal_money_and_quantities`.
- `pico_costs_retain_472_micros_across_raw_rollup_api_and_analytics`.
- `covered_money_reduction_matches_decimal_or_fails_closed_on_overflow`.

## Performance

After normalization, the covered reduction reads flat Decimal128 money mirrors
from index slots and aggregates exact credits before truncating compatibility
displays. Before integration with main, the density benchmark compared base
0.30.2 against the exact-accounting implementation retained in 0.33.0, using
sequential runs with the same fixture timestamp and document/key counts. The
table reports the unfiltered medians from those completed runs; these are not
measurements of the integrated release.

| Window  | Base median | Exact accounting median | Median budget |
| ------- | ----------- | ----------------------- | ------------- |
| 1 day   | 354.6 ms    | 403.2 ms                | 500 ms        |
| 7 days  | 465.9 ms    | 522.1 ms                | 1,000 ms      |
| 31 days | 671.0 ms    | 791.7 ms                | 2,000 ms      |

All window/filter cases passed the exact-oracle, index-coverage and median-budget
assertions. The remaining overhead relative to the integer base is reflected in
these measurements.
