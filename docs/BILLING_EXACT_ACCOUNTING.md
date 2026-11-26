# Exact credit accounting

NyxID 0.34.0 implements exact credit accounting for issue #1672.

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
An explicit final quantity of zero returns a legacy wallet hold even if its
historical rate is unavailable. Unreleased quantities that exhaust settlement
retries are retried in a slow lane (at most 25 rows and five seconds per
reconcile pass, one hour between attempts); the final quantity is preserved.
Provider-delivery dead letters whose money settlement has completed are separate.

Ordinary forwarding records the API process generation in its existing write.
The registry records database-time registration before serving and retains that
time across heartbeats. Integrity classifies forwarded unfinished requests by
ownership, final evidence and recovery protocol, with bounded row/request counts
and held amounts; this diagnostic never changes usage. Missing registry evidence
permits no death inference. Old live generations without registration times block
legacy inference. No healthy stream gains a lifetime limit.

The classification starts from potential wallet, grant and allowance holds,
using three partial indexes that exclude zero reservations and settled benefit
funding. It reads 128 candidates per source in rotation, at most 10,000 distinct
rows within four seconds. Receipts and applied resource locks are deducted in a
single pinned snapshot. Completed groups survive truncation as lower bounds,
with a stable reason, stage, elapsed time and database code when available.
Zero-hold forwarding history cannot consume the candidate limit. A separate
index-only count examines at most 100,001 entries in each of the four indexes;
subtracting the sum of potential-hold counts from the capped forwarded count
provides a conservative hold-less lower bound. Overlapping funding and returned
holds can undercount it. No payloads are read for that count.


Ordinary unknown forwarded work with a positive reservation is ended only after its owning process generation
is absent beyond the registry window and margin. Legacy work additionally predates
every live registration by an hour. Final quantities, final checkpoints and durable
intents recover first; tagged pool/Realtime, managed/certified execution, runtime
retain their own exits. A terminal transaction competes with the real
finalizer and records a chained metadata audit; ordinary settlement then returns
unused holds. `stuck_requests::policy::unknown_consumption_policy` is the single
charging choice: the default final zero waives genuinely unknown consumption.
The owner chose this rule on 2026-11-15: settle recorded final usage, charge nothing
for genuinely unknown consumption, and release the unused hold.
Stranded funded voice requests share the held-request batch and process-death
proof, including a registry-live voice takeover. Recovery first uses voice's
recorded response sum or observed seconds (with its existing initial-request
rule); only missing evidence uses the shared unknown policy. The existing window
sweep still settles sealed windows or windows past their 24-hour evidence deadline,
including metering-only windows and rows without ownership proof: each window is
30 seconds of a call capped at 1,800 seconds. Both paths use the window's own
recorded seconds when it exists. The shared path reads and decides before fencing
session/window and meter rows in one transaction; a late finalizer respects the
winner; a sealed window retains its evidence metadata. Failed windows do not
abort the billing pass. Their additive `reconcile_failed_at` database timestamp
orders them behind untried windows, then oldest failure first, in the existing
100-window batch. Only failures add a retry-marker write. Old replicas ignore
this marker and the window's additive `recovery_fence`, retaining their previous
retry order and respecting the existing seal/meter winner. One error summary
per window pass records the failed count; successful windows still count toward
recovered settlements. Integrity reports voice identity kind and deferred
reason, using the same evidence predicate as publication; non-voice pages perform
no voice reads. Shared held-request recovery logs at most one summary per pass:
expected waits (live sessions, pending evidence, registry unavailability, time
budget or publication conflict) at debug, malformed evidence or actual read,
publication or settlement failures at warn.

The tested but unused reserved-estimate alternative uses the recorded reservation
quantity, or an exactly recoverable old reservation, and leaves ambiguous old
estimates unresolved. A paused owner
that resumes after losing the terminal race cannot charge again; its unrecorded
consumption may be waived. Healthy streams retain ownership without a new lifetime
limit. Old replicas ignore the additive ownership/estimate fields and respect the
existing terminal status; rollback needs no data conversion or operator step.

The sweep rotates wallet, grant and allowance candidate indexes, at most 100 rows
and 50 requests in five seconds per pass, with a durable cursor per source.
Once a held request is selected, its existing terminal transaction closes all
unfinished siblings, including zero-hold coordinators, so a resumed finalizer
cannot publish new usage through them. A separate housekeeping lane follows the
money lane, reading at most 100 candidates from the existing forwarded/unreleased
index within its own one-second budget and retaining a durable cursor. It uses
the same process-death proof, excludes dedicated execution protocols, and only
ends whole requests (at most 128 siblings) with no remaining holds or recorded
final evidence. Its terminal transaction records zero usage and
`stuck_request_exit.policy=waive_unknown_consumption`, `lane=holdless`, and a
summary pass ID; it changes no wallet, benefit, tax, ledger or carry resource.
The marked row's release write also acknowledges the zero and sets a 30-day
expiry, touching only usage metadata; shared usage push and acknowledgement are
unchanged and never select it. Rollups count a zero-usage request on its original
day and survive source-row expiry; raw billing history no longer includes expired
rows. Each nonempty pass writes
one chained `billing_holdless_requests_ended` summary with layer/metric counts
and age bounds, without person IDs; interrupted summaries resume durably.
Integrity retains separate `holdless_ended_requests` and `holdless_ended_rows`
counters. Completely finished rows are already terminal, released and
acknowledged, so older replicas select no settlement or push work for them.
No rollout step or environment setting is required.

Ordinary metering-only rows (free services and the person's own key) receive
`expires_at = counted_at + 30 days` in the same write that marks their rollup
count complete. Already-dated rows keep their date; wallet-backed rows retain
acknowledgement-owned retention. Uncounted rows receive no date from this policy.
The existing billing pass walks 100 IDs at a time through the existing `_id_`
index, retaining a durable cursor in `usage_rollup_state`. It keeps taking
pages, dating only finalized (or forwarded dead-letter), counted metering-only
rows without a date, until roughly one second has elapsed or the pass is
drained. The cursor advances durably after every page. A complete sweep records
its time and sleeps until the next daily due time; a dormant pass does one state
read and no write. The cursor wraps to pick up rows counted by older replicas
during a rolling upgrade. No new index, setting or operator step is required.
Failed/abandoned rows without quantities and unacknowledged wallet-backed rows
keep their existing retention behavior.

`GET /api/v1/billing/usage` keeps the exact raw aggregate for `24h`, `7d`, and
`30d`. Longer periods align their start to a UTC midnight and use permanent
daily rollups before the retained raw window. The additive `covers_from` field
states the earliest source time included. Rows from the permanent part set
`detail_retained` to false because operation, machine, key, and acknowledgement
detail is not stored there; the endpoint returns permanent rows separately from
newer raw rows so consumers can sum service totals without inventing request
detail. Allowance units are reported only on retained raw rows. `machine_id` and
`job_id` filters remain raw queries and expose the retention boundary in
`covers_from`.

Final funding amounts, `settled=true`, the Lago quantity, the diagnostic
`lago_carry_id`, and the carry cohort commit in one majority transaction, fenced
by the settlement claim. An aborted final row write rolls back the carry. A
retry either observes the entire committed split or recomputes both together.
A worker that outlives its claim returns the committed wallet share together
with its committed Lago quantity, never its stale computed share.

### Allocation-local service ceilings and reclaim

An allocation remains one `CreditGrant` and one journal account. Optional disjoint `service_limits` contain server-resolved catalog IDs, compatibility slugs, a stable key, and exact `limit`, `consumed`, and `reserved` Decimal128 credit counters. A shared total of 10,000 with Twitter up to 500 and OpenAI+Anthropic up to 300 does not earmark 800 credits: unspecified eligible services may use the shared total. Other allocations and purchased credits fund excess independently. Canonical IDs take precedence over slugs; a historical user-connection ID is not treated as a catalog identity. Missing policy fields mean uncapped, not pending reclaim.

Ordinary and durable public/pool/managed admission commit total/rule holds, allowance/wallet holds and all meter rows in the same transaction. Machine windows use their existing caller-owned lifecycle transaction. Reservation excludes an active resource operation; relevant operations are helped before admission, and contention fails closed rather than silently changing funding order. Existing frozen rates, public/managed dispatch gates, pool leases, machine finite/no-overdraft/evidence/reserved-only settlement and carry transactions remain authoritative.

`revoke_requested_at` immediately fences new holds and unreserved extra usage. Existing holds and acquired durable locks finish. Finalization requires zero reservations and no resource lock, atomically captures exact remaining value as `terminal_amount`, and confirms the existing balanced `grant-revoked:{id}` journal movement once. Release, settlement and reconciliation retry it. Reclaim never debits another allocation or wallet. Expiry uses its own terminal movement and remains distinct from reclaim. Effective `held` includes unapplied extra lock commitments; raw `reserved` fields remain unchanged, including per-rule responses.

Resource lock acquisition checks durable operation history and writes the usage row under its immutable invocation claim in the same transaction. Takeover drains locks for the usage, including exhausted resources, before recomputing coverage. Allowance and grant coverage comes from recorded histories, so a competing consumer reducing an extra operation cannot reduce wallet funding incorrectly. New admissions cannot create a hold without recoverable usage intent. Never-forwarded cancellation and abandonment commit the terminal meter transition and exact tax-inclusive wallet hold release in one transaction; benefit release then resumes through its existing operation locks and history. Old binaries retain the separate-write release gap during rollout or rollback. The account runner also compares reservation counters with live meter holds; it returns a persistent, unlocked wallet excess after two unchanged scheduled observations as described below.

Optional actor-scoped UUID submission keys freeze exact payloads and recipient/policy snapshots. Deterministic per-recipient IDs resume partial/concurrent retries without resetting spent, held or reclaimed state; a changed payload conflicts. Schedule periods preserve their existing deterministic identities, recipient cutoffs and frozen policies; every issuance gets fresh counters. Sparse pause/note edits do not re-resolve unchanged rules after catalog retirement.

### Recipient credit schedules

Schedules may specify immutable `timing: recipient` instead of the legacy default `calendar`. Calendar schedule periods and their identities are preserved. Recipient schedules continuously discover new eligible owners and keep one durable `credit_schedule_recipients` row per schedule/owner, with an immutable UTC `anchor_at` and indexed `next_due_at`. Recipient reads and writes use primary/majority concerns so anchors survive failover before grants use them. These anchors are financial identity metadata and must be retained alongside their grants and journal history; only `credit_schedule_recipient_scans` is disposable progress. Leaving and rejoining does not reset an anchor. Discovery pages are bounded by the existing disbursement budget; account creation, successful human sign-in and organization/group joins can enroll and issue directly, while discovery repairs missed enrollment writes. Eligibility is checked live before each issuance. Ineligible rows stop being due until discovery, sign-in or a qualifying join restores eligibility. Organization and group joins advance an eligibility version even when the row still appears eligible; workers compare the version before marking a row ineligible, preserving concurrent rejoins. Joining while paused records the original anchor but issues nothing until resume. Discovery anchors use the page observation time. Older all-users and org members, selected users, and missed hooks may take a complete discovery cursor pass (population divided by available per-tick budget); a tick with exhausted issuance budget defers discovery. The signup, sign-in and join hooks avoid that cursor delay for the person they process.

Personal-account export includes only the requesting person's cycle metadata under `billing.credit_schedule_recipients`, using a recipient-leading index. Account erasure retains anchors as financial evidence, as it retains grants; deleted accounts fail live eligibility. The schedule-owned discovery cursor is transient coordination, contains only an opaque user ordering boundary, advances or resets on each pass, and is excluded from personal export.

Daily/weekly windows are durations from the anchor. Monthly windows are calculated from the original anchor, clamping short months without drifting the original day. Each issuance retains UUID v5 `{schedule_id}:{period_start_ms}:{recipient_user_id}` and ledger dedupe `grant-issued:{grant_id}`. Concurrent/retried inserts retain the original persisted amount and policy. Journal recovery reads the persisted grant rather than a newly computed candidate. `next_due_at` advances monotonically after the grant is durable; failed posting is recovered through the existing unledgered-grant sweep. Pending grants remain unspendable. A delayed or resumed schedule issues the current window only. Expiry is resolved against the individual window; existing grants keep their rules after edits.

New recipient authoring requires both `BILLING_ALLOCATIONS_ENABLED` and fleet readiness for `billing:recipient-schedules-v1`; the admin allocation overview advertises `recipient_schedules` only when both pass. The readiness gate requires durable capability activation and live serving support from every observed generation. Roster and drain declarations are advisory under the shared readiness protocol; they do not prove invisible old workers have stopped. Operators must still upgrade and drain every API replica and billing worker before authoring recipient schedules. Allocations being enabled alone cannot open recipient authoring during a mixed rollout. Upgrade **all** API replicas and billing workers before creating recipient schedules; old workers ignore the additive timing field and cannot safely process these rows. Before rolling back to old workers, pause **every** recipient schedule, stop and drain all upgraded billing workers and in-flight disbursements, then verify no active recipient schedule remains. Keep these schedules paused on old binaries. Resuming a paused recipient schedule is gated like authoring (allocations enabled plus this readiness), so an upgraded replica refuses it while any observed generation lacks support; an old replica cannot enforce that gate. Disabling authoring alone is not a rollback. Omitted timing preserves calendar behavior, and PATCH rejects timing changes. No existing schedule or grant is converted automatically.

Enable `BILLING_ALLOCATIONS_ENABLED` only after every API replica and billing worker is upgraded and old writers drained. Dedicated authoring routes prevent old servers silently ignoring policy fields; the flag does not attest worker versions. Disabling authoring cannot make already-persisted policies safe for old binaries. No migration invents allocation balances.

### Seeding missed calendar-schedule recipients

Recipient schedules already enroll existing eligible owners automatically, regardless of signup age. For legacy calendar schedules, admins can explicitly catch up currently eligible owners through `POST /api/v1/admin/credits/schedules/{schedule_id}/seed`. Ordinary calendar disbursement keeps its frozen signup/join cutoff. Seeding uses the current calendar period's frozen recipient policy, exact amount, limits and expiry with live eligibility, including owners behind the completed walk's cursor. No historical periods are backfilled and no one-time grant is converted.

Each request examines at most 50 owners and returns `period_start`, `next_user_id`, `created_count`, `existing_count` and `pending_activation_count`. Subsequent requests supply that `period_start` and `after_user_id`; an elapsed period is rejected rather than silently starting a new one. Optional `expected_updated_at` rejects stale authoring snapshots; the admin UI sends it on every page. Paused schedules and expired period benefits are rejected. The overview advertises optional `schedule_seeding`; older servers never display the action.

Seeding uses the ordinary deterministic grant identity and `grant-issued:{id}` dedupe. Any existing current-period grant counts as received, including consumed, expired, revoked and pending grants. Retrying or racing the billing worker never refills that identity. Journal recovery uses the persisted grant amount, and unjournaled money remains unspendable until the existing recovery sweep succeeds. Partial success is durable; retrying the open dialog resumes from the last successful page. Reopening the dialog starts a safe scan from the beginning. Each page durably audits the acting admin and intended grant IDs before insertion, so partial failures remain attributable; new grants record that admin as `granted_by`. Active organization membership is required for org-targeted catch-up. Period counts are derived from durable grants and updated monotonically. This action writes no new schedule policy that a legacy worker could reinterpret.

### Scheduled credits at signup, sign-in and membership changes

Post-commit credit loading works on one active person and that person's live memberships.

- Account creation covers email registration with or without an invite, social signup and admin creation. Completed human sign-in covers password and MFA in browser and token clients, social web and mobile callbacks, native social token exchange, the first-party CLI browser handoff, one-time login code and device account delivery, and approval flows that establish a browser session. Signup and sign-in match all-users, selected-user, current active-organization memberships and direct group targets. Refresh, pending or failed proof, and Agent Key-only delivery are outside this hook.
- Organization creation processes the founder after the organization and membership commit. Admin additions, invite acceptance and rejoins match schedules targeting that organization. Direct group additions match that group. The account and the event's membership must still qualify when read.
- A committed organization exit has no billing side effects. It preserves existing grants and anchors. Future issuance checks live eligibility, and a rejoin retains the original recipient anchor.

An active recipient schedule enrolls the person and issues a due current-cycle grant inline, even before the discovery worker reaches them. A signup or join records the eligibility time, bounded by schedule creation. Sign-in discovers missing anchors at observation time. Joining while paused records the anchor for use on resume. Already-enrolled recipients keep their anchor when matched again.

Calendar periods may be opened only by the ordinary worker or admin seeding. Signup, sign-in and joins can issue from an active schedule's already-open current period, including a completed walk. They match the period's frozen targets against live eligibility and use the frozen amount, limits and expiry despite subsequent edits. Before a period opens, its ordinary walk handles issuance. The login query batches matching period, schedule and grant reads and avoids a recipient walk or count on an already-funded sign-in.

Both timings use ordinary deterministic grant and ledger identities. Credits are spendable after their issuance posting is durable. Existing spent, reclaimed or expired grants are never refilled. Progress publications use monotonic counts derived from durable grant identities so an overlapping walk, seed or lifecycle insert cannot count a grant twice. The foreground check waits up to three seconds after the account, sign-in or membership commits. If it times out, the client can still receive its credentials and durable partial work remains recoverable. Another eligible sign-in, worker discovery and journal recovery, or **Grant missing credits** for calendar schedules can recover deferred work.

There is no in-place calendar-to-recipient conversion. Creating a replacement schedule creates a separate financial identity and can grant another full allocation to members who already received calendar credits, even if the old schedule is paused. Do not use pause-and-recreate as a duplicate-free migration. Keep the existing schedule and use current-period seeding until an administrator has explicitly chosen a transition policy for existing recipients. Calendar joins continue to use the current frozen period rather than starting an independent cycle.

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

### Reporting grant flows

The admin Grant credits report reads immutable issuance and consumption in the
same bounded UTC window. Exact v2 `grant_issued` takes the credit posting to
`platform:promotions`; `grant_consumed` takes the credit posting to
`grant:{reference_id}`. Summing both balanced legs doubles the movement. Legacy
events use micros when present, otherwise whole credits. Period consumption can
exceed period issuance because older grants remain spendable. Financial owner
IDs separate person and organization accounts; service slugs exist on consumption
and never imply a service allocation of shared issuance. Full totals, daily
buckets and independently paginated account/service breakdowns come from one
snapshot aggregation. See [billing analytics](BILLING_ANALYTICS.md#grant-credits-granted-and-used)
for the response, filters and bounds. These reads do not change funding order,
settlement, deduplication or ledger writes.

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

A billing-enabled account-integrity runner leases its reconciliation job across replicas and checks
up to 1,000 wallets, 1,000 grants and 1,000 allowance periods in one bounded snapshot. It runs at the
chain-verification cadence, holds the lease for the active pass, and releases it
when the pass reaches all lexical cursors. Deployments with billing disabled or
`CHAIN_VERIFY_INTERVAL_SECS=0` spawn no account runner; zero also disables
automatic hash-chain verification. Durable lexical cursors make collections
that finish early wait for the other collections. Active money locks and
incomplete journaling defer accounts; a deferred cycle never certifies a full
pass. Mismatches retain the cursor and report account, stored balance and journal
balance at error level. The one-second deferred-refresh worker uses a per-request
lease before calling Lago, so replicas cannot duplicate a provider fetch.

The same snapshot pass verifies wallet holds against every unreleased meter row
(including forwarded and finalized rows), subtracting the one applied wallet
settlement whose row is still unreleased. Grant and allowance holds use original
reservation allocations less recorded settlement/release operations; an applied
resource lock accounts for the gap before its history record. Grant rule holds
are checked too. These counters are separate from journaled asset checkpoints.

Integrity also exposes a read-only `billing_hold_backlog` inventory of unfinished
wallet, grant and allowance rows, grouped by meter status and creation age
(under one hour, 1–24 hours, 1–7 days, over seven days). It shows row counts,
outstanding holds and distinct resources, wallets and owners; allowance units
stay separated by metric. Each kind uses a read-only snapshot transaction for
meter rows, cursor batches and joined resource locks, excluding recorded returns
and matching applied locks. Aborting the transaction at the end releases its
pinned history.
Each kind scans at most 10,000 rows using the existing partial indexes, with a
three-second server and four-second overall client bound. Expired or unavailable
snapshots retry from scratch up to eight times within that same client bound;
partial attempts are discarded. Truncation, malformed rows or exhausted reads
make that kind explicitly incomplete; unavailable reads show no totals and are
retried on the next page read. Failure details include a stable reason, stage,
elapsed milliseconds and database error code when available, never raw queries
or driver messages.
These are amounts still held, not proof of abandonment or permission to charge
or release them. Conservation diagnostics separately count affected resources
and wallets, including resources without a wallet. Reading these diagnostics changes no counters and adds no work to billed
requests; the scheduled account pass separately repairs proved excess below.

`billing_hold_conservation` reports unchanged mismatches seen in consecutive
scheduled passes and at least five minutes of stability measured with MongoDB
time. It separates wallet excess, wallet deficit, grant and allowance counts,
with at most five IDs and signed total differences per kind. Resources without
a wallet use their resource key. Every affected resource is counted; wallets
are deduplicated per kind, and the sample shows one resource's difference per
wallet, not a sum of units.
Rapid passes retain the first unchanged timestamp. A revisit within the same
pass preserves confirmation; hold repair waits while an asset mismatch holds
the cursor. A balance change does not hide an unchanged hold mismatch but does
restart repair confirmation.

For unchanged financial/identity/policy/lock state, positive wallet, grant total,
grant rule and allowance-period excesses are returned after two consecutive distinct
passes confirm the same state and its first unchanged observation is at least five
minutes old in database time. A small snapshot/majority
transaction rechecks the runner lease, observation, resource and live holds, writes
one chained `billing_wallet_hold_excess_repaired`, `billing_grant_hold_excess_repaired`
or `billing_allowance_hold_excess_repaired` audit event, and deletes the observation.
Benefit audit metadata identifies the resource, optional wallet, per-counter amounts
and both passes. Any deficit component, incomplete data, missing owner/allocation,
unexplained receipt or active lock excludes that entire benefit resource. Remaining,
consumed and asset checkpoints do not change; an expired grant can then finish in
the existing expiry sweep. Benefit confirmation includes the canonical resource,
allocation and receipt images; changes restart its database-time age.

Wallet refresh/expiry clocks and provider/tax generations remain excluded from wallet
confirmation because they can change without funds moving. Zero wallet shares stay
excluded from its live hold fingerprint. A clean observation also removes its
diagnostic; a completed pass removes observations for resources no longer visited.
Manual Integrity checks keep separate cursors and do not confirm or repair holds.
Benefit repair assumes the confirmed production fleet contains no pre-20-October
binary that reserved benefits outside a transaction; all running builds are from
this week, so ordinary rollout or rollback among those builds needs no operator step.

Checks add no commands to admission, settlement or release. Each nonempty
eligible page reads observations once and live-row presence once; idle resources
need no individual read or full model decode. Only live rows, nonzero total/rule
counters, locks or prior observations require detail. The wallet presence query
is covered by a partial index on unreleased rows; benefit indexes include only
rows containing the relevant reservation ID, including legacy funding without
`settled`. Startup builds each index over existing data; old binaries neither
depend on nor drop them. Latest journal-posting seeks are batched per snapshot.
Each detailed query returns at most 10,001 matching rows. More than 10,000,
malformed amounts/models or unprovable lock context produce a separate incomplete
hold diagnostic, never repair. They do not change the asset certificate or its
status detail; unreadable asset fields still defer that asset. Pre-cutover
wallets need no hold query and remain deferred by the exact-accounting gate.

An ordinary rolling update needs no action. An old process frozen between its
two release writes past two account observations and at least five minutes can,
if it then resumes after repair, subtract another live hold by at most its own
release amount (the nonnegative-counter predicate still applies). That deficit
is reported and never auto-changed. Rolling back restores the old release gap;
no persisted value schema changes or rollback migration are required. The
account cadence remains `CHAIN_VERIFY_INTERVAL_SECS` (default 3,600 seconds),
not the separate 300-second billing-maintenance interval.

The separate periodic billing maintenance pass (settlement recovery, grant and
purchase expiry, Lago price/usage/balance refresh) uses the renewable
`billing:reconcile` cluster lease. It skips contending ticks and, on renewal loss,
finishes the current write protocol before stopping at a row/component/chunk
boundary. Financial row claims, locks and dedupe are unchanged; request-time
writers do not acquire this supervisor lease. See
[BILLING_OPERATIONS.md](BILLING_OPERATIONS.md#reconcile-ownership-and-verification-status)
for timing, drain and mixed-version limits. Automatic and manual chain verification discard losing `Ok` scans, always log a local
`Broken` at error level and retry its publication up to three times (on conflict,
keeping the stored break at an earlier or equal sequence, or returning an error on
exhaustion), so a late break after repair may restore an alarm that the next scan
clears by rechecking its break cursor.

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

Wallet `snapshot_generation` orders provider balance reads against local publication
and invalidation. It advances on publication, a balance webhook before its provider
read, and the first wallet-backed usage acknowledgement. A rejected read changes
no balance or ledger entry: webhook/reconcile callers request a fresh read, the
deferred worker retains its existing request, and expiry retries from its stored
operation/receipt without creating a refresh request. Old binaries ignore the
field; their balance writes and acknowledgements are not fenced. Ordering is
complete only once every replica runs this release; rollback needs no data change.

Purchased credits expire independently after 365 days. The Lago void boundary
floors at five decimal places. Residue remains spendable in the provider purchase
and is reconsidered from provider history on every sweep; there is no local
write-only expiry carry. Action limits apply after flooring so tiny residues
cannot starve actionable purchases. History increments and `history_applied`
commit together, fenced by the operation token. Provider receipts recognize
legacy history writes. Dedupe confirmation retains the expiry lock until every
transactional expiry posting is durable.

Expiry processing claims use MongoDB time with a 600-second lease renewed every
60 seconds and released immediately on return. Before each void POST, the exact
live operation/token must durably set `dispatch_started_at`. Definite provider
refusals clear that marker for a later pass; uncertain responses retain the hold
and appear in Integrity. Every later pass reads history first. After 24 hours
without a matching receipt, one timestamp-fenced re-dispatch is allowed, with a
metadata-only audit event and warning. Lago v1.50 applies voids synchronously;
the full-day quiet period is conservative protection against a double void.
Each further uncertain attempt waits another 24 hours. The diagnostic clears
automatically on receipt recovery and reports the next retry time.

The additive processing deadline is bound to its token. The legacy `lease_until`
stays at least as late as an uncertain dispatch's retry boundary, so an old
binary cannot use the upgraded worker's immediate release to re-send. An old
takeover changes the token, restoring that old worker's lease as the authority.
Old workers still ignore dispatch markers once their lease permits takeover;
these retry rules require every writer to upgrade. A void name is not provider
idempotency: a pause between the final sender check and the socket write can
still let an old request land after a later retry.

Cold wallet provisioning takes the renewable `billing:provision:{owner_id}` lease;
existing wallets return before any lease command. Lago customer/subscription ids
remain deterministic, and new wallets use the unique active customer/code pair
with code `nyxid`. A definite create refusal clears its dispatch marker. An
uncertain create is read back on retry; after two minutes (six 20-second client
timeouts) without a wallet, one new create is allowed. The stable provider code
also protects against a late first create. The checkpoint is deleted in the
same transaction as the local wallet and funding posting, leaving no cleanup
gap after that commit. Old binaries ignore these barriers; old code-less creates
can still race using another code. Rollback needs no data change.

Both dispatches capture monotonic and wall-clock time when their marker commit
returns. Immediately before HTTP send, either clock showing more than 30 seconds
elapsed suppresses the send as ambiguous; the marker stays for readback and the
existing quiet period. A backward wall-clock step also fails this check. Along
with the 20-second client timeout this limits ordinary sender delay to about 50
seconds after observing the commit, not an absolute bound across a pause after
the check. No new database command is added.

During graceful drain, expiry finishes the current wallet operation's durable
step before the reconcile pass stops at its next unit boundary. Renewal continues
and normal return releases only that operation's processing token. Request-path
provisioning remains request work; its owner lease is excluded from bulk shutdown
release. On return it is released or deleted by exact token; on abort it expires
by the shared lease TTL while its dispatch checkpoint remains recoverable.

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
update. Once every exact key exists, scalar covering definitions need no
presence inspection or nested traversal. v5 added the metering-only
`audio_input_tokens` / `audio_output_tokens` measures (provider-reported voice
subsets of prompt/completion, never priced). v6 adds the equally metering-only
`image_input_tokens` / `image_output_tokens` subsets and retires v5; summaries
folded before audio or image metering read those measures as zero. Before completion, document reads
preserve the missing-versus-null distinction. Startup creates replacement indexes
before retiring their known obsolete definitions; unrelated indexes remain
unchanged.

## D6. API and clients

Remote Machine runtime rates additionally bind immutable `profile_version` to the controlled resource spec. New staging accepts only the approved `remote-machine` profile and exact 0.000040 Credits/second; historical settlement reads retain their accepted exact prices. Quote and preflight share the profile/plan/sync/verification/retirement selector, while pinned quotes and active runs validate their original rate/profile/price/terms without repricing. Missing `profile_version` only in historical rates (including embedded run snapshots) means literal `remote-machine`, established by the original sole-profile constructor; explicit null and unknown bindings do not admit. No migration, ledger rewrite or recharge is performed. Upgrade every backend replica and runtime billing worker before enabling any additional profile's rates. See [Remote Machine API contracts](REMOTE_NYX_MACHINE_FINAL_READ_CONTRACTS.md#controlled-profile-and-runtime-rate-contract-2026-10-15).

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

### Stream usage checkpoints

Funded HTTP event streams make a best-effort checkpoint of protocol-confirmed
final usage before forwarding its event, using the relay's existing event and
JSON parsing. A write waits at most one second. A database error, timeout or
lost row predicate logs one metadata-only reason and disables checkpointing for
that stream; bytes and normal finalization continue unchanged. Text, interim,
private ChatGPT, Gemini, individual image and unknown reports add no writes.
Anthropic interim counters merge in memory. Duplicate final reports are skipped;
a later ambiguous report can withdraw a prior eligible checkpoint once.

The optional meter `usage_checkpoint` remains separate from final quantities.
Recovery promotes only a usable final report, atomically with its meter
quantities, then uses ordinary idempotent settlement. Pool recovery respects the
attempt lease; ordinary checkpoint recovery uses the existing reservation-abandon
delay and reconcile cadence. Missing classes retain the existing unknown-usage
policy. Non-streamed and metering-only requests add no checkpoint work.

Old binaries ignore the optional field and retain their previous crash exposure;
an old pool recoverer may still release an unknown attempt before a new recoverer
uses its checkpoint. New recovery never charges an already released row.
Rollback needs no data change, and promoted final quantities remain recoverable
by either binary. Voice retains its existing durable duration/response windows.

### Realtime WebSocket session recovery

Funded direct and node Realtime bridges initialize an optional `realtime_session`
on the existing coordinator meter row in the transaction that marks the request
forwarded. It has a database-time one-hour deadline and starts with zero observed
usage. Each distinct `response.done` boundary saves one cumulative numeric prefix,
including completed responses without usage; duplicates, content/audio frames,
unsupported sockets and metering-only sessions add no checkpoint writes. Each
write uses the same one-second best-effort bound as HTTP streams: failure logs
metadata once and disables further checkpoints, while delivery and normal close
continue. No payload or response-ID list is stored.

The one-hour bridge limit covers blocked awaits and checks monotonic and wall
clocks before resumed work. A healthy client still receives the ordinary close
at the limit; close flushing is bounded to one second. Recovery starts only after
the database deadline plus 60 seconds, on the existing reconciliation cadence.
Normal close and recovery compete in one transaction that seals the session and
publishes all final meter quantities. A late close cannot replace recovery; open
prefixes are never final quantities. Ordinary settlement then charges once and
returns unused wallet, grant and allowance holds.

Recovery charges the recorded prefix by the existing close rule: one forwarded
request, recorded transport bytes and reported token classes. Aggregate-token
pricing adds `ceil(uncovered_bytes / 4)` once over the durably recorded uncovered
segments; typed classes remain reported-only. A saved pricing flag preserves routes that
already estimate all transport bytes, including resale; enabling durability does
not switch their pricing to provider totals. The policy is named
`recovered_session_usage` in `billing/realtime_usage.rs`. With no durable response
boundary, zero observed bytes/tokens is the crash billing decision, not proof of
no consumption. The unrecorded suffix is waived. A normal close uses its complete
in-memory totals and never adds the checkpoint again.

Old binaries ignore the optional state; sessions they start keep their existing
behavior. Either version can settle published final quantities. Rollback pauses
recovery of open marked sessions until capable workers return, without a data
change or an operator step for an ordinary rolling update. A process pause between
a deadline check and socket I/O can still allow an unobserved effect; the terminal
fence prevents another settlement, not that external effect. Certified model
sockets without the required native usage vector and other unsupported protocols
retain their existing behavior; no generic quantities bypass their evidence rules.

### Usage dashboard filters and models

The admin Usage page keeps its shared filters directly below the heading. Users
select request actors; Users / organizations selects billing owners. Service and
UTC time filters apply to summary totals, all charts, and the List tab. Platform
keys are billed to the acting person, so filtering an organization as billing
owner does not mean filtering all activity by its members. The NyxBot tab and its
assistant reply feedback retain their own filters.

Analytics supports `breakdown=model` using the existing meter and rollup model
dimension. Each chart exposes a Group by selector; new workspaces start with a
Tokens by model panel. Select Chrono LLM as the service to inspect its models.
Model reporting reuses the existing pricing-model / provider-reported-model
projection; reporting never writes a model into the pricing or Lago event path.
A recorded model does not prove an underlying provider, fallback, or model
revision. Those details need an upstream reporting contract. Existing records
without a model remain Unattributed; no historical model identities are inferred
or backfilled. Provider input/output/cache token counts keep their existing
accounting semantics.

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
0.30.2 against the exact-accounting implementation retained in 0.34.0, using
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

### Assistant provenance in usage summaries

Assistant provenance is additive display-partition metadata, not a root rollup
identity. Existing root hashes/grouping and the exact-v2 normalization contract
remain stable. Modern free meters retain the low-cardinality assistant source;
they do not introduce per-key/ack partitions for unrelated free traffic. Existing
connection attribution remains available. Source snapshots and zero-cost
partitions survive API-key revoke/soft delete and credential removal.

Upgrade all rollup writers to preserve this provenance through folding. An old
writer ignores the source field and may fold newly captured free usage into a
source-less summary during a mixed-version rollout. The scoped report excludes
that uncovered summary even when its underlying meters were captured recently.

A legacy free root can have measures but no cost partitions. When modern source
partitions join that root, readers recover the root-minus-partitions residual
for every additive measure. It remains unattributed and is never assigned to the
new source. This calculation is on the atomic document read, including late old
writers and daily bootstrap/promotion. A single-display accelerator is valid
only when its sole partition accounts for all root rows. NyxBot free-summary
reads always expand partitions plus residual, so an old writer rebuilding its
older shortcut cannot assign the combined root to one new source. Normalization
still writes exact money and scalar query mirrors atomically with concurrent
folds; this reporting change adds no hot-path migration or money admission gate.

Mixed exact and legacy groups preserve the exact gross/wallet/allowance subtotal
when the legacy cached rate is absent. Cost presence remains null for an
unknown-only group, while known zero exact settlements remain known. Model
account Other rows subtract in Mongo before presentation count clamping and
subtract cost-presence counts separately, retaining unknown-only remainders as
unavailable. No quantities or distinct cardinalities are fabricated in the
model-report DTO.

### Wallet usage tax

`UsageFunding.wallet_funded` and `total_charge` remain exact **pre-tax** funding
amounts. Newly admitted rows set `tax_accounting` and retain an optional applied
Lago rate snapshot. Only a nonzero wallet share requires a fresh tax profile;
benefits remain untaxed. A benefit-only estimate that spills into wallet funding
at settlement uses the latest verified stored profile regardless of age, since
upstream work is complete; no verified profile leaves settlement retryable.
This snapshot read occurs before the Lago carry transaction, which persists it
atomically with the claim-fenced funding split. Wallet holds use base plus tax ceiled at one pico;
settlement uses base plus proportional tax rounded half-up at one pico.
`wallet_tax_credits` records that tax and `wallet_charge_credits` records the gross
debit. All wallet reserved/pending counters and overdraft checks are gross.
Machine runtime windows use the same rule; preflight keeps grant seconds separate
from wallet seconds priced at the gross per-second rate.

The claim-fenced Lago carry still uses only `wallet_funded`, so tax never changes
emitted quantities or cumulative residue. The existing usage-settled ledger
append journals pre-tax usage and tax as separate balanced transfers from the
wallet, using `usage:<row>` and `tax:usage:<owner>`. Locks remain until that entry
is durable. Lago fee/tax cent rounding remains an ordinary provider-refresh
`external:lago` adjustment. No ledger/hash encoding, dedupe key or checkpoint
protocol changes. Legacy rows without tax fields settle pre-tax. Deploy all
replicas before enabling Lago customer taxes; see [operations](BILLING_OPERATIONS.md).

## Organization channel guest budgets

Default-off `nyxbot:org-specialist-channels` adds one typed payer exception to
`BillingOwnerResolver::resolve_for_execution`. A verified org-specialist channel
**guest** with live admin-enabled exposure bills its organization's provisioned
wallet for model and delegated tool usage, including platform keys. Members keep
org BYOK → org and platform key → person. Ordinary platform-key execution keeps
the billing principal and `PaysFrom::Personal` classification without an owner-type
read, including org-owned API keys whose principal is the org. Those keys never
enter the guest lane. No auto-provisioning or transport-admin sponsorship is used
for guests.

After `nyxbot:org-guest-grants-v1` fleet readiness, no daily cap is required or
enforced for either legacy or Grants-adopted bindings. Before readiness, old and
new replicas retain the original positive-cap enforcement. The saved cap remains
for compatibility; do not delete or reset it or the financial records.
`org_channel_guest_budgets` and `org_channel_guest_holds` continue reserving the
existing exact allowance/grant/wallet estimate in the funding transaction. Only
the daily-cap predicate is omitted in ready mode. Wallet suspension, subscription,
funding and the six-per-minute sender limit still apply. The hold includes benefit
value and tax-inclusive wallet reservation; benefits remain untaxed. Usage-ID
settlement/release stays idempotent and uses the original UTC admission day,
including holds created before cutover and settled after revocation or midnight.
Ordinary/member billing gains no extra reads or payer changes. Rollback disables
and drains guests before downgrade; spending above the retained legacy cap can
refuse further old-binary admissions and must never be erased to bypass it.

Usage retains org `billing_owner_id` and additive `org_channel_guest` metadata
(binding ID and a scoped sender hash, never message text or credentials). The
org's billing usage and bot settings show spend; settings also show reserved
credits and named guest access from Grants or the frozen legacy restriction. Financial rows follow accounting
retention and are never discarded while unsettled. Every executing replica must
support the typed lane before exposure is enabled; its observed-support
readiness converges automatically without roster/drain declarations. Independent
billing migration and rollback requirements above remain unchanged.
