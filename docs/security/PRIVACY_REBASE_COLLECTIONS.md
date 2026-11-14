# Privacy registry additions from the round-10 rebase

Audited base: `cdcd410c20b0cd45b37b352394b367f348d41c25`. Paths below are
relative to `backend/src`. The production source scanner found six additions;
the two other names in the review have only test writers. None needs a new WS5
index, export subsection, erasure domain, or inventory revision.

## Financial verification evidence

`billing_hold_observations` is declared by
`services/billing/hold_conservation.rs:24` (`OBSERVATIONS`). Its writer is
`verify` at `:615`; the persisted document quotes these fields:

```text
_id = "{collection}:{id}"
wallet_id, kind, difference, pass, previous_pass, fingerprint,
repair_fingerprint, repairable, exact, persistent, stable_since, updated_at
```

The source collection is `billing_wallet`, `credit_grants`, or
`usage_allowance_periods`. The resource id in `_id`, and optional `wallet_id`,
indirectly identify the financial owner. No name/email or direct person selector
is persisted. `owner_id`, `recipient_user_id`, and `owner_user_id` are read from
the source resource at `:578-592` to resolve its wallet; they are not copied into
the observation. `OwnerSelector::NotPersonal` therefore means no direct person
selector here, **not** that this linked financial evidence is non-personal.

Disposition: `Retained(FinancialBilling)`, using the existing disclosure
"financial and billing records"; export `Excluded(RetainedRecord)`. Personal
erasure and the orphan sweep preserve outstanding verification evidence just
as they preserve its wallet/grant/period. No TTL: verification deletes a resolved
observation (`hold_conservation.rs:526`), repair deletes its own observation
(`:725`), and a completed scheduled pass deletes stale observations
(`account_reconciliation.rs:390`). Expected volume is at most one current
observation per mismatching wallet/grant/period, rather than one row per request.
No WS5 index: no erasure/export/sweep predicate is added, and verification reads
exact `_id` values using the existing primary index.

## Platform price and publication state

`billing_plan_charges`: constants and model are
`models/billing_plan_sync.rs:6-9` (`CHARGE_COLLECTION`, `PlanCharge`). The complete
persisted model fields are:

```text
_id, plan_code, metric_code, service_id, writes_rate_cache, metric_name,
metric_description, credits_per_unit, repair_attempts, repair_next_at,
repair_failed_at, repair_reason
```

Writers: `plan_sync::begin_in_session` (`services/billing/plan_sync.rs:233`),
repair backoff removal (`plan_sync/repair.rs:275`), and
`failure_in_session` (`plan_sync/repair.rs:354`). The latter's field projection
at `:295` quotes `plan_code`, `metric_code`, `service_id`, `writes_rate_cache`,
`metric_name`, `metric_description`, `credits_per_unit`, then adds the same
repair fields and `_id` at `:345`. The service id identifies a catalog price;
there is no person/owner/actor/wallet/grant/period id. Disposition:
`NotPersonal`, export `Excluded(NotPersonal)`. No TTL: durable desired prices
and removal intents survive for provider repair. Expected volume: one row per
platform plan/metric, including removed or historical prices. No WS5 index:
platform pricing configuration is outside person erasure, export, and sweep.

`billing_plan_sync`: `models/billing_plan_sync.rs:5` (`PLAN_COLLECTION`),
untyped document. All writers are `begin_in_session` (`plan_sync.rs:241`),
`completed` (`:254`), repair completion (`plan_sync/repair.rs:267`), and failure
dirtying (`:362`). Their stored fields quote:

```text
_id = plan_code
generation, dirty, uncertain, active_attempt, completion_generation,
verified_generation
```

`active_attempt` is a random operation UUID (`plan_sync.rs:152`), not an actor.
No person or financial-owner linkage is stored. Disposition: `NotPersonal`,
export `Excluded(NotPersonal)`. No TTL: synchronization state persists for
repair; one row per platform plan. No WS5 index: no personal path queries it.

`billing_rate_epoch`: `models/billing_rate_epoch.rs:4-8`
(`COLLECTION_NAME`, `RateEpoch`). Persisted fields quote `_id` and `generation`;
`_id` is the constant `"rate-cache"`. The only writers are
`advance_in_session` (`services/billing/rate_cache.rs:38`) and
`advance_captured` (`:77`). There are no person/actor/financial-owner fields.
Disposition: `NotPersonal`, export `Excluded(NotPersonal)`. No TTL: one
permanent ordering singleton. No WS5 index: platform state with primary-key
reads, outside personal erasure/export/sweep.

`billing_tax_plan_epoch`: `models/billing_tax_plan_epoch.rs:3-7`
(`COLLECTION_NAME`, `BillingTaxPlanEpoch`). Persisted fields quote `_id`,
`generation`, `guard_token`; `_id` is `"all-plans"`. Writers in
`services/billing/taxes/ordering.rs` initialize it (`capture_plan_epoch`, `:22`),
update its guard (`publish_in_session`, `:166`), and advance generation
(`invalidate_plan`, `:224`, duplicate-key retry at `:229`). The guard is random
publication coordination, not a person/token credential. There is no profile,
person/actor/financial-owner field. Disposition: `NotPersonal`, export
`Excluded(NotPersonal)`. No TTL: one permanent ordering singleton. No WS5 index:
only primary-key platform operations; no personal erasure/export/sweep query.

## Existing node-delivery erasure

`node_credential_desired`: `models/node_credential_desired.rs:5-8`
(`COLLECTION_NAME`, `NodeCredentialDesired`). Relevant persisted fields quote:

```text
_id, node_id, service_slug, service_id, owner_id, actor_user_id,
generation, acknowledged_generation, sent_unconfirmed_generation,
parked_reason, retry_attempts, retry_revision, pending, suppressed, retry_at,
claim_token, claim_until, preparing_token, preparing_until
```

All production writers are in `services/node_credential_delivery.rs`:
suppression in `record_changes` (`:241`), `advance` (`:293`), delivery claim
(`:435`), delivery outcome (`:538`), reconnect scheduling (`:594`),
`Preparation::finish` (`:696`), and `prepare_update` (`:732`). Other uses
resolve to the model constant re-exported as `COLLECTION` at `:19`.
`owner_id` is polymorphic person/org ownership; optional `actor_user_id` is
opaque attribution. There is no actor name/email, credential, or frame payload.
`service_slug` is a catalog slug, and `parked_reason` is a fixed reason code.

Disposition: existing domain cleanup, specifically `local::pass` node drain
plus `personal_erasure_registry::final_metadata`, owner selector `owner_id`.
Main already drains these children by `node_id` before removing the node and
finally deletes remaining rows by exact `owner_id` (`personal_erasure_registry.rs:24`).
Org-owned rows with the departing actor UUID remain unchanged. Export is
`Excluded(TransientCoordination)`: queue delivery/claim state is not authored
account content. No TTL: the existing node/owner lifecycle removes intent;
expected volume is one slot per node/service, enforced by the existing unique
`{node_id: 1, service_slug: 1}` index (`node_credential_delivery.rs:68`). No new
WS5 index: the per-node drain already uses that prefix; the final owner sweep is
over resource-bounded slots, not an unbounded per-event stream. Main also owns
the existing `{pending: 1, retry_at: 1}` retry index.

`node_delivery_intent_pins_existing_node_drain_and_final_metadata_cleanup`
pins the registry selector/disposition, drain-list membership, exact final-owner
filter, and absence of a competing inventory path.
`completed_erasure_redacts_every_persisted_attribution_and_global_identity_scan_is_clean`
also proves that a target-owned intent is removed and an org-owned intent with
the target actor id remains byte-identical, while the name/email global scan
remains clean outside audit logs.

## Test-writer evidence

The complete literal search (before adding registry/docs evidence) was:

```text
rg -n 'cache_unit_probe|shutdown_claims' backend/src
services/billing/plan_writers_tests.rs:622: ...collection::<Document>("cache_unit_probe")
services/billing/plan_writers_tests.rs:661: ...collection::<Document>("cache_unit_probe")
services/shutdown/tests.rs:177: ...collection::<Document>("shutdown_claims")
services/shutdown/tests.rs:184: collection: "shutdown_claims",
services/shutdown/tests.rs:194: ...collection::<Document>("shutdown_claims")
```

The probe's sole writer is `plan_writers_tests.rs:622` (a test transaction's commit marker);
`:661` reads its counter. `billing/mod.rs:2508-2509` declares that whole module
under `#[cfg(test)]`. There is no constant resolving to `cache_unit_probe`, and
no non-test read/write or startup use in `main.rs`, `db.rs`, services, or handlers.
The shutdown test writes at `shutdown/tests.rs:177`, supplies a dynamic test
release specification at `:184`, and reads at `:194`; `shutdown.rs:10-11` gates
that module with `#[cfg(test)]`. Neither test fixture has production retention,
volume, export, or erasure semantics. They receive no registry entries or
indexes. The scanner's existing test-filename and `#[cfg(test)]` rules apply;
no special-case exclusion was added. Any future production literal/constant
use will fail `every_production_collection_literal_is_registered`.

The rebased startup fixture uses the new base's unchanged `db::ensure_indexes`
(including node delivery indexes) plus its security-readiness and personal
erasure index calls. Upgrade snapshots now explicitly include the delivery
queue's indexes and assert both main-owned prefixes exist before branch startup.
The branch still adds only the accepted 17 indexes and never drops, renames, or
rebuilds existing indexes.
