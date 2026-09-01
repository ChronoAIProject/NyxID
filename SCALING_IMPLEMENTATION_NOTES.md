# Horizontal scaling implementation notes

## Completion predicate

The implementation is complete when all ten audit findings have code, tests, and deployment coverage. Every correctness-sensitive shared state uses MongoDB. Node and MCP traffic can cross replica boundaries without buffering streaming bodies. Internal forwarding rejects unauthenticated, stale, or externally addressed requests. The backend passes `cargo build -p nyxid`, `cargo test -p nyxid`, and clippy for the touched backend targets.

## Measured scope

- Branch baseline: `0243b547` on `feat/horizontal-scaling`.
- Primary backend files: 33,838 lines across the eight files named in the audit.
- Workstreams: node routing, shared leases and stores, MCP delivery, rate limits and caps, deployment, and documentation.
- Rigor: high. The change affects authentication, replay prevention, request cancellation, WebSocket ownership, streaming, and global admission limits.

## Dependency order

1. Define Mongo coordination records and authenticated replica identity.
2. Add node ownership leases and the internal forwarding transport.
3. Route every node-bound operation through the shared dispatcher.
4. Add shared leases, replay records, dedup records, counters, and leader election.
5. Add MCP read-through persistence and cross-replica notifications.
6. Update deployment files and documentation.
7. Run focused and full verification.

## Audit status

| Item | Status | Implementation | Evidence |
| --- | --- | --- | --- |
| 1. Node WebSocket registry | Done | `466336d1`, `f1e42614`, `af57274a`, and `31dc6dd6`: fenced Mongo owner leases, ownership-aware status/routing/sweeps, authenticated private forwarding, remote revocation, streamed HTTP and duplex forwarding, exact-connection capability reads, reconnect fencing, delete-before-disconnect ordering, pre-header cancellation, and large-frame support for every node-bound operation. | `remote_proxy_cancellation_before_headers_clears_owner_work`, `remote_ws_passthrough_preserves_large_binary_frames`, `reconnect_in_same_generation_fences_old_connection`, `inactive_node_cannot_renew_connection_owner`, `proxy_cancel_arriving_before_dispatch_prevents_socket_enqueue`, the mounted billing route test, and the full 5,708-test run pass. |
| 2. OAuth refresh sweep | Done | `7481e34a`, `e7c031f5`, `992676f9`, `8e3ac6f2`, and `e4f7fe6d`: per-credential Mongo leases cover proactive sweep and both proxy refresh paths. A losing replica polls for the credential revision or lease outcome and cannot return stale data while the winner still holds the lease. Renewable lease loss cancels provider work, and stale results are fenced. | Both deterministic `*_refresh_is_single_flight_across_replicas` tests return the fresh token on both replicas with one provider request. Both `refresh_sweep_*` tests, `stale_legacy_refresh_cannot_overwrite_reauthorized_token`, all 89 `user_token_service` tests, and the full suite pass. |
| 3. DPoP replay cache | Done | `02026c57` and `d7894cbf`: Mongo first-writer insert with a unique identity and server-time TTL expiry is authoritative across replicas. | `validate_proof_rejects_replay_across_callers`, `replay_insert_is_first_writer_wins_across_callers`, TTL-index coverage, and the full suite pass. |
| 4. Event dedup cache | Done | `02026c57`, `4f37443f`, and `bb2af0dd`: fenced Mongo claim/commit/release state machine with unique event identity, server-time expiry, and TTL cleanup. | `event_dedup_claim_commit_and_release_are_atomic_and_fenced`, `expired_event_claim_cannot_be_committed`, TTL-index coverage, and the full suite pass. |
| 5. Telegram polling | Done | `7481e34a` and `e297a1e0`: one renewable Mongo leader lease owns polling; the offset checkpoint is fenced, durable, and survives takeover. | `named_lease_is_exclusive_and_release_is_fenced`, `expired_named_lease_can_be_taken_over`, `checkpoint_is_fenced_and_survives_lease_handoff`, renewal-loss coverage, and the full suite pass. |
| 6. MCP sessions and SSE | Done | `23792ee7`: Mongo read-through session validation, write-through lifecycle, cluster-wide admission, and a durable ordered notification outbox polled by the SSE holder. | All 17 MCP session tests pass, including `validation_reads_through_on_a_replica_local_miss`, `cross_replica_notifications_are_durable_and_strictly_ordered`, deletion, recovery, and shared-admission regressions; the full suite passes. |
| 7. Rate limiters | Done | `5d72d82c` and `6968b17d`: every production global, auth, per-IP, per-key, per-agent, platform-user, device, chat, event, edit, trigger, public proxy, and public MCP limiter uses Mongo atomic token-bucket or fixed-window admission. Local implementations are test-only. | All 50 `mw::rate_limit` tests plus `fixed_window_counter_never_admits_above_the_global_limit`, `token_bucket_never_admits_above_the_cluster_wide_burst`, server-time refill coverage, and the full suite pass. |
| 8. Process caps | Done | `6968b17d` and `bb2af0dd`: WebSocket passthrough and per-user SSH caps use renewable cluster-wide Mongo slots that cancel work on lease loss. `NODE_MAX_WS_CONNECTIONS` is explicitly a per-replica resource cap and atomically reserves pending authentication plus live sockets. | `renewable_slots_enforce_one_cluster_cap_across_managers`, `renewable_slot_loss_cancels_guarded_work`, `connection_reservation_is_atomic_and_spans_socket_lifetime`, and the full suite pass. |
| 9. Deployment config | Done | `eeb9f0bb`, `f5c4c335`, and `72e53f55`: two backend replicas, separate public/headless Services, downward API identity, private internal-port NetworkPolicy, one-hour streaming ingress, MCP Prefix routing and optional affinity, scale-safe Compose, and frontend WebSocket headers. | `kubeconform`: 17 valid; current `yq eval-all` and merged `docker compose config --quiet`: pass; rendered nginx 1.27 `nginx -t`: pass. |
| 10. Documentation | Done | `5d6e2908` plus the final notes update: deployment, node proxy, environment, channel gateway, cap semantics, Mongo coordination, internal routing, and operational topology now describe the implemented multi-replica system. | `git diff --check` passes; stale stateless, per-instance rate-limit, and single-replica limitation claims are absent. |

## Decisions

- Completion requires code, tests, deployment coverage, and Mongo-backed shared correctness for all ten audit items. Compilation alone does not prove the security and streaming invariants.
- Final database tests use an isolated MongoDB replica set through `NYXID_TEST_DATABASE_URL`. The transaction tests cannot use a standalone MongoDB deployment.
- The `Node` document owns the fenced connection lease. Routing reads that record before credential resolution, and a local socket is usable only when its full owner fence still matches.
- Each process uses a generation UUID, and each socket uses a connection UUID. Cleanup matches the full fence tuple so an old reader cannot unregister a replacement connection.
- Cross-replica dispatch uses operation-specific HTTP and WebSocket forwarding on a separate internal listener. HMAC authentication binds the method, path, timestamp, nonce, and body digest, while MongoDB rejects nonce replay.
- Forwarding preserves request IDs, large frames, early response streaming, duplex traffic, and caller cancellation. Delete and revoke operations change durable state before disconnecting the socket.
- MongoDB is authoritative for leases, replay claims, counters, slots, ownership, notification delivery, and event deduplication. Local state is an optional fast path only.
- Event deduplication uses a fenced claim, commit, and release state machine. A worker cannot commit an expired claim.
- Every configured production rate limit is global. WebSocket passthrough and per-user SSH caps use renewable cluster slots that cancel work after lease loss.
- `NODE_MAX_WS_CONNECTIONS` is a per-replica file-descriptor and memory cap. One atomic reservation covers both pending authentication and the live socket.
- MCP cookie affinity is a latency optimization. Correctness comes from Mongo read-through session state and the durable ordered notification outbox.
- Kubernetes uses separate public and headless Services. A NetworkPolicy allows the internal listener only between backend pods, and the public ingress never targets that port.
- Production Compose publishes only the frontend port and resolves scaled backend replicas through Docker DNS. The backend has no fixed container name or published host port.
- The implementation was split into focused node-routing, Mongo-coordination, MCP-delivery, and deployment changes so each contract could be reviewed and tested independently.

The full runtime contract is in `docs/HORIZONTAL_SCALING_ARCHITECTURE.md`.

## Design constraints

- MongoDB is the only shared data store.
- A replica identity contains a stable pod or host name plus a process generation UUID.
- A replica address is stored only for internal routing and never appears in an external error.
- Every ownership record has a lease timestamp. Compare-and-set operations fence stale process generations.
- Internal requests use a deterministic HMAC key derived from existing key material. Authentication binds the method, path, timestamp, nonce, and body digest.
- Streaming proxy responses remain streams across both hops. Caller cancellation must cancel forwarded work.
- Durable Mongo state is authoritative. Local maps remain optional fast paths only.

## Test evidence

| Check | Result | Output |
| --- | --- | --- |
| Pre-change `cargo build -p nyxid` | Pass | Finished in 3m08s; only the macOS compact-unwind linker warning and upstream `proc-macro-error2` future-incompatibility notice |
| Pre-change `cargo test -p nyxid` | Environment failure | 5,630 passed, 84 failed in 178.09s because the auto-detected MongoDB is standalone; transaction tests require a replica set and fail at `test_utils.rs:188` |
| Pre-change `cargo test -p nyxid` with replica set | Pass | 5,714 passed, 0 failed in 109.75s using `mongodb://127.0.0.1:27019/?replicaSet=rs0&directConnection=true&retryWrites=true` |
| Ownership `cargo check -p nyxid` | Pass | Finished dev profile with no touched-code warnings |
| `expired_owner_fences_a_stale_local_socket` before fix | Expected failure | Stale local map entry made an expired persisted owner dispatchable |
| `expired_owner_fences_a_stale_local_socket` after fix | Pass | 1 passed, 0 failed, 5,718 filtered out |
| `stale_reader_cannot_unregister_replacement_connection` | Pass | 1 passed, 0 failed, 5,718 filtered out |
| `connection_owner_debug_redacts_internal_address` | Pass | 1 passed, 0 failed, 5,718 filtered out |
| Node owner Mongo CAS tests | Pass | 3 passed, 0 failed against an isolated MongoDB 8.0.14 `rs0` on `127.0.0.1:27019` |
| `remote_complete_proxy_preserves_response_and_request_identity` before fix | Expected failure | Complete forwarding returned the node UUID instead of the original request UUID |
| Two-replica node forwarding tests | Pass | 5 passed, 0 failed: complete HTTP, early streaming + cancellation, SSH exec/tunnel, terminal, WS passthrough, credential ack, and remote disconnect |
| Mixed-case internal HMAC regression before fix | Expected failure | `signature_verification_accepts_mixed_case_hex` rejected a case-equivalent hex encoding because `verify()` compared the encoded strings |
| Internal HMAC tests | Pass | 4 passed, 0 failed against `rs0`, including mixed-case hex acceptance and Mongo-backed shared nonce replay rejection |
| Deterministic OAuth loser regressions before fix | Expected failure | Both modern and legacy contenders read the unchanged token while the provider response was held at a rendezvous. Each returned stale access with exactly one provider request. |
| Deterministic OAuth loser regressions after fix | Pass | Both `*_refresh_is_single_flight_across_replicas` tests return fresh access on the winner and contender with exactly one provider request |
| Node routing tests | Pass | 13 passed, 0 failed against the replica set |
| Kubernetes schemas | Pass | `kubeconform -strict -summary k8s`: 17 resources valid, 0 invalid, 0 errors, 0 skipped |
| Kubernetes YAML | Pass | `yq eval-all '.' k8s/*.yaml`: all documents parsed |
| Production Compose | Pass | Merged `docker-compose.yml` and `docker-compose.prod.yml` passes `docker compose config --quiet`; backend has no container name or published host port |
| Frontend nginx | Pass | Rendered `frontend/nginx.conf.template` passes nginx 1.27 `nginx -t` with WebSocket headers and one-hour stream timeouts |
| Deployment documentation | Pass | `git diff --check`; removed stale stateless, per-instance rate-limit, and sticky-session claims |
| Node hardening regressions | Pass | Large HTTP body and WS binary frames, reconnect fencing, inactive-owner renewal rejection, pre-header cancellation, exact local capability reads, delete ordering, and atomic socket reservation all pass against `rs0` |
| Mongo coordination regressions | Pass | 15 tests pass for leases, checkpoints, replay, fixed windows, token buckets, renewable slots, event dedup, expiry fencing, and TTL indexes |
| Rate limiter tests | Pass | 50 tests pass; production constructors use Mongo-backed stores |
| MCP replica-safety tests | Pass | 17 tests pass for read-through validation, durable ordered notifications, deletion, recovery, and shared admission |
| PR CI touched modules against `rs0` | Pass | 4 `internal_auth`, 66 config-related, and 15 Mongo coordination tests pass with no failures |
| OAuth refresh tests | Pass | 89 `user_token_service` tests pass against `rs0`, including deterministic modern and legacy cross-replica single-flight, proactive sweeps, provider failures, and stale-write fencing |
| PR CI source audit | Pass | `backend/src/config.rs` has no unsafe or `libc` access; `internal_auth.rs` has no fixed test nonce; `libc` is dev-only |
| Final `cargo fmt --all --check` | Pass | Clean after the PR CI fixes |
| Final `cargo build -p nyxid` | Pass | Finished in 45.13s; only the known macOS compact-unwind linker warning and upstream `proc-macro-error2` notice |
| Final replica-set `cargo test -p nyxid` | Pass | 5,708 passed, 0 failed in 430.23s (483.57s wall) using `mongodb://127.0.0.1:27019/?replicaSet=rs0&directConnection=true&retryWrites=true` |
| Final `cargo clippy -p nyxid --all-targets -- -D warnings` | Pass | Finished in 1m44s with no clippy warnings; only the upstream future-incompatibility notice. The code commit hook passed clippy again. |
| Final deployment syntax | Pass | Current Kubernetes YAML parses with `yq`; merged production Compose passes with its required env-file shape and validation-only password |

Final Mongo-backed verification uses an isolated local replica-set deployment
through `NYXID_TEST_DATABASE_URL`. This executes the repository's transaction
tests without skips or weakened assertions.
