---
title: The broker model
description: How NyxID acts as a credential broker so agents and apps call downstream APIs without ever holding the raw key.
---

NyxID sits between callers — agents, apps, pipelines — and the downstream APIs they need to reach. Callers authenticate to NyxID with a NyxID-issued key. NyxID authenticates to the downstream API with the stored upstream credential. The upstream credential is never handed to the caller.

This separation is the broker model. It is not a proxy in the load-balancing sense; it is a credential choke point that enforces policy, injects secrets, and records what happened.

## Why a broker

The conventional alternative is to give each agent or application the upstream API key directly. This creates several compounding problems:

- Rotating one key requires finding and updating every consumer that holds it.
- Revoking access for one agent means rotating the shared key and distributing a new one to every other agent.
- Auditing which agent made which call requires each agent to tag its own requests, which is not enforced.
- Key sprawl grows proportionally with the number of agents and services.

With a broker, the upstream credential lives in one place. Revoking a single agent's NyxID key has zero effect on other agents. Rotating an upstream credential is a single update in NyxID, transparent to all callers. Audit records are written by the broker itself, not by the callers.

## The trust boundary

```
Caller                  NyxID Proxy                  Downstream API
  │                          │                              │
  │  NyxID Agent Key         │                              │
  │  (nyxid_ag_…)            │                              │
  ├─────────────────────────>│                              │
  │                          │  authenticate NyxID key      │
  │                          │  look up stored credential   │
  │                          │  inject into outbound req    │
  │                          ├─────────────────────────────>│
  │                          │  Authorization: Bearer sk-…  │
  │                          │<─────────────────────────────│
  │<─────────────────────────│  downstream response         │
```

The upstream credential (`sk-…`, an OAuth token, a bearer token) never crosses the left boundary. The caller only ever sees the NyxID key it was issued. If that NyxID key leaks, it can be revoked in seconds. The underlying provider credential stays intact.

## What gets stored

NyxID holds credentials as AES-256-GCM ciphertext in MongoDB, using envelope encryption: each stored credential is encrypted under a per-record Data Encryption Key (DEK), which is itself wrapped by a Key Encryption Key (KEK). See [Encryption & key management](/docs/shared/concepts/encryption) for the full key hierarchy.

The data model for user-held credentials has three records:

- **UserEndpoint** — the target URL (e.g. `https://api.openai.com`)
- **UserApiKey** — the external credential encrypted at rest
- **UserService** — routing config binding the endpoint, credential, and auth injection method

When a proxy request arrives, these three records are resolved, the credential is decrypted just-in-time, injected into the outgoing request, and immediately dropped from memory. See [Endpoints, keys & services](/docs/shared/concepts/endpoints-keys-services) for how those records relate.

## Credential injection modes

NyxID supports several injection strategies, chosen per service:

| Mode | What gets added to the outbound request |
|------|-----------------------------------------|
| `bearer` | `Authorization: Bearer <credential>` |
| `header` | A named HTTP header (e.g. `x-api-key`) |
| `query` | A query parameter |
| `basic` | HTTP Basic auth (`Authorization: Basic base64(user:pass)`) |

## What the broker does not do

The broker model is about credential isolation, not content filtering. NyxID forwards the request body as-is. It does not parse, modify, or store request or response bodies (approvals inspect the body just enough to build a human-readable action description, but never persist it).

## Related guides

- [Connect your agent](/docs/ai/getting-started/connect-your-agent)
- [Endpoints, keys & services](/docs/shared/concepts/endpoints-keys-services)
- [The proxy](/docs/shared/concepts/the-proxy)
- [Agent isolation](/docs/shared/concepts/agent-isolation)
