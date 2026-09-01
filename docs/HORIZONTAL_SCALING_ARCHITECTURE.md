# Horizontal scaling architecture

## Scope

NyxID replicas share durable coordination through MongoDB. Live credential-node
WebSockets and MCP SSE responses remain process-owned, but callers do not need
to land on the owning process. A private authenticated HTTP plane carries only
the traffic that must reach a live process; MongoDB remains the authority for
ownership, leases, replay state, rate budgets, and durable notifications.

This design preserves the existing node dispatch boundary: once a node may
have received an operation, an ambiguous failure is classified as dispatched
and unsafe methods are not retried on another node.

## Replica identity

Each process has three identity values:

- `instance_name`: stable for the pod or host, normally `POD_NAME` or
  `HOSTNAME`.
- `generation_id`: a UUID generated on every process start.
- `internal_base_url`: the peer-reachable address advertised by that process.

The internal listener has its own bind address and port. It is never merged
into the public application router and is not exposed by the ingress. The
Kubernetes deployment advertises each pod through its pod IP and a headless
service.

## Node ownership

`Node.connection_owner` is the single source of truth for a live node socket:

```text
instance_name
generation_id
connection_id
internal_base_url
claimed_at
renewed_at
expires_at
capabilities
capabilities_resolved
```

`generation_id` fences a prior process using the same pod name.
`connection_id` fences an older socket in the same process. Claim succeeds only
when the owner is absent, expired, or belongs to the same process generation.
A reconnect on the same process installs a new `connection_id`; teardown of the
old reader cannot clear it. Renew, capability updates, release, and disconnect
all match the full `(instance_name, generation_id, connection_id)` tuple.

The owner renews its records while sweeping only its local sockets. Losing a
renewal closes that local socket and clears its pending operations. Any replica
may conditionally clear an expired owner and mark the node offline. TTL cleanup
is not used as the correctness signal: every read and compare-and-set checks
`expires_at` explicitly.

Route resolution treats an active node with an unexpired owner as connected
somewhere. Capability and status responses use the persisted owner snapshot,
not a local `DashMap`. A request resolves the owner immediately before dispatch
and retries owner resolution once after a pre-dispatch fence rejection.

Delete, revoke, and administrative disconnect first invalidate the exact owner
snapshot in MongoDB, then ask that owner to close the socket. If the peer is
unreachable, its next renewal fails and self-revokes. A stale teardown cannot
affect a replacement connection.

`NODE_MAX_WS_CONNECTIONS` is a per-replica file-descriptor and memory cap. Its
name, configuration help, and deployment documentation state those semantics.

## Internal forwarding

`NodeDispatch` is the public service used by node-bound callers. It owns the
local `NodeWsManager`, owner lookup, local-versus-remote routing, fencing,
transport conversion, cancellation, and error de-identification. Callers never
receive a peer address.

The private plane uses operation-shaped transports behind one authentication
guard:

- unary HTTP for SSH exec, credential pushes, and disconnect;
- streamed HTTP bodies for HTTP proxy responses, without buffering;
- one WebSocket per SSH tunnel, web terminal, or downstream WebSocket
  passthrough operation.

The ingress replica computes the existing node request HMAC tuple. Node signing
secrets never cross the internal plane. The owner reports acceptance only after
the local bounded writer accepts the node frame. Failures before that point are
safe to retry; transport ambiguity after request transmission is always widened
to `dispatched = true`.

Dropping an ingress response stream closes the peer request. Dropping a duplex
session sends a best-effort close and removes both correlation entries. Bounded
channels retain the current full-buffer behavior. Billing reservation,
forwarded marking, byte accounting, and settlement remain on the ingress
replica; an authenticated internal call receives a narrowly scoped egress
permit and is never metered a second time.

## Internal authentication

The internal HMAC key is domain-derived from `ENCRYPTION_KEY`, with the same
deterministic fallback pattern used by other shared HMAC keys. An optional
64-hex override supports key management systems where the local encryption key
is unavailable.

Every request signature binds the protocol version, method, path, timestamp,
nonce, and SHA-256 body digest. Verification requires:

1. the dedicated internal listener;
2. a timestamp inside the configured skew window;
3. a constant-time HMAC match;
4. an atomic nonce insert into a MongoDB collection with a unique key and TTL;
5. an exact current node-owner fence for node-bound operations.

Authentication failures close or return a generic unauthorized result. Peer
URLs and verification details are never included in external errors, response
models, audit payloads, or `Debug` output.

## MongoDB coordination primitives

The cluster module owns four small algorithms:

- `LeaseStore`: fenced renewable ownership with explicit expiry, used for OAuth
  refresh single-flight and the Telegram poller leader.
- `ReplayStore`: unique insert plus TTL, used for DPoP JTI and internal nonces.
- `RateWindowStore`: atomic fixed-window admission using MongoDB server time and
  TTL cleanup. All production authentication, per-key, per-agent, channel,
  trigger, platform, public, and middleware limits use it, so their configured
  cap is cluster-wide.
- `SlotStore`: one expiring renewable document per occupied slot. It globally
  enforces WebSocket passthrough and per-user SSH concurrency. Loss of renewal
  cancels the associated session.

Event dedup is a separate state machine because delivery failures must remain
retryable. A unique-key claim has a short lease, success commits the full dedup
TTL, and a transient forwarding failure conditionally releases the caller's
claim. Concurrent replicas therefore cannot both forward the same event, while
a failed attempt does not suppress a provider retry.

OAuth sweep and proxy-time refresh acquire the same credential-specific lease
before contacting a provider. A loser waits briefly and rereads the credential
instead of reusing a rotating refresh token. The Telegram poller renews one
leader lease and stops its long poll immediately if fenced.

## MCP sessions and notifications

MongoDB is authoritative for MCP authentication and session state. Validation
and authorization lookups read through on a local miss and revalidate cached
entries against the database, so a delete or user-wide revocation cannot leave
a stale-positive credential on another replica. Creates, touches, updates, and
deletes are awaited write-through operations.

Notifications use a durable MongoDB outbox keyed by `(session_id, sequence)`.
The sequence is allocated atomically on the session row. An SSE holder merges
its local low-latency channel with ordered outbox polling, deduplicates by
sequence, and resumes after reconnect. Producers never report success unless
the notification is durable, so a POST on a non-owning replica cannot silently
drop it.

## Deployment properties

The backend deployment runs at least two replicas. The public ClusterIP remains
the ingress target and a separate headless service supplies pod-addressable
DNS. Ingress and frontend nginx preserve WebSocket upgrades and allow one-hour
node WebSocket and MCP/SSE reads. `/mcp` uses prefix routing. Cookie affinity is
an optimization for SSE locality, not a correctness requirement.

Docker Compose leaves backend containers unnamed and unbound from fixed host
ports so `docker compose up --scale backend=N` places them behind the frontend
proxy.

## Verification invariants

- Two replicas admit exactly one owner, lease holder, replay key, dedup claim,
  rate budget, and bounded slot as applicable.
- Every node operation succeeds when ingress and socket owner differ.
- Streaming begins before the full upstream body exists and cancellation tears
  down the remote operation.
- Stale generations and connections cannot renew, release, dispatch, or close
  replacements.
- External responses never contain internal addresses or authentication data.
- MCP sessions and notifications work across three different replicas.
