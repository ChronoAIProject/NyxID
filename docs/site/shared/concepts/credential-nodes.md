---
title: Credential nodes
description: Why credential nodes exist, how they let NyxID route requests to private or on-premise services without storing credentials centrally, and how routing and failover work.
---

NyxID's default proxy path stores the user's credential encrypted in its own database and injects it when forwarding requests. That model works well for cloud services — OpenAI, Anthropic, GitHub, and similar APIs that are reachable from the public internet. It does not work when the target is a service running on a private network, a developer's laptop, or a self-hosted instance that should never send its credentials off-host.

Credential nodes solve this. A node is a lightweight agent that runs on the target host, holds credentials locally, and receives proxy requests from NyxID over a persistent WebSocket. The credential is injected on the node, locally, before the request is forwarded to the downstream service. The raw credential never transits the NyxID server.

## The control plane / data plane split

NyxID acts as the control plane: it authenticates callers, resolves routing, enforces approvals, and writes audit records. The node is the data plane: it holds credentials and makes the outbound HTTP call.

```
Caller ──HTTP──> NyxID Server
                     │
                     │  WebSocket (request metadata only)
                     │  no credential in this direction
                     ▼
              Node Agent (your host)
                     │
                     │  HTTP (credential injected here)
                     ▼
              Downstream Service
              (localhost:11434, internal API, etc.)
```

This is an opt-in feature. Users without nodes continue using the standard direct proxy. Users with nodes can selectively route specific services through a node while keeping other services on the direct path.

## Why nodes exist

Three scenarios where direct proxy falls short:

**Private network services.** A Home Assistant instance, a Ollama model server, an internal database — these are not reachable from NyxID's servers. A node agent runs on the same network and bridges the gap.

**Credential sensitivity.** Some organizations require that certain secrets (for example, an enterprise SSO token or a database password) never leave the host they are used on. With a node, NyxID's server only ever sees an HMAC-signed proxy request — it never sees the credential value.

**Self-hosted providers.** OpenClaw and similar self-hosted AI gateways run on-premise. The node handles credential injection and proxies through to the local gateway without exposing the gateway URL or access token externally.

## Node registration and authentication

A node is registered using a one-time token issued by NyxID (`nyx_nreg_…`). The registration flow:

1. A user or admin calls `POST /api/v1/nodes/register-token` to get a registration token.
2. The node agent calls `nyxid node register --token nyx_nreg_... --url wss://...` to consume the token and receive a permanent auth token (`nyx_nauth_…`) and an HMAC signing secret.
3. The raw auth token and signing secret are shown once and stored locally in the node config, encrypted with AES-256-GCM.
4. Only the SHA-256 hash of each is stored on the NyxID server.

After registration, the node connects to `wss://…/api/v1/nodes/ws` and authenticates via the first WebSocket message (not via HTTP headers, which would be visible in server logs). If no valid auth arrives within 10 seconds, the connection is closed.

## How routing works

Service bindings map a node to a service slug. When a proxy request arrives for a user:

1. NyxID checks whether the user has an active node binding for the requested service.
2. If a binding exists and the node is online (confirmed by both the database status and the in-memory WebSocket connection pool), the request is dispatched to that node.
3. If no binding exists, or the node is offline, NyxID falls through to the standard direct proxy.

The node never self-reports which services it handles. Bindings are checked server-side, so a compromised node cannot route itself into serving services it was not explicitly bound to.

## Failover

A user can bind multiple nodes to the same service with numeric priority values. Lower values mean higher priority. When the primary node fails (offline, timed out, or unhealthy), NyxID automatically retries the next node in priority order. A new request ID is generated per retry to avoid correlation conflicts.

Nodes are considered unhealthy and skipped during failover if their error rate exceeds 50% across at least 10 samples (tracked per node in `NodeMetrics`). Heartbeats run every 30 seconds; nodes that miss the 90-second heartbeat window are marked offline and their WebSocket connections are closed.

:::tip
After all nodes fail, NyxID falls through to the standard direct proxy. For services where the credential must not leave the host, make sure no plain credential is stored in NyxID, so the fallback fails safely rather than falling back to central proxy.
:::

## HMAC request signing

Every proxy request NyxID sends to a node is signed with HMAC-SHA256 using the shared signing secret established at registration. The canonical string covers the timestamp, a nonce, the HTTP method, path, query string, and body. The node verifies the signature and rejects requests with:

- Timestamps more than 5 minutes in the past
- Nonces seen within the recent window (last 10,000 nonces tracked)

This prevents both replay attacks and request forgery by anyone who might intercept the WebSocket channel.

## Credential storage on the node

The node stores credentials in a local encrypted config file at `~/.nyxid-node/config.toml`. The key material is encrypted with AES-256-GCM using a per-host key at `~/.nyxid-node/.keyfile` (0600 permissions). Alternatively, credentials can be stored in the OS keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service) by running `nyxid node register --keychain`.

Credentials are keyed by service slug, not service ID, so they remain valid if the service ID changes (for example, after re-registering a service).

## Streaming through nodes

Node-routed responses support streaming. When the downstream service returns an SSE stream, a large file, or any chunked response, the node sends it back to NyxID over WebSocket using binary frames. Each frame is prefixed with the 36-byte request ID so NyxID can route chunks to the correct pending request without JSON parsing overhead. Small responses (under 256 KB) use the simpler base64 JSON path.

## Multi-instance operation

Each profile runs its own node daemon and config directory:

```
~/.nyxid-node/                          default profile
~/.nyxid-node/profiles/coding-agent/    --profile coding-agent
```

On macOS, each profile registers as a separate launchd LaunchAgent (`dev.nyxid.node.{profile}`). On Linux, each gets a separate systemd user unit (`nyxid-node-{profile}.service`). This allows a single host to participate in multiple NyxID user accounts or serve different service sets per profile.

## Related guides

- [Agent isolation](/docs/shared/concepts/agent-isolation)
- [The proxy](/docs/shared/concepts/the-proxy)
- [Endpoints, keys & services](/docs/shared/concepts/endpoints-keys-services)
