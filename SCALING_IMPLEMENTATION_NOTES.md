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
| 1. Node WebSocket registry | In progress | Architecture grounding and routing design | Pending |
| 2. OAuth refresh sweep | Pending | Per-key Mongo lease for sweep and proxy refresh | Pending |
| 3. DPoP replay cache | Pending | Mongo unique insert and TTL expiry | Pending |
| 4. Event dedup cache | Pending | Mongo unique insert and TTL expiry | Pending |
| 5. Telegram polling | Pending | Mongo leader lease with renewal and takeover | Pending |
| 6. MCP sessions and SSE | Pending | Read-through sessions and durable notifications | Pending |
| 7. Rate limiters | Pending | Atomic Mongo fixed-window counters | Pending |
| 8. Process caps | Pending | Global counters or explicit per-replica contracts | Pending |
| 9. Deployment config | Pending | Kubernetes, nginx, and Compose changes | Pending |
| 10. Documentation | Pending | Deployment, environment, and node architecture docs | Pending |

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

Final Mongo-backed verification will set `NYXID_TEST_DATABASE_URL` to an isolated local replica-set deployment. This is required to execute, rather than skip or weaken, the repository's transaction tests.
