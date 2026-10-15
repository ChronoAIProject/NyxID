# Remote Machine personal feature flag — local verification, 2026-10-15

Scoped implementation is locally verified and awaits caller review. This report is current-task evidence, not a deployment-readiness or whole-project acceptance claim.

## Scope and preservation

One key, `experimental:remote-machines`, is declared in the existing backend and typed frontend catalogs. The same backend definition is false in production and test registries. The service helper uses the existing personal actor resolver and existing override precedence. There is no new role, registry, organization policy or rollout endpoint.

The MVP plan and final API/read contracts were updated before source edits. `docs-first-milestone.json` records that checkpoint with no non-document mismatch against the supplied baseline. The initial preservation snapshot matches all 60 hashes in `/tmp/nyx-remote-flag-20261015/baseline.json`. Deltas use its `baseline-files` snapshots, or the baseline HEAD for previously clean files absent from that index; they do not treat the pre-existing work as this task's patch.

All 19 historical untracked reports and the prior profile/rate verification report remain byte-identical. `protected-reports-preservation.json` contains their before/after hashes. The inherited `machine/src/managed.rs`, `machine-updater/src/sandbox.rs`, profile/rate, exact Credits, grant, Sandbox/PVC and pricing changes are retained. Current-task edits consist of admission code, tests and these three current documents; no image/Node implementation change or image build was made.

## Acceptance and implementation

| Requirement | Implementation and evidence |
|---|---|
| Default false; personal isolation | One shared false definition in both registries; default/personal resolution test and full existing specificity matrix; mounted enabled actor does not opt another actor in. |
| Quote/create/resume/rekey admission | Human/owner checks precede the service helper; shared lifecycle gate covers Resume, dedicated Rekey and generic Rekey before platform/replay/funding effects. Mounted valid free/paid flows succeed ON; absent/OFF denial compares durable resources, quotes, operations, holds and provider-call counts. |
| Exact denial and retry | Existing HTTP 403 / `forbidden` / numeric 1002; wire message `Forbidden: Remote Machines are not enabled for this account`. OFF denies new HTTP retries; ON restores exact saved create/resume/rekey replay. |
| Runtime-rate authority | Stage/resync/retire keep first-party-human then platform-admin checks before personal flag. Enabled ordinary users, operators, external OAuth, delegated tokens and API keys remain denied. Denied resync has zero provider calls. Admin/operator GET is independent of the flag. |
| Ongoing owner management | Enabled free/paid Machines reach Ready through production services and declared observation fixtures; OFF still permits mounted owner list/get/entitlement/history/operations/billing reads and pause/delete, with foreign-owner denial. Free `/billing` retains its existing exact 404 when no paid contract exists. |
| Ongoing safety | ON→OFF established paid run renews funded coverage, records native running evidence, meters and settles exact pending Lago debit, releases holds and allows deletion. Actual manager funding tick still stops on insufficient spendable funds. Accepted outbox recovery and manager unallocated deletion/free-slot cleanup continue OFF. Background rate retry and reconciliation tests run with the new flag default OFF. |
| Frontend | Existing personal `useFeature` plus authenticated-current-user readiness; hosted fourth option/list CTA/direct form mount fail closed. Open free/paid resume reviews become ineligible on OFF/loading/account change. Captured-owner dispatch guards cover quote/create/resume/rekey; existing self-host choices and management remain. There is no existing hosted rekey button; its mutation hook is gated. |
| Profile/rate regression | Positive mounted fixtures, including nested HTTP journeys, explicitly opt in. Profile invalid-input checks use specific 400/422 responses; saved image/profile/PVC and exact replay tests remain. |

## Disposable database and build binding

Used only task-owned cached-image MongoDB **8.0.32**, container `nyx-flag-20261015-a6d1e436`, named volume `nyx-flag-20261015-a6d1e436-data`, replica set `nyx_flag_rs` and localhost port **62472**. `mongo-readiness.log` proves writable primary, insert, abort and commit. The Rust transaction-helper test also passes. Every backend suite explicitly received:

```sh
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:62472/?directConnection=true&replicaSet=nyx_flag_rs'
```

Free disk was checked before compilation (77 GiB initially; 76 GiB at build). The no-run build command was:

```sh
CARGO_INCREMENTAL=0 cargo test -p nyxid --bin nyxid-server \
  --config 'profile.test.package.nyxid.debug=0' --no-run
```

Execution deviated from the one-build target: one in-flight attempt was cancelled after a final test expectation edit overlapped compilation, then two no-run builds succeeded. The second successful build contains only consolidated test corrections. Both successful builds emitted 35 warnings, with no compilation error. No whole-workspace test/build command was used. The cancelled attempt is not passing evidence.

The first successful executable SHA256 is `67e399532dca943e28e65a1d39b759f36052f7f0f0a7086bdc98447825b2903d`. It supplied 237 distinct passing tests. Nine failed test cases required only test corrections: existing API Display prefixes (five mounted tests), explicit opt-in for three nested positive HTTP journeys, and the new OFF-settlement assertion using `pending_lago_debits` rather than reducing the provider balance snapshot. Production sources are identical across successful builds (`test-only-source-binding.json`). Only those nine failed cases were rerun on the final executable; broad passing suites were not repeated.

Final executable: `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID/target/debug/deps/nyxid_server-3aabc3a1b2ac32bb`. SHA256: `b245ae91adebd5b46a8f6685c448160fea27812973a29fa20db6f5e07b2b4763`. Final Rust-source snapshot covers 1141 files; aggregate SHA256 `8dc5a4adab218b9998f9d615a7dad158e464d5164c283d1e014320269b848843`. Each suite record carries its command, explicit database URL, selected/passing test names, executable SHA and log SHA. The runner checked source and executable hashes before each group. No prior-task executable/result is used as evidence.

## Executed tests

All commands below invoke the fresh test executable with `--nocapture` and the recorded `--test-threads` count, under the explicit database URL above. The broad `remote_machine` run excludes the five mounted flag tests and the registry test already selected elsewhere. Exact arguments are in the adjacent JSON records.

| Filter | Passed / selected in that invocation | Evidence |
|---|---:|---|
| `transaction_test_database_supports_atomic_writes` | 1/1 | `backend-db-readiness.log` |
| `services::feature_flag_service::tests::` | 30/30 | `backend-feature-flags.log` |
| `runtime_profile_admin_routes` | 1/1 | `backend-admin-profile.log` |
| `services::billing::machine_runtime::` | 21/21 | `backend-runtime-billing.log` |
| `services::billing::reconcile::tests::machine_` | 4/4 | `backend-runtime-reconcile.log` |
| `remote_machine` | 180/184 | `backend-remote-machine-before-test-corrections.log` |
| `routes::remote_machine_flag_tests::` | 5/5 | `backend-mounted-flags.log` |
| `services::remote_machine_runtime::tests::journeys::` | 3/3 | `backend-corrected-journeys.log` |
| `remote_flag_off_keeps_established_renewal_metering_settlement_and_delete` | 1/1 | `backend-corrected-safety.log` |

The corrected three journeys and one settlement test replace the four initial failures in the broad group; they are not added twice. Total: **246 distinct backend tests passed**, zero remaining failed tests, zero ignored/skipped database tests. The first failed invocations remain preserved; filtered-out repository tests were not run or counted.

Frontend verification ran from `frontend/`:

- `npm test --` with the 12 explicit files in `frontend-result.json`: **186 distinct Vitest tests passed**, zero failed/skipped. Includes feature catalog/hook, Machine mutations, setup/list, create/detail/paid start, schemas, intents and preflight.
- `P4_E2E_OUTPUT_DIR=<attempt>/frontend-playwright-artifacts npx playwright test --config playwright.remote-machine.config.ts`: **14 passed**, zero failed/skipped, preserving 12 flows and adding absent/OFF direct hosted URL flows.
- `npx tsc -b --pretty false`; `npx eslint <17 changed frontend files>`; `npx prettier --check <17 changed frontend files>`: all exit 0. Exact lists are in `frontend-result.json`.
- A post-format hook run passes 31 tests already included in 186; the earlier 142-test green run also overlaps. Expected red tests and resolved test-only selector/type/format issues are retained and are not acceptance passes.

This is **446 distinct automated test cases**, not a full repository suite. Frontend SHA manifest `3a1490dbb6bb608b1b42cc07b27d226d31321a4a3f902bdb5714ad3d4af36a84` binds all 17 changed files. The final 186-test selection preceded only a documented hook-test formatting/cast cleanup followed by its 31-test rerun; browser production behavior stayed unchanged. Final lint/typecheck/format checks followed all frontend edits.

Final formatting/diff checks: `rustfmt --edition 2024 --check --config skip_children=true <11 changed Rust files>` and `git diff --check` both exit 0. `rustfmt-final.json` and `final-checks.json` record the full commands. The branch and HEAD remain identical to the supplied baseline. Of the 60 initially dirty/untracked files, 42 are byte-identical and 18 have scoped flag/document/test deltas recorded against their captured contents.

## Real components, doubles and boundaries

Real: mounted Axum routes, signed JWT and API-key handling, role checks, persisted feature overrides/resolver, MongoDB 8 transactions, lifecycle/manager services, exact Credits/reservations/intervals/settlement and retained ownership fences. Frontend uses real forms, Zustand auth state, feature/mutation hooks, QueryClient, schemas, Vite and Chrome.

External doubles: cfg(test) platform readiness, local Lago protocol implementations, Kubernetes/Sandbox HTTP and native observations, Node lease/capability metadata and retained PVC identity. The paid deletion test supplies an explicitly named native-cleanup proof double; separate actual manager tests cover unallocated cleanup and accepted outbox progress. Frontend component/server hooks and browser HTTP responses are doubles; frontend checks do not prove backend enforcement. No live Kubernetes/controller/runtime/CSI, physical stop, storage erasure, image or Node acceptance is claimed.

Only the task container, its named data volume and its anonymous config volume were removed after database suites. The task-owned Vite process exited; port 4627 has no remaining listener. Existing MongoDB/Mailpit and other in-use services were not stopped or reused. `cleanup.json` records removal. No commit, push, deploy, live flag/data mutation or live service/configuration change was performed. Caller review and the pre-existing selected-environment/physical-isolation acceptance boundaries remain pending.

Artifacts: `/var/folders/v4/y1w4cvsd32x9y9wxrw_98llc0000gp/T/consensus-rnd/sshx/nyx-flag-20261015-implementation/attempt-1`. `verification-manifest.json` and `task-delta.diff` contain current-task hashes/deltas, preservation, command records and log bindings. `result.json` and its sentinel are published atomically only after final checks and cleanup.
