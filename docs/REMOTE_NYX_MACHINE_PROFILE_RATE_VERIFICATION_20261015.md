# Profile-to-rate implementation verification — 2026-10-15

Status: implementation and scoped local verification complete. Caller-owned independent review and real selected-cluster acceptance remain pending. Scope is MVP 0e and the controlled-profile section of FINAL_READ_CONTRACTS. The 19 pre-existing untracked Markdown files are byte-for-byte preserved, and branch HEAD remains `bde170f0e5522acce16891f4824daa806b661269`. No commit or deployment was made.

## Before-source documentation milestone

The MVP objective/priority requirements and P1–P6 acceptance table, API/read contract, and this planned verification document were written before any implementation source edits. Baseline branch/head and historical-document SHA256 values are recorded in the runner attempt `baseline.json`. Source tracked status was clean at that point. This milestone does not assert later tests passed.

## Criteria established before implementation

- P1: admin safe profile response; approved rate/profile staging; null/unknown and invalid prices.
- P2: real Mongo selector excludes foreign profiles, unverified/unsynced/retired/wrong-plan rates; explicit no-match; quote/preflight alignment and pinned estimates.
- P3: free/paid canonical spec, mismatch rejection before wallet hold/Machine/outbox writes, exact replay and changed-profile conflicts.
- P4: frozen prices, renewal/settlement/resume, saved image/profile/RuntimeClass/PVC; adapter/quota/manager and actual Sandbox renderer/CEL.
- P5: missing-field top-level and embedded BSON rates map only historical literal; explicit null and unknown/mismatch rejected, no ledger change.
- P6: actual frontend components/hooks/intents with API doubles, Vitest/typecheck/lint; preserve original setup/capability/specialist behavior.

Verification used only the new task-owned MongoDB 8 replica set `nyx-profile-rate-20261015-d23aa502`, bound to `127.0.0.1:52017`, replica set `profile_rate_rs`. Every backend test explicitly received `NYXID_TEST_DATABASE_URL=mongodb://127.0.0.1:52017/?directConnection=true&replicaSet=profile_rate_rs`. Primary/insert/abort/commit readiness was proven with mongosh and the real Rust transaction helper. Cargo builds ran sequentially with `CARGO_INCREMENTAL=0`; free disk space was checked before heavy builds. The task container and its disposable volume were removed after the final suites. Existing containers were not restarted, removed or pruned.

## Boundaries

No commit, push, deployment, live-data inventory or in-use service mutation. No additional production size, resize or disk expansion. No image rebuild if image/Node sources are unchanged. Selected-cluster Kubernetes/runtime/CSI checks, physical cutoff and strong predecessor writer isolation retain their previous pending/deferred boundaries. Independent review belongs to caller.

## Implemented criteria

| Criterion | Implementation |
|---|---|
| P1 | `models/machine_runtime_billing.rs`, `billing/machine_runtime/prices.rs`, `handlers/admin_machine_runtime_billing.rs`; existing safe profile view and rate identity/metric generation. |
| P2 | `latest_machine_runtime_rate`, `machine_runtime_preflight(owner, profile_version, quote)`, `handlers/remote_machine.rs`; one selection rule, pinned quote contract and explicit unavailable pricing. |
| P3 | `remote_machine_service::{controlled_profile,resolve_profile,normalize_profile_request}`, `remote_machine_runtime::prepare_start`, Billing prepare/accept/live reread; strict HTTP DTOs and service-side validation. |
| P4 | saved-contract admission in `remote_machine_runtime`/manager; actual-profile quota; shared immutable resource constants; renderer retains canonical existing strings and strict allowlist. |
| P5 | Missing-only serde default for rate snapshots; no data rewrite. The sole historical identifier is established by captured source at `bde170f0`. Exact full old intents may omit only the new profile field; all other parameters and saved contract remain checked. |
| P6 | schemas/hooks/intents and create/resume/preflight/detail components; sole profile is automatic, resource values come from safe responses, estimates match profile/rate/exact price/freshness. |

Administrative rate staging now requires both `profile_version` and decimal-string `credits_per_second`. Public create/quote requests allow omission of profile only as the literal original selection, reject explicit null/unknown and reject arbitrary allocation fields. Resume keeps the saved profile and current quote for it. New rates start pending; existing Lago sync/readback and retry must succeed before new quoting.

A preliminary backend no-run build exposed fixture-only missing field, field-name and Mongo collection type annotations. The errors and corrections are retained in `backend-green-build*.log`; they are not successful test evidence. The two original implementation regressions failed before fixes as intended (`backend-red-billing.log`, `backend-red-machine.log`). Frontend red logs likewise retain seven original and two saved-retry estimate failures before fixes. The final passing evidence follows; earlier failures remain in separate logs and are not counted as passes.


## Final local results

The attempt artifacts are under `/var/folders/v4/y1w4cvsd32x9y9wxrw_98llc0000gp/T/consensus-rnd/sshx/nyx-profile-20261015-implementation/attempt-1`. `verification-manifest.json` records final changed-file hashes, test log hashes, executable identity, unchanged historical documents and cleanup. `docs-first-milestone.json` proves that the three initial documents were written at `2026-10-15T05:37:57Z` with an empty tracked-source diff. `final-source-snapshot.json` binds the final backend build and unchanged frontend sources.

Fresh backend compilation succeeded with:

```sh
CARGO_INCREMENTAL=0 cargo test -p nyxid --bin nyxid-server \
  --config 'profile.test.package.nyxid.debug=0' --no-run
```

`backend-green-build-4.log` produced `target/debug/deps/nyxid_server-3aabc3a1b2ac32bb`. Six passing groups below (303 tests) ran sequentially on that new build. The admin route test then exposed only a BSON timestamp-precision assertion issue; after correcting the test, `backend-green-build-5.log` rebuilt the final sources and its fresh executable passed that admin test. All production source hashes are identical between builds 4 and 5 (`test-only-timestamp-fix.json`); the other passing groups retain their build 4 evidence. Both executable hashes are preserved in the final manifest. Every backend run used the explicit disposable database URL. No selected test was ignored or skipped; filtered-out tests were not run and are not counted. The seven groups contain **304 distinct backend tests**.

| Filter | Test threads | Passed | Evidence |
|---|---:|---:|---|
| `transaction_test_database_supports_atomic_writes` | 1 | 1 | `backend-db-readiness.log` |
| `services::billing::machine_runtime::` | 2 | 21 | `backend-billing-runtime.log` |
| `remote_machine` | 4 | 182 | `backend-remote-machine.log` |
| `runtime_profile_admin_routes` | 1 | 1 | `backend-admin-profile.log` |
| `services::billing::reconcile::tests::machine_` | 2 | 4 | `backend-runtime-reconcile.log` |
| `services::billing::machine_runtime_math::` | 2 | 2 | `backend-runtime-math.log` |
| `services::machine_external` | 4 | 93 | `backend-machine-external.log` |

Additional current-source checks:

| Command | Result | Evidence |
|---|---|---|
| `CARGO_INCREMENTAL=0 cargo test -p nyxid-machine --lib -- --test-threads=2` | 38 passed; 0 ignored | `shared-machine.log` |
| `CARGO_INCREMENTAL=0 cargo test -p nyxid-machine-updater --lib sandbox::tests -- --test-threads=2` | 39 passed; 0 ignored; 51 outside-filter tests not run | `sandbox-updater.log` |
| `cargo run --quiet -p nyxid-machine-updater --example sandbox_contracts` | Actual renderer fixtures exported | `sandbox-fixture-export.log`, `sandbox-fixtures.json` |
| `sh deploy/remote-machine/tests/contracts.sh <attempt>/sandbox-fixtures.json` | 17 actual CEL validators; 3 real Rust-rendered objects; 262 cases passed | `admission-cel.log` |
| `npm test -- --run remote-machine use-machines machines.test.ts machine-setup.test.tsx machine-grants` (frontend) | 17 files; 232 passed; 0 failed/skipped | `frontend-regression.log` |
| `npx tsc -b` (frontend) | Exit 0 | `frontend-typecheck.log` |
| `npm run lint` (frontend) | Exit 0; 0 errors / 33 warnings | `frontend-lint.log` |
| `npx eslint <15 changed frontend files>` | Exit 0; 0 warnings/errors | `frontend-changed-lint.log` |
| `P4_E2E_OUTPUT_DIR=<attempt>/frontend-e2e-final-artifacts npx playwright test --config playwright.remote-machine.config.ts` (frontend) | Fresh final-source rerun: 12 passed; 0 failed/skipped | `frontend-e2e-final.log`, screenshots and source snapshot |
| `rustfmt --edition 2024 --check <22 changed/new Rust files>` | Exit 0 | `rustfmt-final.json` / `.log` |
| `git diff --check` | Exit 0 | Final manifest |

This is **625 passing automated tests plus 262 CEL admission cases**, not a claim that the entire repository suite ran. The backend build emitted 35 warnings, matching the baseline count; full frontend lint had 33 warnings, while changed-file lint was clean. Earlier overlapping frontend runs and the first browser run are retained but are not added to totals.

## What the passing tests establish

- **P1:** Mounted admin POST/GET/retire routes expose `profile_version` and the safe CPU/memory/GiB projection while preserving prices, sync/retirement and timestamps. Unknown/obsolete/null profiles, unapproved/zero/negative/NaN prices and resource fields cannot insert a rate. UUID and metric derivation remain stable.
- **P2:** Real Mongo selection excludes otherwise well-formed foreign rates, explicit null, unsynced/unverified/retired/wrong-plan and invalid records. Quote and entitlement preflight agree on profile, rate, price and estimate. No match has an explicit reason. Pinned quotes retain their price/estimate after newer rates or retirement.
- **P3:** Paid/free specs agree; profile, resource, image and quote tampering leaves wallet holds, Machine/run/interval rows and outbox unchanged. Exact and pre-additive full-intent retries survive expiry/provider outage/changed deployment image without new funds/resources; different bound parameters and corrupted saved rate profiles reject.
- **P4:** Existing real-service journeys cover frozen price, concurrent atomic funding, interval replay, renewal, settlement, manual resume and native lifecycle uncertainty. Pause/resume retains original profile/image/RuntimeClass/PVC name and UID. Quota uses validated resources and zero additional retained workspace/PVC demand. Actual adapter/renderer/CEL tests retain the strict fixed allowlist. Authorization, grants/revocation and local gateway contention pass in the 93-test external suite.
- **P5:** Missing top-level and embedded BSON profile fields decode to the original exact rate identity/price and support renewal. Explicit null fails; unknown/mismatch does not admit. Source-established compatibility uses the literal historical identity and performs no migration, backfill, repricing or historical recharge. The positive synthetic quote fixture is canonical; obsolete quote terms remain negative cases.
- **P6:** Actual schemas/hooks/forms/intents display safe response dimensions and compare profile, rate, exact picocredit price and freshness before showing estimates. Saved retries retain their original quote instead of showing a replacement estimate. Browser desktop/mobile flows retain the three original setup choices, hosted flow, capabilities and specialist behavior.

Final regression corrections preserve the existing HTTP 409 resource/quote conflict and HTTP 402 Billing-unavailable mapping. The admin boundary now maps malformed decimal input to validation rather than internal error. Fixture corrections supplied a complete pinned image digest, used the authoritative existing HTTP status, and compared POST/GET creation time at BSON millisecond precision while preserving full equality of all other fields. Initial failing suite logs are retained as `backend-*-initial.log` and `backend-admin-profile-before-timestamp-fix.log`; the final logs above all pass.

## Evidence boundaries and handoff

Real components exercised: Rust/Axum mounted routes and services; MongoDB 8 transactions and exact Credits/funding/settlement; OAuth/JWT/API-key/grant/revocation paths; actual Sandbox renderer and CEL expressions evaluated by cel-go, with kubectl kustomize; React/RHF, Zod, TanStack Query, immutable browser storage, Vite and Chrome.

Explicit doubles: Lago/provider responses; Kubernetes/Agent Sandbox control and observation HTTP responses; local bootstrap/WSS peers in journeys; frontend HTTP API responses. Synthetic admission requests exercise actual validators without a live Kubernetes API server/controller/CSI. No live-data inventory was performed or inferred. Historical constructor evidence is captured from `bde170f0`, not from a production database.

Only `remote-machine` 2 vCPU / 2 GiB / 20 GiB remains saleable at the approved new-sale price; temporary capacity remains null. Optional RuntimeClass/StorageClass and existing auth, consent, retention and lifecycle policies remain. No image/Node implementation sources changed and no image rebuild occurred; shared immutable resource constants and renderer expressions retain the canonical existing allocation output. No additional production profile, resize, disk expansion or data migration is implemented.

Upgrade all backend replicas and runtime Billing workers before enabling any future profile's effective rates. Caller-owned independent review, selected-cluster/runtime/CSI acceptance, and the previously deferred physical-cutoff/strong-writer-isolation proof remain pending. The implementation worker performed no commit, push, deployment or live data/service mutation. `result.json` and `completion.sentinel` are published only after this evidence and cleanup are complete.
