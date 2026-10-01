# Machine nodes integration with main

This integrates machine-node commit `e90eefc96f26b7dee99de1ad6a94b85684aa20f1`
with main `be1883bd72d4ef4deb91207678decb6f754a1a16` (0.39.0). The merge is
left staged and uncommitted. Main's version values are retained.

## Conflict resolutions

| File | Resolution and semantic decision |
|---|---|
| `.github/workflows/ci.yml` | Retain main's resource preparation/coverage jobs and skill-script filters, plus machine crate filters, extension freshness and container e2e jobs. |
| `backend/src/handlers/admin_nodes.rs` | Report live-owner HTTP signature, upload and HTTP cancellation capabilities together. |
| `backend/src/handlers/assistant_action_effects_nodes.rs` | Preserve automation behavior and add defaulted machine fields to node fixtures. |
| `backend/src/handlers/mod.rs` | Register both machine/saved-login and automation/pool/consent modules. |
| `backend/src/handlers/node_admin.rs` | Retain machine settings/capabilities and main's node capability fields; use the shared first-party human guard. |
| `backend/src/handlers/node_ws.rs` | Keep machine binary/control/upload handling and HTTP cancellation capability reporting. |
| `backend/src/handlers/nyxbot.rs` | Retain machine setup/control watches and automation trigger-created watches. |
| `backend/src/handlers/proxy.rs` | Preserve priority/AI pool routing, billing and cancellation alongside machine gateway/git/streaming ingress; prohibit machine upload replay or pool entry. |
| `backend/src/models/mod.rs` | Retain all machine, login, consent, pool and automation models. |
| `backend/src/models/node.rs` | Preserve defaulted machine profile/settings and both upload and HTTP cancellation capabilities. |
| `backend/src/services/assistant_acknowledgement_service.rs` | Combine machine/login grants with per-run webhook policy, trigger-bound decisions and transactional scheduler wakes. |
| `backend/src/services/assistant_authority_tests.rs` | Fixtures retain machine/login grants and confirmation-policy fields. |
| `backend/src/services/assistant_live.rs` | Decode and route machine/setup/desktop events and trigger-created events through the shared change stream. |
| `backend/src/services/credential_push_service.rs` | Retain correlation/cancellation/upload capabilities in credential-push fixtures. |
| `backend/src/services/destination_routing_tests.rs` | Keep destination routing coverage with both capability families and machine fields. |
| `backend/src/services/google_auto_activation_tests.rs` | Keep activation coverage with both capability families and machine fields. |
| `backend/src/services/mod.rs` | Retain machine services plus automation, consent and failover services. |
| `backend/src/services/node_dispatch.rs` | Keep pool cancellation/stream semantics and signed cross-replica machine requests/uploads/desktops; wrap streams in main's cancellation-aware type. |
| `backend/src/services/node_dispatch_tests.rs` | Retain pool and machine cross-replica tests with combined capability fixtures. |
| `backend/src/services/node_owner_service.rs` | Persist both capability families and machine profiles under the same ownership fence. |
| `backend/src/services/node_routing_service.rs` | Preserve main's routing/failover behavior; fixtures gain machine fields and upload capability. |
| `backend/src/services/node_ws_manager.rs` | Combine HTTP cancellation with bounded machine/upload/desktop paths; absent capabilities continue to deserialize to false. |
| `cli/src/commands/node.rs` | Retain machine image/profile lifecycle and sandbox flags; validate custom CA settings before any restart and pass CA mounts to both images. |
| `cli/src/node/proxy_executor.rs` | Use main's fallible shared-TLS destination client for selected targets and streamed git requests. |
| `cli/src/node/ws_client.rs` | Preserve shared TLS, machine TCP_NODELAY/frames and HTTP cancellation; register streamed HTTP uploads for cancellation through response completion. |
| `docs/chat/08-nyxagent-engine.md` | Retain machine and automation engine contracts; explain combined confirmation and authority. |
| `docs/chat/09-nyxbot-orchestrator.md` | Retain machine setup/control/login tools and automation tools; explain triggered machine turns. |
| `frontend/src/components/assistant/nyxbot-agent-details.tsx` | Render both machine/login grants and agent automations. |
| `frontend/src/components/ui/textarea.tsx` | One React 19 component accepts forwarded ref props, shared styling and invalid-state styles for both callers. |
| `frontend/src/pages/admin-usage.router.test.tsx` | Preserve main's awaited router-load/import sequencing and machine-inclusive router coverage. |

## Integration behavior and regression tests

Webhook machine calls are checked after live grants, owner-control state and
canonical arguments. A digest-bound, one-use owner card satisfies webhook and
machine/per-login confirmation together. Scheduled turns have no webhook policy;
they retain the normal machine confirmation setting. Exec, writes/attachment
saves, job cancellation and mutating computer actions are destructive because
they can overwrite data or interrupt arbitrary work. Checked login filling and
requesting owner control are changing but not destructive.

| Boundary | Regression evidence |
|---|---|
| Webhook changing machine call and exact one-use approval | `machine_webhook_changes_share_one_exact_card_with_machine_confirmation` covers both `machine_confirm=none/changes`, trigger binding, changed arguments, replay and universal-tool dispatch. |
| Scheduled execution and owner control | `machine_scheduled_run_without_confirmation_executes_and_keeps_owner_control` authenticates the real chat key against a trigger-bound turn, executes with `machine_confirm=none`, then verifies owner takeover refusal. |
| Read-only webhook calls and specialist authority | `machine_read_only_tools_pass_webhook_gate_and_specialist_grants_stay_required` covers all five requested tools and the live specialist grant requirement. |
| Changing versus destructive | `machine_webhook_login_and_control_are_changing_but_not_destructive` covers both webhook policies, exec, login fill and owner-control request. |
| First-party desktop upgrade | `machine_desktop_upgrade_requires_owner_human_and_same_origin` mounts the OAuth rejection layer and tests session/access-token upgrades, OAuth/API-key denial, foreign owners and wrong origins. Existing machine authority/route-denial tests cover guests and delegated reads. |
| No streamed node retry | `machine_stream_upload_never_retries_a_fallback_node` covers failure before dispatch and a dispatched credential failure with a dispatchable fallback. |
| No streamed pool retry | `machine_gateway_stream_cannot_enter_pool_or_replay_on_another_member` refuses the gateway ingress before polling its body or contacting either pool member, even when ambiguous retries are enabled. |
| Streamed upload cancellation | `machine_upload_cancellation_closes_provider_after_body_is_complete` verifies actual upstream transport closure before headers and during an SSE response. |
| Machine Docker TLS and sandbox arguments | `machine_docker_restart_validates_ca_before_touching_existing_container` and `machine_docker_creation_preserves_ca_mounts_and_sandbox_profile` verify pre-mutation validation, CA mounts, sandbox flags and persistent volumes. |
| Common MCP router stack | The machine adapter future is boxed only on machine calls; `mounted_chat_service_edits_record_verified_actor_and_separate_request_groups` covers the mounted route that exposed the combined branches' larger stack usage. |
| Cross-replica streams and normal pools | Existing machine upload/desktop relay, pool runtime/AI/billing and node cancellation suites run unchanged on the merged tree. |

## Validation environment

Rust checks use 1.98.1, `CARGO_INCREMENTAL=0`, and one build job. Backend tests use
two test threads and a private MongoDB 8 replica set on **27024**:
`NYXID_TEST_DATABASE_URL=mongodb://127.0.0.1:27024/?directConnection=true`.
Ports 27020 and 27022 are untouched. The sole excluded backend test is the known
`curation_concurrent_writers_and_shared_budget` hang; performance tests retain
their normal ignored status and are run explicitly afterward. Frontend tests use
two workers. Logs are under `target/machine-merge-*`.

## Completed browser and frontend checks

- Frontend: **4,057 tests passed**, 402 files, 135.08s (`machine-merge-frontend.log`).
- Lint: **0 errors**, 29 existing warnings (`machine-merge-lint.log`).
- Type-check: `npx tsc -b` passed (`machine-merge-tsc.log`).
- Production build: passed, including credential-accept and mock-footprint checks (`machine-merge-build.log`).
- Extension unit/signature/deterministic package freshness: **5 passed** (`machine-merge-filler.log`), also rerun while building the e2e image.
- CI resource-script unit tests: **19 passed** (`machine-merge-ci-scripts.log`).
- Production Linux arm64 image and container e2e: **passed** (`machine-merge-image.log`, `machine-merge-e2e-build.log`, `machine-merge-e2e.log`).

The production image manifest is
`sha256:d2dc42e0c60b93f35f50c5b16a02bdfed457b275d0954d46a868b254253e4a7a`;
the e2e image is
`sha256:e24c1bea02146eeaf83cf0ab9dc7a926b9aaee097e4ecef11fed33dabbbdbbc8`.
The real browser test verifies renderer sandboxing, agent NoNewPrivs, signed
extension installation, secret isolation/scrubbing, login typing, takeover,
file transfers and hand-back. At 1280×800 it measured:

| Measurement | Merged production container |
|---|---:|
| Idle desktop payload | **0 bytes/s**, 0 changed frames/s |
| Typing | **30.08 fps**, 209,245 bytes/s |
| Scrolling | **29.14 fps**, 2,026,538 bytes/s |
| Owner input-to-frame | **29.685 ms p50 / 53.024 ms p95** |
| Takeover with blocked agent work | **1.191 ms** |
| 4 MiB file round trip | **120.720 ms** |

This is a shared macOS development host running a Linux arm64 Docker Desktop
container. A host Rust build was active during this container check. macOS live
desktop capture still requires the owner's Screen Recording/Accessibility
permissions; the exact repeatable owner command remains in
[MACHINE_NODES_VALIDATION.md](MACHINE_NODES_VALIDATION.md).

The final CLI/shared-machine run passed **1,435 tests**, with 2 existing ignored
benchmarks, across 14 test/doc-test targets (`machine-merge-cli-final2.log`).
Its unit tests include the real upload-cancellation transport check; the Docker
integration tests verify both ordinary and machine images. Focused machine
validation passed **41 tests**, with the signed-dispatch benchmark ignored for
its separate timing run (`machine-merge-focused-final.log`). The mounted MCP
stack regression passed with the default stack size
(`machine-merge-stack-regression.log`).

## Complete backend chunks

| Chunk | Final result | Test duration | Log |
|---|---|---:|---|
| `handlers::` | **2,103 passed**, 0 failed | 472.07s | `machine-merge-handlers-final.log` |
| `services::` | **3,907 passed**, 0 failed, 5 ignored benchmarks | 566.67s | `machine-merge-services.log` |
| Remaining tests (`--skip handlers:: --skip services::`) | **1,140 passed**, 0 failed | 48.76s | `machine-merge-rest.log` |

Chunk filters can overlap; their sum is not a unique-test count.
These include the new automation/machine tests and main's pool AI/priority,
stream cancellation, scheduler, incremental-consent and human-auth coverage.
The handler chunk was rerun in full after fixing the mounted MCP stack overflow;
no stack-size override was used. Initial compile/test-fixture failures were
corrected before these final runs. Rust compilation emitted the macOS linker's
large-debug-unwind-section warning; Cargo also reported the existing
`proc-macro-error2` future-incompatibility notice. Neither is a test failure.

Reproduction (create an isolated replica set before using the test URL):

```sh
export NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27024/?directConnection=true'
export CARGO_INCREMENTAL=0
cargo +1.98.1 test -j 1 -p nyxid --bin nyxid-server -- handlers:: --skip curation_concurrent_writers_and_shared_budget --test-threads 2
cargo +1.98.1 test -j 1 -p nyxid --bin nyxid-server -- services:: --skip curation_concurrent_writers_and_shared_budget --test-threads 2
cargo +1.98.1 test -j 1 -p nyxid --bin nyxid-server -- --skip handlers:: --skip services:: --skip curation_concurrent_writers_and_shared_budget --test-threads 2
cargo +1.98.1 test -j 1 -p nyxid-cli -p nyxid-machine
cargo +1.98.1 clippy -j 1 --workspace --all-targets -- -D warnings
cargo +1.98.1 fmt --all -- --check
node --test cli/tests/machine_filler.test.mjs
```

Final workspace/all-target Clippy **passed** with Rust 1.98.1 and `-D warnings`
(`machine-merge-clippy-complete.log`, 5m48s including Cargo lock wait). Final
`cargo fmt --all -- --check` and staged/unstaged whitespace checks also passed.

## Four timing checks on the merged tree

Run sequentially after all builds and functional suites completed, using their
compiled backend test binary to avoid another dependency rebuild. Each check
passed. As with the earlier measurements, these are local debug-build results
on a shared development host, not sustained network-capacity guarantees.

| Check | Merged-tree measurement | Log |
|---|---|---|
| Actual CLI exec over loopback WS, 100 samples, command runtime excluded | **7.758 ms p50 / 31.560 ms p95** (≤50 ms p95 target) | `machine-merge-timing-exec.log` |
| Streaming gateway and real smart-HTTP git | 100 MiB: direct **0.199s / 501.67 MiB/s**, gateway **0.185s / 540.54 MiB/s**; 12 MiB clone: direct **0.662s**, gateway **0.696s**; clone/fetch/pull/push verified | `machine-merge-timing-gateway.log` |
| Capability report to durable NyxBot wake via change stream | **16.94 ms** (<10s target) | `machine-merge-timing-setup.log` |
| In-process signed exec dispatch, 100 samples | **2.843 ms p50 / 3.175 ms p95** | `machine-merge-timing-dispatch.log` |

The individual test durations were 2.58s, 7.27s, 0.54s and 0.91s, respectively.
The 100 MiB direct/gateway variation is within local benchmark noise; it does
not imply an inherent throughput improvement from proxying. Repeat with:

```sh
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27024/?directConnection=true' \
  target/debug/deps/nyxid_server-846e5cc10b95c0c8 machine_loopback_exec_performance --ignored --nocapture --test-threads 1
# Same binary/options, replacing the filter with:
# machine_gateway_streaming_and_git_performance
# machine_exec_dispatch_performance
# machine_setup_change_stream_wakes_thread_without_sweep (omit --ignored)
```

The remaining frontend/container commands were:

```sh
# In frontend/:
npm test -- --maxWorkers=2
npm run lint
npx tsc -b
npm run build
# From the repository root:
docker build -f cli/Dockerfile.machine -t nyxid-node-machine:merge-review .
docker build --build-arg MACHINE_IMAGE=nyxid-node-machine:merge-review -f cli/tests/Dockerfile.machine -t nyxid-machine-e2e:merge-review .
docker run --rm --name nyxid-machine-merge-e2e --shm-size=256m --security-opt seccomp=cli/resources/machine-container/seccomp.json nyxid-machine-e2e:merge-review
```

## Final worktree and cleanup

All 30 conflicts and the integration fixes are staged. `MERGE_HEAD` remains
present for the user's merge commit; no commit, push, abort or version edit was
performed. The version remains main's **0.39.0**.

The dedicated `nyxid-machine-merge-mongo` container on port 27024 was removed
with `docker rm -f -v`; e2e used `--rm`, leaving no test containers or anonymous
volumes. Only this worktree's obsolete incremental data and 246 temporary Rust
object files (1.95 GiB) were removed. The two machine-specific Docker cache
mounts were pruned (1.68 GB); the tested image tags remain for review. Final
free space was approximately **33 GiB**. Other worktrees and their MongoDB
containers were left intact. Nothing required by this integration remains
unfinished.
