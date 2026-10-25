# Remote Machine separation: final integration evidence

Branch: `feat/remote-machine-agent-separation-authorization`, based on
`80e329961` (`feat/remote-machine-pause-resume-health`). This note covers the
approved two-phase proposal, with the user's direct-new-contract override.
The original proposal, capability-reconfiguration proposal and ZIP are preserved.

The preceding integration snapshot and its broad verification are preserved
below. Formal review round1 subsequently identified the first-dispatch private
home failure (ARCH-1) and an obsolete full-image smoke assertion (TEST-1).
The later-source repair addendum records their correction and scoped rechecks;
the earlier checks are not evidence for these later source changes. Whole-goal
runtime acceptance and deployment readiness remain unclaimed until the external
gates below are met.

The subsequent 2026-10-25 local Kubernetes check used the complete Hosted image
with a current-source overlay and an explicitly approved Localhost seccomp
profile. Its nine main product stages, dual-context Commands/Files/Secure
browser/Computer checks and 48 OS identity-permission checks passed. Actual
Developer browser calls still failed with 12413, and Agent authorization
application remained pending despite successful new tool executions. The tested
LinuxKit RuntimeDefault still blocked Chromium. Full separated acceptance and
formal release-image/target-kernel/CSI acceptance therefore remain unclaimed.
Environment-specific reports, screenshots, logs and credentials are retained
locally rather than included in the source commit.

## Integration plan and ownership

The final integration worker owns only the remaining cross-module repairs and
verification; all preceding source ownership has ended. No delegation, Git
lifecycle change, deployment or running-service reconfiguration is needed.

1. Add behavioral regressions for sealed retained-volume delivery and separated
   recovery through the existing admission service. Observe their failures.
2. Complete the existing create/bootstrap/WSS contract: mint an allocation
   identity in the accepted create, bind the controller-observed original PVC
   into sealed delivery, and consume initialization authority once for an
   authenticated daemon UUID under the current Node owner and lifecycle fence.
   Install that authority in the actual parent dispatcher before reporting.
3. Bind each logical context's generation to the authenticated runtime in the
   existing admission transaction. A replacement advances once, retaining the
   context, grants and workspace identity. Concurrent/repeated admissions
   converge; old leases and login generations remain fenced.
4. Align only affected fixtures and verify final shared/backend/mounted API/WSS,
   CLI parent/restart/store, renderer/admission and frontend contracts. Every
   Rust invocation explicitly uses `CARGO_INCREMENTAL=1`; reuse caches.
5. Record requirement-to-evidence mapping, actual pass/ignore/skip counts,
   external acceptance limits and any remaining local obligations here.

No general machine reconfiguration, compatibility bridge, migration or new
authorization service is in scope. Full v4 evidence, read-only root, the four
existing capabilities, sandboxing, external grants and Billing/lifecycle fences
remain required.

## Current direct contract

`managed::BootstrapPayload` v2 now additionally requires `hosted_storage`:
`{original_pvc_uid, allocation_id}`. The accepted-create transaction persists a
server-minted UUIDv4 allocation; bootstrap obtains the original PVC UID only from
`ManagedMachine`, populated by the existing exact controller observation. It
seals both facts and binds them into exact ciphertext replay. Invalid/missing
identity fails before Node credential installation. Protected NodeConfig retains
this tuple independently from its semantic configuration report.

Managed WSS auth additionally requires `managed_daemon_id`, selected once per
connection-loop lifetime and used as the actual parent Runtime UUID. Before
constructing the runtime or touching its workspace, the CLI verifies the exact
binding, signed current window and `auth_ok.hosted_storage` grant. That grant is
`{binding, storage, runtime_id, initialize}`. The existing authenticated Node
owner and lifecycle transaction records one initializer runtime on Machine;
same-daemon reconnects converge, every replacement receives recovery-only
authority. Expiration of replay ciphertext never removes the durable allocation.
No environment, path-existence or generation/epoch inference grants allocation.
A crash before the first journal is durably initialized can require operator
investigation: a successor cannot turn absent metadata into a fresh workspace.

The parent opens the original-volume store before reporting configuration or
support. That same dispatcher handles provisioning and quarantine. A managed
status report must use the daemon UUID admitted on that exact current WSS
connection; Agent ACK cannot write machine-configuration evidence.

The existing Agent admission transaction associates each logical context with
the authenticated daemon UUID. Its existing Node/runtime, Agent assignment,
active-turn and managed lifecycle/funding writes fence recovery. On a changed
daemon it advances a separated context generation exactly once, clears only
materialized login bindings, and signs that newer generation in the ordinary
Authority. It preserves the context ID, assignment revision, tool/login grant,
retained command UID/workspace and tombstones. Repeated and concurrent admissions
converge. All-off assignments cannot admit work or reconstruct a profile. Old
runtime admissions/renewals remain denied; the Node never chooses a generation.

## Requirement and evidence map

Source paths below are relative to the repository. Test evidence is local unless
the final verification table explicitly says otherwise. The approved no-
compatibility override supersedes proposal migration, mixed-version and rollout
matrix paragraphs; required-field/version rejection is the current contract.

| Approved outcome | Implementation and meaningful evidence | Limit |
|---|---|---|
| §4/8/10.2 consistent Access/Details flags, reasons, membership, partial and all-off grants | `remote_machine_configuration::safe_view`, `machine_access_service`, shared `MachineCapabilityForm`; hosted membership/default and owner-review tests; frontend access/detail/capability tests and narrow browser cases | An all-off assignment remains assigned and grants no execution. |
| §5 three choices and versioned affirmative consent | Strict hosted handler/choices, `normalize_request`, immutable create intent; `remote_machine_service_tests`, mounted routes, creation schema/intent/component and browser tests | No old-intent acceptance or migration contract. Automatic with insufficient support stays unresolved. |
| §5 canonical complete idempotency | Server normalizes full body before digest/acceptance; frontend retains exact full immutable body/key through reload/retry, including quote/policies/Agent/profile | Changed replay is rejected before new allocation; no compatibility digest. |
| §5/6.3 every new hosted membership starts all-off under both flags | `hosted_assignment` applied to explicit, implicit NyxBot, membership and materialization paths; `all_new_hosted_assignments_are_membership_only_for_every_choice_and_flag_pair`, service/route regressions | Creation and lifecycle replay do not restore removed or revoked grants. |
| §6 first-party owner prepare/read/commit through mounted HTTP | Persisted `HumanContextAction`/`ContextReview`, strict handlers and dedicated target response, existing `configure`/outbox; eight review tests including mounted signed human/delegated negatives and concurrent commits | No client Selection, no delegated/Agent Key approval, no general reconfiguration endpoint. |
| §6 exact full scope, versioned warning, partial/all-off/login preservation | Review captures previous assignment, exact capabilities, null/empty/partial logins, deltas, policy, expiry and config target; service and UI tests | Separated narrows inherited null login scope to explicit empty; no shared-cookie/file migration. |
| §6 live/concurrent dependency fences | Commit shares Agent/Node/Machine, actor/owner/org/membership and flag epoch writes; tests cover authorization, flags toggle-and-back, profile/config, owner/policy/expiry drift and concurrent commit | Approval/commit are persisted decisions, not runtime or physical-stop evidence. |
| §4.4/5.3 configuration chain independent from Agent ACK | Strict semantic bootstrap/config v2, exact digest, independent current WSS report/evidence, target revision/runtime/epoch/connection; service and mounted HTTP→sealed bootstrap→WSS journeys | Safe UI view exposes pending/applied summary and bootstrap/evidence reason; raw connection/observation records and invented delivery substages are not exposed. |
| §5.3 actual authenticated storage producer/consumer | Accepted allocation + controller-observed PVC → sealed payload/replay → protected NodeConfig → current WSS grant → actual parent store; storage concurrency/substitution tests, mounted WSS journey, actual CLI socket negatives | Initializer consumed before journal creation: crash before durability fails closed for a successor; no empty-workspace recovery. |
| §7 Shell/File-only has complete prerequisites without Browser/Computer authority | Managed config separates infrastructure from API ceiling, image/entrypoint/runtime always prepare complete infra; shared support, service denial, parent and renderer tests | Missing Chromium/sandbox/custody evidence remains unavailable; local substrate is not complete v4 acceptance. |
| §7.1 constrained identities and unchanged hardening | Immutable 2,048-account UID/GID pool 20000–22047; image/runtime validate collisions/groups; protected store tests exercise dropped identities, exhaustion and refusal; Rust renderer/CEL tests | Read-only root, no-new-privileges, RuntimeDefault and CHOWN/SETUID/SETGID/KILL retained; no useradd at runtime. |
| §7.2 protected retained workspace/journal | `hosted_context` original PVC/node binding, exclusive lock, atomic fsync/rename, ownership/symlink/corruption checks, UID/generation/tombstones; actual hardened store and parent restart tests with dropped-UID marker | Local retained tmpfs/filesystem fixtures do not establish CSI continuity. Credentials and HOME/browser/download/tmp remain ephemeral. |
| §7.2 authorized reconstruction, stale authority denied | Admission binds Context runtime; greater signed generation required by HostedStore; concurrent/repeated separated recovery regression and parent same-generation refusal/new-generation acceptance | Old runtime/lease and quarantined authority stay denied. No Node-local generation minting or profile identity reuse. |
| §8 truthful final status and failure handling | Independent authorization/configuration/environment observations, quarantine/delivery/execution pending, `MachineApplicationStatus`; backend and UI tests for ACK-only, stale config, offline/support failure and Ready-with-Agent-failure | Pending is retained without evidence; no timeout-as-ACK, stop, refund or deletion claim. |
| §9 external clients and existing lifecycle/Billing | Existing external source/grant/ceiling fences unchanged; local external/desktop/authority and remote-machine lifecycle/funding journeys selected in final verification | Local controller/Lago/identity fixtures are not real client/controller/payment acceptance. |
| §11 engineering boundaries | Existing services/action/outbox reused, plain model fields, dedicated HTTP DTOs, Zod/hooks/useAppForm, incremental caches; scoped build/lint/format/diff checks | No branch/worktree mutation, commit/push/PR/deploy, live-service configuration or paid resources. |

The decisive production-path regressions are named explicitly so the contract
can be checked without treating a mocked success response as runtime evidence:

| Boundary | Regression in the actual producer/consumer |
|---|---|
| Durable initialization versus recovery | `services::remote_machine_bootstrap::tests::hosted_storage_initialization_is_bound_once_to_live_authenticated_daemon`: concurrent first claims, same-daemon replay, replacement recovery, ciphertext TTL expiry and stale fences. |
| Original-volume delivery | `services::remote_machine_bootstrap::tests::sealed_storage_replay_cannot_substitute_original_volume`: decrypted sealed payload and exact replay reject a substituted volume. |
| Ordinary separated recovery | `machine_access_tests::separated_runtime_recovery_advances_once_without_widening_grants`: concurrent/repeated admission, one new generation, preserved grants/context, stale runtime/renewal refusal and later all-off refusal. |
| Actual daemon startup and reporting | `ws_client::tests::managed_socket_rejects_storage_grant_before_runtime_start`, `managed_socket_reports_installed_configuration_on_authenticated_current_runtime`, and `managed_handshake_refuses_missing_or_stale_fence_before_runtime_start`: real local socket and parent startup, exact current grant/report and missing/stale authority rejection. |
| Retained dispatcher and quarantine | `managed_configuration::tests::hosted_parent_binds_report_and_store_and_rejects_unbound_quarantine`: real parent dispatcher, dropped-UID marker across daemon recreation, old-generation refusal, newer-generation reconstruction and quarantine; no Chromium is present. |
| Mounted owner flow and live fences | `machine_context_review_service::tests`: persisted HTTP prepare/read/commit, dedicated four-field target DTO, idempotent commit retry, concurrent 200/409, dependency drift and membership-only choice/flag matrix. |

## Final verification

Every Rust build/check/test below explicitly uses `CARGO_INCREMENTAL=1`,
including Cargo subprocesses and Docker stages. Existing caches are reused;
none was cleaned. `HOST_TARGET` below is
`/tmp/nyxid-hosted-substrate-target`. Linux runners reuse
`/tmp/nyxid-hosted-linux-target`. Backend DB fixtures use the available local
MongoDB replica set at `127.0.0.1:27018` with disposable UUID databases and real
transactions.

| Final-source command | Actual result |
|---|---|
| `CARGO_INCREMENTAL=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo check -j 1 -p nyxid --bin nyxid-server` | Passed, 5m57s; includes storage issuer, WSS identity binding, Context runtime/recovery and final dedicated HTTP target mapping. |
| `CARGO_INCREMENTAL=1 CARGO_PROFILE_TEST_DEBUG=0 cargo test -j 1 -p nyxid --bin nyxid-server --no-run` | Final fixture-correction build passed, 9m45s. The earlier reported 45m49s included waiting for the preceding recovery build's Cargo lock; no cache was cleaned. |
| Final linked binary, `CARGO_INCREMENTAL=1 CARGO_PROFILE_TEST_DEBUG=0 <binary> machine_context_review_service::tests --test-threads=1` | Eight passed, none ignored. Actual mounted successful prepare/read/commit and exact four-field target DTO; empty commit retry; mounted concurrent 200/409 commit; policy/flag/profile/expiry/owner/config drift; all three choices × both flags via mounted membership producer/projection. |
| Same final binary/environment, selector `remote_machine --test-threads=1` | 316 passed, none failed/ignored, 566.75s. Includes strict bootstrap/storage initialization/replay, actual mounted owner HTTP → sealed bootstrap → authenticated WSS, current configuration evidence and lifecycle/funding regressions. |
| Same final binary/environment, selector `machine_access_tests --test-threads=1` | 45 passed, none failed/ignored, 32.82s. Includes concurrent separated runtime recovery and preserved partial/all-off/login authority. |
| Same final binary/environment, selector `feature_flag_service::tests --test-threads=1` | 39 passed, none failed/ignored, 1.65s. One case also runs under the Remote Machine selector. |
| Same final binary/environment, selector `handlers::node_ws::tests --test-threads=1` | 59 passed, none failed/ignored, 3.15s. |
| Same final binary/environment, selector `node_owner_service::tests --test-threads=1` | Three passed, none failed/ignored, 0.23s. |
| Same final binary/environment, selector `handlers::machine_desktop::tests --test-threads=1` | Two passed, none failed/ignored, 1.81s. |
| Same final binary/environment, selector `machine_external_service::tests --test-threads=1` | 34 passed, none failed/ignored, 17.69s; existing external authority stays independent. |
| `CARGO_INCREMENTAL=1 CARGO_TARGET_DIR=$HOST_TARGET cargo test -p nyxid-machine --lib` | 65 passed, none ignored. |
| `CARGO_INCREMENTAL=1 CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=$HOST_TARGET cargo test -p nyxid-machine-updater --lib` | 154 passed, one existing ignored live-TUF test. |
| `CARGO_INCREMENTAL=1 CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=$HOST_TARGET cargo test -p nyxid-cli --lib --features node-proxy-test` | 218 passed, two existing environment tests ignored. Hardened Linux tests below explicitly execute their fixture-only ignored tests. |
| `CARGO_INCREMENTAL=1 CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=$HOST_TARGET cargo check -p nyxid-cli --bin nyxid --lib --features node-proxy-test` | Passed on final CLI source; no new warning. |
| `CARGO_INCREMENTAL=1 sh cli/tests/managed_runtime_integration/run.sh` | Passed on arm64 Linux: three actual parent/config/quarantine tests, two authenticated socket/report/negative tests and 20 bootstrap tests; none ignored in these selected runs. Fixture-only tests run with `--include-ignored`. Offline locked library/binary checks passed. |
| Hardened Linux fixture, `env CARGO_INCREMENTAL=1 cargo test --offline --locked -p nyxid-cli --bin nyxid managed_handshake_refuses_missing_or_stale_fence_before_runtime_start -- --nocapture` | One passed, none ignored; missing/stale authenticated managed binding cannot construct a Runtime. Same read-only root/no-new-privileges/four-capability envelope. |
| `CARGO_INCREMENTAL=1 sh cli/tests/hosted_context_store/run.sh` | 16 passed, none ignored; zero doctests. Actual dropped-UID marker/custody, original volume/journal, exclusive supervisor, ownership/symlink/corruption/missing workspace, generation/tombstone/quarantine and pool exhaustion. |
| `python3 -m unittest discover -s cli/tests -p hosted_account_pool_test.py -v` | Two passed. |
| `CARGO_INCREMENTAL=1 CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=$HOST_TARGET cargo run --quiet --locked -p nyxid-machine-updater --example sandbox_contracts` then `CARGO_INCREMENTAL=1 sh deploy/remote-machine/tests/contracts.sh <fresh-export>` | Passed: 17 actual CEL validators, three Rust-rendered objects, 282 admission cases including 192 hostile variants; kustomize/platform contracts passed. |
| Focused `npm test --` over 15 schema/intent/search/hook/assistant/machine/page files (exact list in command manifest) | 305 passed, none skipped. Frontend source unchanged by integration. |
| `npm run build`; `npm run lint`; `npx tsc --project tsconfig.machine-detail-e2e.json --noEmit` | All passed. Lint: zero errors, 33 existing warnings. Existing CSS/bundle build diagnostics retained. |
| Focused `npx playwright test --config=playwright.machine-detail.config.ts --grep 'context opt-in\|assignment survives\|owner stored review\|Machine Ready with Agent'`; creation `--config=playwright.remote-machine.config.ts --grep 'free creation'` | Six owner/status and two creation cases passed at desktop/narrow sizes. Intercepted APIs, proxy `127.0.0.1:9`; client evidence only. |
| `CARGO_INCREMENTAL=1 rustfmt --edition 2024 --config skip_children=true --check <all changed/new Rust files>`; changed shell `sh -n` and Python AST parsing; `git diff --check` | All 48 Rust files and all selected syntax/diff checks passed on final source. Original-file hashes, branch/HEAD and complete changed-file hashes are recorded in the closure manifest. |

The final backend run executed 506 selected tests: **505 distinct tests passed**,
none failed or ignored. One feature-flag test overlaps two selectors. The other
9,929 tests in the 10,434-test target were not run and are not claimed as passes.
The linked final binary is `target/debug/deps/nyxid_server-7a2e4b7b1623ab75`,
SHA256 `8c1c3f7ce1c185b89ee8a3a80c9e76ba567ef71d18e88627ca5dd56350603a51`.

The preceding diagnostic Remote Machine run had 307 passes and nine failures,
all at one manually serialized delete-recovery fixture missing required
`hosted_storage_allocation_id`. The fixture now supplies a UUID allocation and
explicit unset initializer; production validation stays strict. The final
316/316 run supersedes that diagnostic; it does not erase the earlier failure.

Machine-readable commands, exact per-suite results, duplicate accounting and
the complete branch/change/hash manifest are in the local attempt directory
`/Users/jasonwang/.sshx/6adc6a38630718022bac9b52/attempt-1/`:
`integration-verification.json`, `backend-integrated-results.json`,
`backend-final-summary.json`, `closure-checks.json` and
`branch-change-manifest.json`. `result.json` is the final integration handoff.

The three existing ignored tests above are specifically the macOS desktop
performance benchmark, its private RAM-volume capture check, and live GitHub
TUF mirror verification. They are not passes. Linux fixture-only ignored tests
were explicitly included in their applicable disposable runner. Filtered-out
unrelated tests and unexecuted full-image/cluster checks are not passes either.

Before the repairs, meaningful regressions failed as intended: sealed payload
lacked the original PVC, actual CLI WSS reporting still lacked the parent store,
and a replacement separated admission returned generation 1 instead of 2.
Two Linux binary-build attempts were killed by resource pressure (SIGKILL);
neither is a pass. The successful hardened run reduced test debug information
and used one build job, with unchanged assertions, sandbox and capabilities.
The upstream `proc-macro-error2` future-incompatibility diagnostic remains.

## External acceptance gates and scope-only observations

The selected real hosted acceptance in proposal §11.2 remains **unaccepted**:

- Current formal Chromium/CMA/Heca hosted image; two simultaneous complete-v4
  contexts with secure/developer browsers and Shell/File-only API ceilings.
- Every actual worker role's cross-context, legacy/external, credential, journal,
  browser/profile/socket custody under the intended RuntimeDefault/AppArmor and
  browser sandbox; actual arm64 and amd64 image/kernel evidence.
- Original CSI/PVC Pod replacement and pause/resume, retained marker/command UID,
  generation/tombstone continuity and temporary-state reconstruction, including
  old-login/lease refusal on those actual runtimes.
- Actual controller/workload/Node/owner/external-client authentication, physical
  stop observations, real paid run/stop and payment/Billing acceptance.

Available local Docker is arm64 and the hardened fixture deliberately has no
Chromium/CMA/Heca. The documented CMA build-context directory is absent, and no
current formal hosted-local image is available. Older images are not substituted.
Full-image smoke scripts are aligned and syntax-checked, not executed. Complete
v4 support gates remain unchanged and fail closed; no deployment is authorized.

General machine capability/default reconfiguration, old request/bootstrap/image
bridges, existing-data migration, compatibility matrices and automated recovery
from a lost original journal are outside this approved implementation. The safe
configuration summary suffices for the two-chain status contract; detailed raw
transport-stage inspection would require a separate product/API decision.

## Later-source round1 repair: ARCH-1 and TEST-1 (2026-10-24)

This bounded repair owns only CLI context preparation/runtime and targeted
regressions, the full-image smoke consumer, and the related deployment/evidence
notes. It preserves the earlier broad integration results above as their own
source snapshot. It neither assesses the separate frontend Q1 repair nor claims
formal review or whole-goal completion.

The plan was to reproduce the signed first-dispatch gate under startup-equivalent
private homes, correct descriptor pinning/ownership ordering without changing
capabilities or support requirements, align the actual smoke consumer with its
authenticated retained-store grant, then run focused regressions and retain all
external acceptance gates. Scope and engineering mechanics were already approved;
no compatibility path, branch/worktree operation or deployment was added.

ARCH-1 is confirmed in the current consumer: `context_instance` appends the
command HOME to roots and protects both browser homes before provisioning.
Startup leaves those homes owned by UIDs 1000/1001/1002 at `0700`. A supervisor
with `CapEff=CapPrm=CapBnd=0xe1` (CHOWN/SETUID/SETGID/KILL) cannot request a readable
descriptor first. The repair opens each target with `O_PATH|O_DIRECTORY|O_NOFOLLOW`,
retains the protected-ancestor and existing owner checks, takes root ownership
using `fchownat(AT_EMPTY_PATH)` on the pinned descriptor, then reads that same inode
via `openat(fd, ".")`. Existing modes and the browser's exact UID ACL are applied
through the resulting descriptor. No DAC_OVERRIDE, DAC_READ_SEARCH, FOWNER or
sandbox relaxation is introduced. Wrong owners/symlinks/writable parents still
refuse; a worker cannot chmod the resulting home gate.

The regression invokes `Runtime::handle` with signed current v2 authority, an
exact managed configuration, a signed live execution window and an installed
hosted store. Both File and Shell requests pass the actual first-dispatch home
and provisioning gates. They reach the real `browser_resources` gate and refuse
there because the disposable fixture has no Xvfb/Chromium. The test-only complete
v4 prerequisite is explicit supplied data, compiled only under `cfg(test)`; it
is not a live browser, custody or full-image acceptance result. Missing retained
store, old/incomplete evidence, Browser/Computer API denial, both browser UID ACLs,
borrowed policy-group denial, command-user access and later explicitly shared
Shell execution are checked. A separate deterministic pathname-replacement test
checks that ownership/mode work stays on the pinned inode and leaves the
replacement untouched.

The final fixture hands homes to the actual worker UIDs at `0700` and admits the
shared workspace to UID 1000 before dispatch. Its root remains read-only, NNP
remains 1, and the four-capability ceiling is asserted inside the test before
and after execution. `/home` uses an anonymous disposable Docker disk volume
removed by `--rm`: this Docker kernel rejects POSIX ACLs on tmpfs with errno 95,
while the disk volume supports the same ACL under the same hardening. This is
local filesystem evidence, not a CSI mount, RuntimeDefault/AppArmor or hosted
image acceptance result. ACL failure remains fail-closed on unsupported storage.

TEST-1 is corrected only at the actual smoke consumer. The fixture already seals
`hosted_storage` and returns its exact current authenticated WSS grant; CLI WSS
installs the retained store before status reporting. The smoke now verifies the
installed tuple and requires `separated.available` to match the actual complete
v4 evidence independently of the `shared_legacy` selection. A genuine unavailable
probe must retain a nonempty reason; the obsolete missing-storage reason cannot
mask an uninstalled supplied grant. Results retain the actual support and safe
probe evidence. No production support predicate or authority fence changed.

| Added/changed element | GoalArtifact/current-consumer basis |
|---|---|
| Descriptor ownership/read-open ordering for command and both browser homes | Success criterion 5; constraint 6; proposal §§7 and 11.2; existing `context_instance` preparation gates under minimal capabilities. |
| Signed-dispatch, access/ACL, invalid-target and pinned-inode regressions | Success criterion 6; actual `Runtime::handle` and home-protection consumers; preserve no shared fallback, pinning and ownership boundaries. |
| Test-only prerequisite data and disposable ACL-capable home fixture | Reproduce the reported first-operation trigger without weakening real v4 or ACL checks; this is explicitly limited local evidence. |
| Full-image installed-store/evidence assertion and four local consumer tests | TEST-1's current authenticated WSS producer/status consumer mismatch; criterion 6 requires meaningful current-contract regression, not an obsolete unconditional false assertion. |
| README and this later-source addendum | Criteria 1/6 and constraint 4: preserve broad integration evidence, report changed-source checks and unaccepted real-environment obligations separately. |

The local commands/results for this repair are recorded below. All Rust
build/check/test commands, including Docker Cargo subprocesses, explicitly use
`CARGO_INCREMENTAL=1`; the existing `/tmp/nyxid-hosted-linux-target` cache is reused
without cleaning. No backend, frontend, renderer/admission or paid-resource
checks are recounted as having run on this repair's source.

| Command | Actual result and scope |
|---|---|
| `CARGO_INCREMENTAL=1 sh cli/tests/managed_runtime_integration/run.sh` | Linux arm64: 2 signed-dispatch/unsafe-target tests, 3 parent/configuration/quarantine tests, 2 actual authenticated socket/grant/report tests and 20 managed-bootstrap tests passed; none ignored in these selected runs. Offline locked CLI library and binary checks passed. The two socket tests retain missing/PVC/allocation/runtime/binding grant refusals and successful exact store installation. |
| `CARGO_INCREMENTAL=1 sh cli/tests/managed_runtime_integration/run.sh /tmp/nyxid-hosted-linux-target cargo test --offline --locked -p nyxid-cli --lib --features node-proxy-test legacy_pinning_tests -- --include-ignored --test-threads=1 --nocapture` | One passed, none ignored. The pinned inode retains ownership/mode transition across a deterministic pathname replacement; the replacement inode is unchanged. |
| `python3 deploy/remote-machine/tests/full-machine-image-smoke-contract-test.py` | Four passed; full-v4 support with shared choice, mismatched/missing installed grant, obsolete storage denial, and incomplete/failed/unsupported evidence. This loads the actual consumer function without starting an image. |
| Pre-repair source overlays with the final startup-equivalent fixture and the same signed dispatch selector | Expected red: original roots/browser helpers stop at `legacy_roots`; with repaired roots but the original browser helper, stop at `legacy_browser_home`. Each run has 1 failed regression and 1 passing unsafe-target test. Overlays are disposable bind mounts; repository source is not reverted. |
| `CARGO_INCREMENTAL=1 rustfmt --edition 2024 --config skip_children=true --check` on the three repair Rust files; shell syntax, Python AST and scoped diff checks | All passed. Python AST parsing also includes the smoke consumer and its local regression module; scoped diff/whitespace checks passed. |

Diagnostic attempts remain non-passes: the first test compile used the wrong
standard-library `chown` path; early fixture attempts omitted or malformed the
signed managed execution window; another omitted admitted workspace ownership.
After those fixture corrections, real home-read denial was reproduced. Initial
post-fix tmpfs-home runs stopped on the required ACL gate (`EOPNOTSUPP`), leading
to the documented disposable disk-home fixture rather than bypassing the gate.
The old smoke assertion failed the new consumer regressions before correction.
The existing upstream `proc-macro-error2` future-incompatibility warning remains.
These diagnostics are not acceptance results and do not replace the final checks.

No current formal hosted full image was built or run. The documented CMA named
build context (`$HOME/Work/Project/aevatarAI/cma`) and `nyxid-machine:hosted-local`
image are absent; existing older images are not substituted. Real simultaneous
full-v4 contexts, actual browser/desktop/sandbox/custody observations, the selected
RuntimeDefault/AppArmor/kernel/arm64-and-amd64 matrix, original CSI/PVC replacement
and pause/resume, real owner/Node/external-client/controller auth, physical-stop
observations and real paid/Billing acceptance all remain required and unaccepted.
No running user service, external resource or paid resource was changed.

The assigned repair has no remaining required local obligations. The final scoped checks above are assignment evidence only; the external acceptance obligations remain open. The separate frontend repair result is neither inferred nor included.
