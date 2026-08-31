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
| 1. Node WebSocket registry | In progress | Fenced Mongo owner claims, renew/release, capability snapshot, crash expiry cleanup, ownership-aware routing/status, and connection UUID teardown are implemented. Internal forwarding and remote revocation remain. | Routing regression and local fence tests pass; Mongo CAS tests await replica-set recovery. |
| 2. OAuth refresh sweep | Pending | Per-key Mongo lease for sweep and proxy refresh | Pending |
| 3. DPoP replay cache | Pending | Mongo unique insert and TTL expiry | Pending |
| 4. Event dedup cache | Pending | Mongo unique insert and TTL expiry | Pending |
| 5. Telegram polling | Pending | Mongo leader lease with renewal and takeover | Pending |
| 6. MCP sessions and SSE | Pending | Read-through sessions and durable notifications | Pending |
| 7. Rate limiters | Pending | Atomic Mongo fixed-window counters | Pending |
| 8. Process caps | Pending | Global counters or explicit per-replica contracts | Pending |
| 9. Deployment config | Done | Restored tracked Kubernetes manifests with two backend replicas, separate public and headless Services, downward API identity, private-port NetworkPolicy, streaming ingress policy, scale-safe Compose, and nginx WebSocket support | `kubeconform`: 17 valid; `docker compose config --quiet`: pass; `nginx -t`: pass |
| 10. Documentation | Done | Documented the fenced node owner, authenticated private listener, MongoDB coordination stores, durable MCP outbox, global limits, deployment topology, and every new environment variable | `git diff --check`: pass; stale single-replica claims absent |

## Architecture selection

Candidate 1 is the base because it follows the required embedded `Node` owner
record, answers dispatchability from the record already loaded by routing, and
uses operation-shaped forwarding that preserves streaming and mutation-safety
semantics. Candidate 2 contributed the separate internal listener, ingress-side
node signatures, typed duplex session handles, and ordered MCP outbox.

The selected design corrects both candidates where they weaken the brief:

- process generation and socket connection UUIDs replace resettable epochs;
- owner cleanup matches the complete fence tuple;
- internal replay nonces use MongoDB unique/TTL storage;
- all configured production rate limits and passthrough/SSH caps are global;
- event dedup is claim/commit/release rather than post-success read-then-insert;
- session slots renew and cancel work when fenced;
- MCP authentication revalidates durable state so deletes cannot leave stale
  positive cache entries;
- the internal plane binds a separate listener and cannot be routed by the
  public ingress.

The full contract is recorded in `docs/HORIZONTAL_SCALING_ARCHITECTURE.md`.

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
| Node owner Mongo CAS tests | Blocked by test infrastructure | The earlier replica-set listener on `127.0.0.1:27019` stopped and Docker Desktop did not answer; tests remain implemented and will be rerun after the fixture is restored. |
| Kubernetes schemas | Pass | `kubeconform -strict -summary k8s`: 17 resources valid, 0 invalid, 0 errors, 0 skipped |
| Kubernetes YAML | Pass | `yq eval-all '.' k8s/*.yaml`: all documents parsed |
| Production Compose | Pass | Merged `docker-compose.yml` and `docker-compose.prod.yml` passes `docker compose config --quiet`; backend has no container name or published host port |
| Frontend nginx | Pass | Rendered `frontend/nginx.conf.template` passes nginx 1.27 `nginx -t` with WebSocket headers and one-hour stream timeouts |
| Deployment documentation | Pass | `git diff --check`; removed stale stateless, per-instance rate-limit, and sticky-session claims |

Final Mongo-backed verification uses an isolated local replica-set deployment
through `NYXID_TEST_DATABASE_URL`. This executes the repository's transaction
tests without skips or weakened assertions.
