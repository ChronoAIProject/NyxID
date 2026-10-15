# Remote Machine backend Kubernetes metadata authentication — local verification

Scope: `feat/remote-nyx-machine-mvp`, worktree
`/Users/jasonwang/.codex/worktrees/remote-nyx-machine-mvp/NyxID`, unchanged HEAD
`bde170f0e5522acce16891f4824daa806b661269`. This report attributes only the
metadata-auth task; existing profile/rate and feature-flag edits are baseline.

Status: **implemented and locally verified**. All **93 top-level focused tests**
passed (plus the factory test's 14 isolated child scenarios). Real Google
identity/GKE permissions and selected-cluster acceptance remain pending.

## Docs-first evidence

Before implementation source edits, the goal/MVP and acceptance sections were
added to `REMOTE_NYX_MACHINE_MVP_PLAN.md` and
`REMOTE_NYX_MACHINE_FINAL_READ_CONTRACTS.md`. The worker recorded their content
hashes at **2026-10-15T10:56:06.538803+00:00** in the attempt's `docs-first.json`:

- MVP: `bfa7913ed8ac72d3a7742f0f9f6674272e95296e16ebf435920569efe2a9e3b5`
- Read/acceptance: `3527fcdea59d16e4bff61f70effdc1eb8a0039d09e727f7e5987694eb8170510`

Deployment guidance and `ENV.md` document exact mode values, requirements,
limits, failure behavior and identity/permission distinctions. No approved-plan
or goal deviation was needed.

## Implementation and preserved boundaries

- `NYXID_REMOTE_MACHINE_KUBE_AUTH_MODE` is unset-default `token_file`, or explicit
  `google_metadata`. Blank/padded/unknown modes reject configuration. File mode
  requires an absolute protected file and reads each request; metadata mode
  never consults it. No source fallback or unauthenticated execution is added.
- `machine-updater::kubernetes_credentials` is a closed concrete source with
  zeroizing secrets and redacted Debug/errors. It consolidates protected file
  reads, including projected symlink rotation. Static/file Sandbox constructors
  remain supported.
- Backend `remote_machine_kubernetes_auth` owns API/CA/environment selection and
  a process-lived metadata source. Both production factories clone that source,
  including repeatedly rebuilt quota, telemetry and admin verifier consumers.
  The bootstrap verifier remains retained in AppState; no AppState migration.
- Metadata uses the fixed Google GET/path and `Metadata-Flavor: Google`, with
  direct no-proxy/no-redirect transport, 64 KiB response and 16 KiB token limits,
  integer TTL 1–86400 seconds, 1-second connect, 3-second request and 5-second
  total acquisition limits. Conservative monotonic lifetime starts before the
  request; early refresh is min(60 seconds, TTL/10). Refresh is single-flight
  with recheck and a one-second failure cooldown. Waiting is bounded; early
  refresh failure also fails closed, without stale credentials.
- Both clients send sensitive Bearer headers to the actual HTTPS Kubernetes
  API. Certificate and hostname verification remain enabled. The focused test
  found that reqwest/rustls accepts PEM containing no certificates; configuration
  now parses a bundle and requires a nonempty result before building the client.
- Configuration readiness is selected-mode aware. Existing asynchronous
  storage/provisioning and operator reads exercise actual credentials/API
  authorization. No-file metadata configuration is not a missing-file blocker.
- Original Pod Kubernetes tokens remain in TokenReview with audience
  `nyxid-remote-machine-v1`. Google tokens only authorize the Kubernetes HTTP
  request. Pod/Sandbox UIDs, ownership, runtime generation, credential epoch,
  writer fence, grants, native lifecycle and Billing gates are preserved.
- The fixture constructor is gated by `cfg(test)`/`test-support`; the normal
  backend dependency graph has no `test-support` feature. No production metadata
  endpoint environment override, generic auth registry, sidecar/exporter,
  global proxy/SSRF policy exception or KubeletCollector credential change.

## Fresh local verification

| Fresh final group | Result | Scope |
|---|---:|---|
| Shared credentials + Sandbox adapter | 47 passed, 0 failed | 8 credential tests and 39 Sandbox regressions |
| Backend metadata configuration/factories | 2 passed, 0 failed | 14 isolated factory scenarios, both rebuilt clients, one shared metadata fetch, TLS/CA/readiness and original Pod TokenReview token |
| Backend regressions | 44 passed, 0 failed | 12 bootstrap service, 3 Kubernetes identity, 9 native lifecycle, 20 URL validation tests |
| Scoped rustfmt / git diff whitespace / production feature graph | Passed | Normal backend dependency graph has no fixture feature |
| Preservation | Passed | 73 baseline entries checked; all 21 historical reports unchanged |

Exact test commands (repository root):

```sh
CARGO_INCREMENTAL=0 cargo test -p nyxid --bin nyxid-server \
  --config 'profile.test.package.nyxid.debug=0' kubernetes_auth -- --nocapture

CARGO_INCREMENTAL=0 cargo test -p nyxid-machine-updater --lib -- \
  kubernetes_credentials::tests sandbox::tests --nocapture

NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:59551/?directConnection=true&replicaSet=metadata-test' \
  target/debug/deps/nyxid_server-8f70fbec97f5e7c5 \
  services::remote_machine_kubernetes_identity::tests \
  services::remote_machine_bootstrap::tests \
  services::remote_machine_manager::native_tests \
  services::url_validation::tests --nocapture --test-threads=2
```

The final backend build emitted **35 existing warnings** and no compilation
errors. Final executable SHA256:

- Backend `nyxid_server-8f70fbec97f5e7c5`:
  `985526bb070377d57ecd6ee4923de5a079d2ca8194ba677d375f2bc817cad679`
- Updater `nyxid_machine_updater-47c471cccc57d6e0`:
  `301f5fb783c3fb1df5f12a9cb2776105d3d799d575de41d52da50d8d77c295d6`

The final snapshot covers the 11 implementation/test/build files; SHA256 of its
sorted compact JSON `sha256` mapping is `32602d28e9631acf24b09cbd03d109b23742e4f62823d6c8e1ba57428c76fa40`. Source bytes were
checked against it after tests. Full commands, logs, snapshot and preservation
checks are in the worker-owned attempt artifacts. The task-owned MongoDB
container and its anonymous volumes were **removed after tests**, after checking
its task label.

Log reference:
`/var/folders/v4/y1w4cvsd32x9y9wxrw_98llc0000gp/T/consensus-rnd/sshx/nyx-meta-20261015-implement/attempt-1/worker.stdout.log`

The initial pre-change readiness regression failed on
`manager_credential_file_unavailable`, establishing the missing capability.
One intermediate backend compilation was cancelled after correcting a fixture
that omitted the existing `nyxid.dev/managed=true` requirement. The first
completed implementation matrix passed metadata/file/default and invalid-mode
cases, then exposed the empty-CA validation defect (`bad-ca`). The final run
passed that same assertion after the CA fix. These attempts are not counted
as passing final evidence. Existing compiler warnings are reported separately;
no broad workspace/frontend/image suite was run.

## Real components and doubles

Real Rust: the shared credential source/cache, environment connection factory,
Sandbox API, Kubernetes workload verifier, protected file reads, TLS client,
readiness, TokenReview construction and workload validation. Backend tests
rebuild both production factories in isolated child processes; they do not
mutate the parent environment. An unusable proxy tests the metadata bypass;
trusted/untrusted CA and hostname cases exercise real TLS verification.

Doubles: loopback HTTP metadata and generated-CA TLS Kubernetes endpoints;
synthetic access/Pod tokens and Kubernetes responses. Local credential timeout
fixtures shorten the production timeouts to keep failure tests fast. These do
not contact Google's metadata server or a target GKE API. Existing native
lifecycle fixtures emulate Agent Sandbox reconciliation; bootstrap service
fixtures disclose their trusted identity double, while the TokenReview
transport test exercises the real workload verifier. Transaction tests use a
new task-owned MongoDB **8.0.32** replica set `metadata-test`, container
`nyx-metadata-auth-tests-20261015`, loopback port **59551**, with explicit
`NYXID_TEST_DATABASE_URL`; no in-use Mongo is selected. Cleanup status is in the
final result block.

## Exact task file inventory

New implementation/test files:

- `backend/src/services/remote_machine_kubernetes_auth.rs`
- `backend/src/services/remote_machine_kubernetes_auth_tests.rs`
- `machine-updater/src/kubernetes_credentials.rs`
- `machine-updater/src/kubernetes_credentials_tests.rs`

Modified existing implementation/build files:

- `backend/Cargo.toml` (test-only shared-source fixture feature)
- `backend/src/services/mod.rs` (private module registration)
- `backend/src/services/remote_machine_kubernetes_identity.rs`
- `backend/src/services/remote_machine_manager.rs`
- `machine-updater/Cargo.toml` (existing zeroize serde feature; fixture feature)
- `machine-updater/src/lib.rs`
- `machine-updater/src/sandbox.rs`

Documentation:

- `docs/REMOTE_NYX_MACHINE_MVP_PLAN.md`
- `docs/REMOTE_NYX_MACHINE_FINAL_READ_CONTRACTS.md`
- `docs/ENV.md`
- `deploy/remote-machine/README.md`
- This single verification report.

`Cargo.lock` is unchanged. Of 73 pre-existing dirty/untracked files, only the
four overlapping scoped files (manager, Sandbox adapter and the two goal/read
contracts) receive additive task changes; all other baseline bytes remain
unchanged. All **21 historical untracked Markdown reports** are preserved
byte-for-byte. Baseline-relative diffs retain the prior profile/rate and flag
work in those overlapping files.

## Environment acceptance still pending

An operator must verify the actual authenticated Google `chrono-nyxid`
principal, its least-privilege Kubernetes permissions, target API reachability,
CA/hostname and actual metadata issuance/refresh in the target deployment.
Binding Kubernetes ServiceAccount `nyxid-machine-manager` does not authorize a
Google principal by itself. No IAM/RBAC grant, cluster/service configuration
change, deployment/restart, image build, commit or push was performed.

Physical stop bounds, storage writer isolation and real selected-cluster
acceptance remain deferred under the existing MVP contract. Local passing
fixtures establish this credential integration only, not whole-MVP release or
real GKE authorization.
