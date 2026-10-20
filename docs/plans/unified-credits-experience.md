# Credit allocations on current main

Credit allocations extend the current exact-accounting implementation. All
financial mutations retain checked picocredit arithmetic and Decimal128 storage.

## Accepted scope

Only admin Credits changes. Preserve the existing Credit grants / Free allowances / Schedules tab order, URLs, current allowance bundles, reviewed sparse edits/conflicts, recipient kinds and role boundaries. User Billing, wallet/top-up and navigation remain unchanged. Keep the current exact frontend credit library and standard preview command.

Allocate one shared exact credit total, optionally repeated daily/weekly/monthly, with expiry and all/selected-service eligibility. Optional disjoint canonical service groups share ceilings within that total. Unspecified eligible services remain unrestricted. Caps do not earmark funds or become account-wide policy. Show a compact all-allocation monetary headline and optional plain details; show allocated/used/remaining/held, limits, expiry and reclaim inline per allocation. Confirm reclaim with allocation/date/period identity.

Keep existing grant, schedule-period and journal identities. All monetary state uses checked Credits at 12 decimals and exact BSON adapters. Missing new fields remain uncapped/not pending. Stable actor UUID requests freeze payload/recipient snapshots and recover partial concurrent retries. Recurring periods freeze policy and every grant starts fresh counters. Existing target semantics and at-most-50 inline activation remain.

Pending reclaim fences new reservations and extras immediately, honors admitted holds and acquired operations, and journals the final exact unused remainder once. Other allocations and purchased funds remain independent. All admission paths enforce limits: ordinary, public/pool/managed durable admission, machine caller-owned lifecycle transactions and actual extras. Preserve exact rates, carry, no-wallet/legacy wallet-only behavior, public authority, pool lease and machine evidence boundaries. New holds require durable usage intent; do not guess historical orphan intent.

## Rollout

New policy authoring requires explicit capability and dedicated routes, never silent fallback. Upgrade every replica and billing worker and drain old writers before enabling BILLING_ALLOCATIONS_ENABLED. It is an operational gate, not automatic version attestation. Existing policy enforcement continues with authoring off; older binaries cannot safely return against capped/pending rows. Legacy actions remain available without the summary. Exact read fields alone do not prove write capability.

## Validation contract

Backend regression coverage uses a real MongoDB replica set. It must verify
one-picocredit and legacy BSON compatibility, concurrent total/group ceilings,
durable admission rollback and duplicate recovery, settlement claim fencing and
takeover, pending reclaim and journal retry, frozen submissions/periods, live
membership and role boundaries. The current main suites additionally cover
public/pool/managed authority, machine evidence and finite reservations, exact
rate/carry conservation, migration readiness and balanced account checkpoints.
An existing-row upgrade test installs the settlement usage-ID indexes twice
and checks unhinted indexed lookup plans. HTTP tests deserialize exact-only and
legacy whole-credit bodies and reject invalid effective amounts without writes.

Frontend regressions cover the live admin page/dialog/hook contracts, older
backend fallback, reviewed sparse edits, permissions, URL tabs, mobile policy
display, exact amounts and retry/recovery. Customer Billing remains outside the
presentation changes.
