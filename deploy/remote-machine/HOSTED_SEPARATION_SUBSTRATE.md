# Hosted separation substrate handoff

Historical finite-worker handoff. Its next-handoff obligations are accounted for
in [the final integration evidence](../../docs/REMOTE_MACHINE_SEPARATION_INTEGRATION_EVIDENCE.md);
its test results retain their original source and environment scope.

Scope: proposal §§7.1–7.2 and §11.2; runtime worker owns only `cli/`,
`machine-updater/`, and `deploy/remote-machine/`. Backend/shared `machine/`
contracts belong to another worker. This note is a plan and evidence record,
not hosted runtime acceptance or authorization to deploy.

## Implementation plan (recorded before code)

1. Add behavior tests for an immutable, bounded image account supply: exact
   UID/GID/name metadata, no duplicate/rebound identities, explicit exhaustion,
   and rejection of malformed supply. Implement pool generation and validation
   in `cli/container/` and the context runtime. Hosted provisioning must never
   run `useradd`; native provisioning retains its established mechanism.
2. Add store recovery tests for original-volume identity, retained marker and
   UID/generation/tombstones, missing journal/workspace, ownership/symlink
   corruption, exclusive locking, and ephemeral-runtime reconstruction.
   Implement a supervisor-owned hosted store with atomic fsync/rename state,
   durable initialization/provisioning records, and fail-closed recovery.
   Accept the volume binding explicitly; no unauthenticated caller path or
   default empty-volume adoption. Keep node credentials outside retained data.
3. Extend command sandbox directory pinning to accept retained workspace plus
   ephemeral HOME/tmp without broad filesystem grants. Prepare full browser
   infrastructure independently from Browser/Computer API ceilings and retain
   all existing complete v4 evidence requirements.
4. Test the actual `SandboxIntent` renderer and admission CEL before changing
   their storage shape. Mount the single original PVC at a protected retained
   root and expose only its legacy subdirectory at `/workspace`; retain identity
   Memory emptyDir, ephemeral HOME/tmp, read-only root, no privilege escalation,
   RuntimeDefault and exactly CHOWN/SETUID/SETGID/KILL. No migration path.
5. Run owned store/identity/runtime/renderer/admission regressions, local
   incremental Rust builds, and shell/image contract checks. Every Rust
   compilation explicitly sets `CARGO_INCREMENTAL=1`, including image stages.
   Record actual failures, environment limits and next-contract obligations.

## Required next handoff

The backend worker has not finalized new bootstrap/config/report semantics.
The next runtime integration must pass an authenticated original PVC UID and
runtime identity to the hosted store, install the approved separation choice,
bind configuration revision/digest to authenticated current-generation reports,
and keep Agent authority ACK distinct from machine configuration evidence.
No fabricated evidence or native/Docker result can satisfy formal Pod probes.

The runtime integration must install `Store::open_hosted` into the parent's
context dispatcher before accepting separated execution. Both an authenticated
managed binding and managed entrypoint environment prevent use of the native
store as a fallback. The current handoff deliberately does not claim bootstrap
installation: the backend/shared semantic contract is owned elsewhere.

Use the live daemon runtime UUID, not a persisted credential epoch, as the
store's runtime identity. A recovered context needs an authorized generation
greater than its previous generation before recreating browser profiles. The
store refuses same-generation recreation on another runtime; the next backend
handoff must supply that authorized generation and retain the independent
machine-configuration pending state until actual application evidence matches.

## Implemented layout and bounds

- Image identities: UID/GID 20000–22047 inclusive (2048 total), locked passwords,
  nologin shells, no pre-created HOME, exact names and account metadata.
  Allocation remains monotonic; exhaustion, collision or account rebuild failure
  refuses execution. The existing 128-context/128-generation maxima remain
  upper bounds; the shared finite identity supply can exhaust earlier.
- One original PVC: its root mounts at `/var/lib/nyxid-contexts`; only `legacy`
  mounts at `/workspace`. Its `metadata/` is root-only, with a versioned,
  node/PVC-bound journal and supervisor lock. `contexts/<command UID>/workspace`
  sits behind a supervisor-owned role gate. Pending creation is journaled before
  directories are created; committed missing workspaces are never recreated.
- Node auth/signing/bootstrap, command HOME/tmp, and generation-specific secure
  and developer browser state stay under protected ephemeral identity/HOME/tmp
  mounts. The command sandbox pins the retained workspace separately from its
  ephemeral HOME/tmp; it receives no broad retained-root rule.
- Shell/File-only managed configurations prepare browser/desktop infrastructure
  while keeping Browser and Computer ceilings false. Complete existing v4
  probes, renderer sandbox, API permission checks, and credentials custody gates
  are preserved. Infrastructure installation is not successful v4 evidence.

## Local evidence collected

`CARGO_INCREMENTAL=1` was set explicitly for every Rust build, check and test,
including Docker-run checks and the hosted Dockerfile's build commands. No
cache was cleaned. All Rust source edits passed rustfmt; shell syntax and
`git diff --check` passed.

| Command / surface | Actual result and limit |
|---|---|
| `python3 -m unittest discover -s cli/tests -p hosted_account_pool_test.py -v` | 2 passed; bounded locked supply and pre-write UID/name/primary-group collision rejection |
| `sh cli/tests/hosted_context_store/run.sh` | Rebuilt arm64 test image with the actual generator; 16 passed, none skipped, serial execution. Read-only Docker root, no-new-privileges, exactly CHOWN/SETUID/SETGID/KILL. Actual dropped-UID file access, command seccomp/Landlock, retained cwd, ephemeral HOME/tmp, sibling denial, original-volume binding, corrupt/missing/symlink/ownership recovery, generation quarantine and exhaustion. Filesystem reopen is not Pod/CSI replacement |
| `CARGO_INCREMENTAL=1 CARGO_TARGET_DIR=/tmp/nyxid-hosted-substrate-target cargo test -p nyxid-cli --lib --features node-proxy-test` | 217 passed, 2 existing ignored; macOS library runtime/authority/custody regressions, not hosted execution |
| `CARGO_INCREMENTAL=1 CARGO_TARGET_DIR=/tmp/nyxid-hosted-substrate-target cargo test -p nyxid-cli --bin nyxid node::managed_runtime::tests` | 17 passed, none ignored; sealed managed bootstrap/restart and infrastructure contract regressions |
| `CARGO_INCREMENTAL=1 CARGO_TARGET_DIR=/tmp/nyxid-hosted-substrate-target cargo test -p nyxid-machine-updater --lib` | 154 passed, 1 existing ignored live TUF test |
| Rust `sandbox_contracts` example + `sh deploy/remote-machine/tests/contracts.sh /tmp/nyxid-hosted-sandbox-fixtures.json` | 17 actual CEL validators, 282 admission cases, including 192 hostile variants; actual rendered PVC/subpath shape, protected mounts and unchanged security limits. No Kubernetes mutation |
| Docker `CARGO_INCREMENTAL=1 cargo check --offline --locked -p nyxid-cli --lib --features node-proxy-test` | Linux arm64 passed; existing upstream future-incompatibility warning for proc-macro-error2 2.0.1 |
| Docker `CARGO_INCREMENTAL=1 cargo check --offline --locked -p nyxid-cli --bin nyxid` | Linux arm64 passed; existing upstream future-incompatibility warning for proc-macro-error2 2.0.1 |

The new renderer and Shell/File infrastructure regressions were observed failing
before their fixes. Initial parallel filesystem tests exposed inherited lock
descriptors during process spawning; explicit orderly unlock fixed recovery.
An imported existing 200 ms process-probe test timed out once under concurrent
compilation; serial reruns, including the final image run, passed all 16 tests.
No production timeout, sandbox or capability was relaxed to pass tests.
The managed-runtime run also found a restart test fixture that manually built
the former infrastructure-less configuration. It now constructs the same
sealed infrastructure configuration as production; all 17 tests then passed.

The rebuilt test image is a Rust/account/filesystem fixture, not the full hosted
Chromium/CMA/Heca image. Full hosted image build and browser v4 probes remain
unverified in this handoff. No DB, controller, CSI, billing, real-client or paid
resource acceptance is claimed.

## Acceptance still requiring the selected environment

Two concurrent full-v4 contexts with Shell/File ceilings; every actual role's
cross-context/legacy/credential/socket access; RuntimeDefault/AppArmor browser
sandbox observability; arm64 and amd64; CSI original-PVC replacement and
pause/resume continuity; stale lease/login rejection; actual controller,
Node/client authentication, stop observations and billing. No production,
hosted cluster or paid-resource operation is authorized in this assignment.
