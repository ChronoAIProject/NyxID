# Remote Machine single-current local verification — 2026-10-15

> **Prior-stage verification retained for full-feature coverage.** The measurements and commands below belong to the single-current resource-contract phase. The later optional StorageClass and relocated hosted-image results are recorded in [HOSTED_STORAGE_VERIFICATION_20261015](REMOTE_NYX_MACHINE_HOSTED_STORAGE_VERIFICATION_20261015.md). Older working notes mentioned below remain local and are not required repository documentation.

Implementation status: all eight approved implementation steps are complete for the authorized local scope. Final required checks below passed. This is not independent SSHX review, termination, whole-goal completion, selected-cluster acceptance or an MVP release claim. S7 remains caller-owned.

The sole identifiers are profile `remote-machine`, product policy `remote-machine-mvp`, individual deletion policy `remote-machine-data`, and account deletion policy `machine-data`. Resources remain 2 vCPU, 2 GiB RAM and a 20 GiB persistent workspace; temporary capacity remains undeclared. Optional Browser/Computer, native Agent Sandbox lifecycle, consent, exact Billing and existing deferrals are unchanged.

All worker artifacts are under `/private/var/folders/v4/y1w4cvsd32x9y9wxrw_98llc0000gp/T/consensus-rnd/sshx/single-current-implementation-20261015/attempt-1`. `verification-runs.json` contains exact executed argument arrays, working directories, timestamps, exit codes, counts and log hashes, including unsuccessful attempts. Raw diagnostics remain behind those log references.

## Requirement mapping

| Criterion | Current evidence and boundary |
|---|---|
| S1 | locally_verified: Contracts updated before source edits: before hashes 01:26:24Z; all contract writes completed 01:28:23Z; source edits began 01:29:05Z on 2026-10-15. 18 historical original bodies and all 7 supplied baseline copies retained; Heca dated measurements unchanged. LOCAL_ACCEPTANCE only hashed/preserved. |
| S2 | locally_verified: Single current profile/product/individual/account identifiers propagated through every named existing consumer. Legacy constants, positive old acceptance, stripped-intent digest replay, dual templates/CEL, temporary-capacity compatibility and historical-spec UI removed. 34 remaining active-scan hits across 13 files are explicit rejection/absence tests. Historical body exceptions are scoped and labelled. Node-only preset has no alias; full profile artifacts are renamed. |
| S3 | locally_verified: 174 Remote tests plus 16 runtime Billing and 10 exact-funding tests passed with real disposable Mongo. Current saved image/RuntimeClass A survives deployment B for quote/resume/rekey/exact retry; obsolete saved/current requests reject. Full canonical intent, quote owner/operation/generation/rate/expiry/consent and transaction/outbox funding checks remain. No duplicate resource/funding on accepted retries; failed and concurrent funding boundaries tested. |
| S4 | locally_verified: Individual deletion requires current loss acknowledgement and writes MachineOwner basis; retry preserves original basis/accepted_at. Missing basis stays closed instead of reconstruction from audit. Account deletion uses a separate current policy, first-party human gate and existing email/native/CLI confirmation. Durable AccountOwner basis precedes cascade; owner absence/admin deletion retains workspace by safety pause. 6 direct/Assistant mounted account tests, 3 account-basis tests, 3 Assistant fingerprint/confirmation tests and 13 CLI profile tests passed, with web Settings/Assistant and mobile native-boundary coverage in the 291-test suite. |
| S5 | locally_verified_with_platform_boundary: Actual renderer and 17 real CEL validators pass the one current profile with fixed 2 CPU/2Gi RAM/20Gi workspace, no temporary resource/sizeLimit fields, protected identity/network/token constraints and optional RuntimeClass. CREATE checks approved configuration; UPDATE retains exact admitted image, RuntimeClass and stable workspace template with UID/resourceVersion guards. 38 shared and 137 updater test executions passed; 5 out-of-scope updater ignores explicitly reported. Fresh full image passed all 4 capability/retention cases. |
| S6 | locally_verified: All required final checks passed. Per-command counts and failures are retained; 2 zero-test filters were corrected and not counted. Frontend 291 tests, mobile 50 tests, browser 12 cases; frontend full build, frontend lint (0 errors/33 warnings), mobile typecheck and scoped Rust formatting passed. Both current full and Heca images built and ran fresh local checks. No historic pass counts used. |
| S7 | caller_owned_pending: Implementation evidence is ready for separate independent review and termination. This worker does not claim review completion, whole-goal completion, deployment acceptance or release. |

## Fresh checks

Counts below are test executions, not a claim of globally unique tests. Library/bin targets overlap. The shared/updater zero-test doctest targets are not added to passing counts. No absent-Mongo skip is counted.

| Check | Final result |
|---|---|
| `mongo-transaction-probe` | 1 passed; 0 failed; 0 ignored |
| `backend-remote` | 174 passed; 0 failed; 0 ignored |
| `backend-runtime-billing` | 16 passed; 0 failed; 0 ignored |
| `backend-machine-funding-corrected` | 10 passed; 0 failed; 0 ignored |
| `backend-account-delete` | 6 passed; 0 failed; 0 ignored |
| `backend-account-basis` | 3 passed; 0 failed; 0 ignored |
| `backend-assistant-delete-corrected` | 3 passed; 0 failed; 0 ignored |
| `shared-machine` | 38 passed; 0 failed; 0 ignored |
| `updater` | 137 passed; 0 failed; 5 ignored |
| `cli-managed` | 34 passed; 0 failed; 0 ignored |
| `cli-usage` | 3 passed; 0 failed; 0 ignored |
| `cli-profile-delete` | 13 passed; 0 failed; 0 ignored |
| `cli-parser` | 33 passed; 0 failed; 0 ignored |
| `cli-setup` | 2 passed; 0 failed; 0 ignored |
| `frontend-focused-source-final` | 19 files; 291 passed; 0 failed/skipped |
| `frontend-build-3` | exit 0 |
| `frontend-lint-1` | 0 errors; 33 warnings |
| `mobile-typecheck-1` | exit 0 |
| `mobile-tests-1` | 50 passed; 0 failed/cancelled/skipped/todo |
| `browser-1` | 12 passed; 0 failed/skipped |
| `healthcheck` | 6 cases passed (kubectl fixture) |
| `renderer-export` | exit 0 |
| `deployment-contracts` | 17 CEL validators; 2 rendered objects; 6 obsolete rejections, 11 image retention, 11 runtime/storage, 24 temporary-field, 86 hostile cases |
| `full-image-build` | exit 0 |
| `full-image-smoke` | 4 cases passed; 0 failed/OOM |
| `heca-image-build` | exit 0 |
| `heca-image-smoke` | 8 CMA tests passed; 0 failed/skipped; package/daemon/MCP checks passed |
| `rust-format-check` | 24 changed Rust files passed |
| `shell-syntax` | exit 0 |
| `deployment-shell-syntax` | exit 0 |

The backend test executable was freshly compiled for `backend-remote`, then reused for later focused backend filters to avoid repeated package build-script relinks. Its SHA-256 is `f4dcef8fbde17974aa02cee561abeadbec2e3040696ec7f08227f3bbafa27dc3`. `backend-test-binary.json` records provenance. The initial broad `--no-run` compile was preliminary, before the last source edits, and is not substituted for these final executions.

Mongo ran as the dedicated disposable `nyxid-single-current-test-20261015` container using MongoDB 8.0.32 and `mongodb://127.0.0.1:27985/?replicaSet=rs0&directConnection=true`. The explicit transaction probe inserted and aborted a transaction. The container and its disposable anonymous volumes were removed after tests; no existing Mongo service was used. Provenance and cleanup are retained.

## Failed attempts, ignores and warnings

- `frontend-focused`: exit 254; npm invoked from repository root; no tests executed.
- `frontend-focused-3`: exit 254; npm invoked from repository root; no tests executed.
- `frontend-focused-2`: exit 1; stale current-policy fixtures and a disabled-button assertion; corrected; final 291 pass.
- `frontend-build-1`: exit 2; strict current-literal fixture typing; corrected.
- `frontend-build-2`: exit 2; remaining imported fixture literal widening; corrected; final full build passes.
- `backend-machine-funding` and `backend-assistant-delete` initially selected zero tests. They are not passing coverage. Corrected filters ran 10 and 3 tests respectively, all passed.
- The updater reported five ignored test executions: two instances of the live GitHub TUF mirror test, companion-container entrypoint, real companion handoff/rollback and real container migration/signed upgrade/rollback. These release/self-update checks are outside this profile/policy cleanup; they were not counted as passes.
- Frontend lint has 33 existing React refresh/hooks warnings and no errors. The successful build reports the existing generated `text-destructive` CSS pseudo-class and large-chunk warnings. Rust builds report existing unused/dead-code/private-interface warnings; no repository-wide warning cleanup or clean-clippy claim is made.
- Docs-first writes finished before source edits; a bookkeeping variable-shadow error after those writes was corrected in the chronology. A later historical recheck initially compared heading-inclusive Heca bytes to a heading-exclusive baseline digest; corrected range and direct byte comparison pass without changing historical data.

## Current image provenance and measured fixture boundaries

Full image: `nyxid-machine:single-current-20261015`, `sha256:667ab8546a95b8ce4660fa49f991a9aff812e5f766ccf694f97ffb67fb421b59`, linux/arm64. The source-only build context contains 1263 hashed current files, no prebuilt binaries or host credentials. Locked Cargo rebuilt the current CLI/Node/shared sources. The context is verified against final source files.

Heca derivative: `nyxid-machine:single-current-heca-cma-20261015`, `sha256:dc62979d8c26d93e656ae7a822ede07122649fda3357fe5bfa295adba3c3d6f7`, built on that exact full image. CMA source is clean at `ab789bde158b864d5db467fb2677ef72c3545cfc`; only its 17-file Codex subtree was copied into the disposable build context. Heca smoke freshly passed 8 tests across four CMA suites, plus pinned tool and authenticated daemon/MCP checks.

| Full-image case | Generation | Checks | Sampled cgroup memory peak (bytes) | Exit after signed cutoff (ms) | Exit / OOM |
|---|---:|---:|---:|---:|---|
| all-capabilities | 1 | 30 | 331612160 | 2154 | 0 / False |
| browser-only | 1 | 21 | 612589568 | 170 | 0 / False |
| commands-files-retained | 2 | 27 | 36737024 | 2113 | 0 / False |
| commands-files | 1 | 27 | 161619968 | 2179 | 0 / False |

The measured full-image limits are 2 CPU (`cpu.max = 200000 100000`) and 2 GiB RAM (`memory.max = 2147483648`), with total memory+swap 2 GiB. `/home` and `/tmp` are each 1 GiB tmpfs; identity is 64 MiB tmpfs; workload token is 1 MiB tmpfs. Browser cases use 32 MiB workspace tmpfs. Commands/Files generation 1 and 2 share an **unbounded disposable Docker volume**, with a signed marker verified after restart. These fixtures do not enforce or establish a 20 GiB CSI workspace or disk-backed emptyDir. Node wire telemetry correctly leaves temporary capacity/available unknown and reports lower-bound usage and actual workspace measurement.

The Heca smoke uses network-none, read-only root, uid 1000, cap-drop/no-new-privileges, 2 CPU/2 GiB limits, 512 MiB `/home/agent` tmpfs and 1 GiB `/tmp` tmpfs. Its observed cgroup values are retained in `heca-smoke-summary.json`. There was no cloud login, live relay/model request or layer4 native-tools acceptance.

## Source scope and historical preservation

The before manifest captured 3,727 files. This flight changes 73 existing files, adds four (two renamed profile artifacts plus historical index and this current result), and removes the two old profile paths. `changed-source-manifest.json` provides every before/after SHA-256 and byte count against the pre-flight dirty worktree, not Git HEAD. All other captured files are unchanged. Generic Billing sources, dependency manifests/locks and the Heca derivative Dockerfile are unchanged. No source edits were delegated.

The active literal/semantic scan covers 3,717 files, excluding the 18 explicitly indexed historical bodies and the Heca dated block. Its 34 hits in 13 files are explicit obsolete-input rejection or removed-field/flag absence tests. All named backend, shared, Node, deployment, web, mobile, CLI and Assistant consumers are included. Existing API, signature/authority, bootstrap AAD, deadline and Billing versions, genuine shell/files names and financial rate/run history remain.

All 18 historical bodies remain byte-for-byte identical behind a 480-byte scoped preamble. All seven supplied historical baseline copies remain unchanged. Heca dated bytes after the original heading retain SHA-256 `d84d662c19d2e5ab4e93788fd7e49ef3996e2e785e8086b77979557bdd2cd6d7`; heading-inclusive digest is `91bba951908137c356579e7667b73e9cb92004838548e167c94ba6217c3b2aa4`. LOCAL_ACCEPTANCE was treated as opaque preservation input only. Historical measurements and pass counts were never promoted into this record.

## Real components and substitutes

- MongoDB 8.0.32 is a real disposable local replica set; actual transactions insert and abort. It is not a live environment. All backend tests use the explicit loopback test URL; no missing-database skip is counted.
- Backend HTTP/lifecycle/Billing/owner-consent tests execute real Rust services and Mongo transactions with Kubernetes/controller, provider/Lago, identity and selected network responses supplied by local fixtures. They do not establish real cluster scheduling, controller defaulting, CSI, or provider production acceptance.
- Deployment uses real Rust-rendered Sandbox objects, real cel-go 0.26.1 expression evaluation, and real kubectl Kustomize rendering. No admission webhook or Kubernetes API server was contacted. Six healthcheck cases use a kubectl double.
- Full image runs current real Node/entrypoint, signed Commands/Files, Chromium navigation/snapshot, cua capture/window observation, revocation and descendant teardown. Bootstrap and WSS peers are local fixtures. Both images are freshly built linux/arm64 only.
- Full smoke enforces 2 CPU and 2 GiB RAM with memory+swap total 2 GiB (no swap). /home and /tmp each use 1 GiB tmpfs; protected identity uses 64 MiB tmpfs and token mount 1 MiB tmpfs. Browser cases use a 32 MiB workspace tmpfs; Commands/Files retains an unbounded disposable named Docker volume across two generations. These mounts do not verify Kubernetes disk emptyDir, Pod ephemeral accounting, or enforced 20 GiB PVC capacity.
- Web unit/components use DOM/native boundary fixtures. Twelve real Chrome cases at 1440px and 390px use fixture HTTP/WebSockets and Vite. Mobile checks are TypeScript and native Alert/HTTP boundary harnesses, not physical iOS/Android or native release builds.
- Heca uses current rebuilt base, pinned real packaged Heca/Codex/ACP/Node binaries, authenticated local daemon/MCP and four upstream CMA fixture suites with networking disabled. No live cloud authentication, relay or CMA layer4 runtime/native tools acceptance was reopened.

## Still deferred / caller-owned

- Caller-owned independent SSHX reviews, aggregation and applicable termination judgment (S7).
- Selected-cluster Agent Sandbox admission/defaulting, associated Pod suspension and exact same-PVC resume under the selected controller/runtime.
- Selected StorageClass/CSI 20 GiB capacity, cleanup/Retain semantics, reliable workspace telemetry and actual disk-backed temporary storage.
- Kubernetes RuntimeDefault / configured or node-default RuntimeClass compatibility, real network/resource/credential enforcement and live NyxBot/OAuth/Agent Key/child-client journeys.
- Previously deferred independent partition-resistant physical shutdown, strong predecessor writer exclusion, universal physical erasure and controller-upgrade operations product.

No commit, push, deployment, live-environment mutation, actual workspace deletion or pending-record cleanup occurred. Only the flight-created Mongo/volume/container fixtures and generated backend incremental build cache were cleaned. Fresh images and evidence remain for review.

## Exact required commands

Commands below are the actual executed child commands; all exits were zero. `verification-runs.json` also retains the initial failed/zero-selection attempts, build/cleanup commands and requested Cargo equivalents.

**mongo-transaction-probe** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
cargo test -p nyxid --bin nyxid-server transaction_test_database_supports_atomic_writes -- --test-threads=1 --nocapture
```

**backend-remote** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
cargo test -p nyxid --bin nyxid-server remote_machine -- --test-threads=4 --nocapture
```

**backend-runtime-billing** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID/target/debug/deps/nyxid_server-19420370fa2158f0 services::billing::machine_runtime --test-threads=4 --nocapture
```

**backend-machine-funding-corrected** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID/target/debug/deps/nyxid_server-19420370fa2158f0 services::billing::funding::machine::machine_tests:: --test-threads=2 --nocapture
```

**backend-account-delete** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID/target/debug/deps/nyxid_server-19420370fa2158f0 account_deletion_policy_tests --test-threads=2 --nocapture
```

**backend-account-basis** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID/target/debug/deps/nyxid_server-19420370fa2158f0 services::admin_user_service::tests::machine_ --test-threads=2 --nocapture
```

**backend-assistant-delete-corrected** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID/target/debug/deps/nyxid_server-19420370fa2158f0 handlers::assistant_action_effects_org::wave4_effect_tests::account_delete --test-threads=2 --nocapture
```

**shared-machine** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
cargo test -p nyxid-machine
```

**updater** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
cargo test -p nyxid-machine-updater
```

**cli-managed** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
cargo test -p nyxid-cli --bin nyxid managed -- --test-threads=2
```

**cli-usage** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
cargo test -p nyxid-cli --bin nyxid node::machine::usage
```

**cli-profile-delete** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
cargo test -p nyxid-cli --bin nyxid commands::profile::tests
```

**cli-parser** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
cargo test -p nyxid-cli --bin nyxid cli::tests
```

**cli-setup** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
cargo test -p nyxid-cli --bin nyxid node::machine::setup::tests
```

**frontend-focused-source-final** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID/frontend`

```sh
npm test -- src/components/machines src/pages/machine-setup.test.tsx src/pages/machines.test.tsx src/pages/settings.test.tsx src/components/billing/billing-machines.test.tsx src/components/assistant/assistant-wave4-account-approval.test.tsx src/schemas/assistant-actions.test.ts src/lib/remote-machine-intent.test.ts src/hooks/use-machines.test.tsx src/schemas/machines.test.ts test/mobile-account-delete.test.tsx
```

**frontend-build-3** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID/frontend`

```sh
npm run build
```

**frontend-lint-1** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID/frontend`

```sh
npm run lint
```

**mobile-typecheck-1** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID/mobile`

```sh
npm run typecheck
```

**mobile-tests-1** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID/mobile`

```sh
npm test
```

**browser-1** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID/frontend`

```sh
env P4_E2E_OUTPUT_DIR=/var/folders/v4/y1w4cvsd32x9y9wxrw_98llc0000gp/T/consensus-rnd/sshx/single-current-implementation-20261015/attempt-1/playwright npx playwright test --config playwright.remote-machine.config.ts
```

**healthcheck** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
python3 deploy/remote-machine/tests/healthcheck-contracts.py
```

**renderer-export** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
sh -c 'cargo run --quiet -p nyxid-machine-updater --example sandbox_contracts > "$1"' sh /var/folders/v4/y1w4cvsd32x9y9wxrw_98llc0000gp/T/consensus-rnd/sshx/single-current-implementation-20261015/attempt-1/sandbox-fixtures.json
```

**deployment-contracts** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
sh deploy/remote-machine/tests/contracts.sh /var/folders/v4/y1w4cvsd32x9y9wxrw_98llc0000gp/T/consensus-rnd/sshx/single-current-implementation-20261015/attempt-1/sandbox-fixtures.json
```

**full-image-build** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
docker build --file cli/Dockerfile.machine --tag nyxid-machine:single-current-20261015 /var/folders/v4/y1w4cvsd32x9y9wxrw_98llc0000gp/T/consensus-rnd/sshx/single-current-implementation-20261015/attempt-1/image-build-context
```

**full-image-smoke** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
env NYXID_SMOKE_RESULT_DIR=/var/folders/v4/y1w4cvsd32x9y9wxrw_98llc0000gp/T/consensus-rnd/sshx/single-current-implementation-20261015/attempt-1/full-image-smoke sh deploy/remote-machine/tests/full-machine-image-smoke.sh nyxid-machine:single-current-20261015
```

**heca-image-build** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
docker build --file cli/Dockerfile.machine-heca-cma --build-arg MACHINE_BASE_IMAGE=nyxid-machine:single-current-20261015 --build-context cma=/var/folders/v4/y1w4cvsd32x9y9wxrw_98llc0000gp/T/consensus-rnd/sshx/single-current-implementation-20261015/attempt-1/cma-build-context --tag nyxid-machine:single-current-heca-cma-20261015 /var/folders/v4/y1w4cvsd32x9y9wxrw_98llc0000gp/T/consensus-rnd/sshx/single-current-implementation-20261015/attempt-1/image-build-context
```

**heca-image-smoke** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
sh deploy/remote-machine/tests/heca-cma-image-smoke.sh nyxid-machine:single-current-heca-cma-20261015 /var/folders/v4/y1w4cvsd32x9y9wxrw_98llc0000gp/T/consensus-rnd/sshx/single-current-implementation-20261015/attempt-1/cma-build-context
```

**rust-format-check** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
rustfmt --check --edition 2024 --config skip_children=true backend/src/handlers/account_deletion_policy_tests.rs backend/src/handlers/assistant_action_effects_org.rs backend/src/handlers/remote_machine.rs backend/src/models/remote_machine.rs backend/src/routes/remote_machine_tests.rs backend/src/services/admin_user_service.rs backend/src/services/remote_machine_adapter.rs backend/src/services/remote_machine_bootstrap_tests.rs backend/src/services/remote_machine_manager.rs backend/src/services/remote_machine_manager_tests.rs backend/src/services/remote_machine_native_tests.rs backend/src/services/remote_machine_quota.rs backend/src/services/remote_machine_reclamation.rs backend/src/services/remote_machine_runtime.rs backend/src/services/remote_machine_runtime_tests.rs backend/src/services/remote_machine_runtime_tests/journeys.rs backend/src/services/remote_machine_service.rs backend/src/services/remote_machine_service_tests.rs backend/src/services/remote_machine_usage.rs cli/src/commands/profile.rs cli/src/node/machine/setup.rs machine-updater/examples/sandbox_contracts.rs machine-updater/src/sandbox.rs machine/src/managed.rs
```

**shell-syntax** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
sh -n cli/container/machine-node-only-entrypoint.sh deploy/remote-machine/tests/contracts.sh
```

**deployment-shell-syntax** — cwd `/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`

```sh
sh -n deploy/remote-machine/tests/contracts.sh
```
