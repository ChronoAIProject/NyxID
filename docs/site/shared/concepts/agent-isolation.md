---
title: Agent isolation
description: How NyxID gives each AI agent its own scoped API key, independent rate limit, and dedicated audit trail, so multiple agents on the same account cannot interfere with each other.
---

When several AI agents operate on the same NyxID account — a coding assistant, a research pipeline, a support bot — they need to be independent: independent credentials, independent rate limits, and independent audit trails. Without isolation, a runaway agent can exhaust the rate limit for every other agent, a leaked key grants access to all the services the account can reach, and audit logs cannot tell one agent's traffic from another's.

Agent isolation in NyxID is built on a single principle: **an API key is the agent identity**. There is no separate "agent" model. Every property that distinguishes one agent from another lives on its API key.

## Why it matters

Consider a developer who runs Claude Code and an n8n automation workflow under the same NyxID account. Without isolation:

- The automation workflow, if it bugs out and makes thousands of requests per minute, will saturate the account-level rate limit and block the coding assistant.
- If the coding assistant's API key leaks, an attacker has access to every service the account can reach — including the workflow's credentials.
- The audit log shows a mix of traffic with no way to attribute which action came from which tool.

With isolation:

- Each agent has its own rate limit bucket. The automation exhausting its bucket does not affect the coding assistant.
- Each agent holds only its own scoped key. The key scope is enforced server-side; a leaked coding-assistant key cannot reach the services the automation uses.
- Every audit record includes the `api_key_id` and `api_key_name`, so the log is fully attributable per agent.

## How it works

### Scope

An API key carries an explicit service scope:

- `allow_all_services: true` (default for new keys) — the key can reach all of the user's services.
- `allow_all_services: false` + `allowed_service_ids: [...]` — the key can only reach the listed services.

The same scope model applies to nodes: `allow_all_nodes` / `allowed_node_ids`.

Scope is enforced at the proxy layer before any credential resolution. A request from an out-of-scope key receives a `403 ApiKeyScopeForbidden` regardless of whether the service exists or the user has credentials for it.

### Per-agent rate limits

Every API key can carry its own rate limit override:

- `rate_limit_per_second` — maximum requests per second for this key
- `rate_limit_burst` — burst capacity

When these are set, the key gets its own token-bucket limiter in-process, separate from the global per-IP limiter. When a key has no rate limit override, it shares the global limiter (unchanged behavior from before isolation was introduced). Idle per-key buckets are evicted after 120 seconds to prevent memory growth.

### Credential overrides

Two agents that both call the same service (say, OpenAI) can inject different upstream API keys. This is done through `AgentServiceBinding` records:

```
AgentServiceBinding
  api_key_id:      → ApiKey (the agent)
  user_service_id: → UserService (which service)
  user_api_key_id: → UserApiKey (which credential to inject)
```

At proxy time, NyxID checks for a binding matching `(api_key_id, user_service_id)`. If a binding exists, its `user_api_key_id` overrides the service default. If no binding exists, the service's default credential is used.

This allows a coding agent to use a standard-tier OpenAI key while a research pipeline uses a higher-quota key — both under the same NyxID account, both calling the same `llm-openai` service slug.

### Audit attribution

Every proxy request made with an API key writes `api_key_id` and `api_key_name` to the audit record. The response also includes an `X-NyxID-Agent-Id` header so callers can verify which identity NyxID attributed the request to.

## Platform labels

API keys can carry an optional `platform` label — values like `claude-code`, `codex`, `cursor`, `openclaw`, `generic`. The label is display-only and does not affect behavior. It is useful in the admin audit log and the key management UI for identifying which tool a key belongs to at a glance.

## CLI profiles

For running multiple agents on one machine with separate NyxID sessions (rather than separate API keys within the same session):

```bash
nyxid login --profile coding-agent
nyxid proxy request openai /chat/completions --profile coding-agent
NYXID_PROFILE=coding-agent nyxid service list
```

Profiles store session tokens in separate directories:

```
~/.nyxid/                              default profile
~/.nyxid/profiles/coding-agent/       --profile coding-agent
```

Each profile is fully independent — separate login, separate token storage, separate CLI state. This is distinct from API key isolation: profiles are for different NyxID user sessions, while API key isolation is for multiple agents within the same user session.

## Backward compatibility

All isolation fields are additive and optional. Existing API keys without these fields behave identically to before: `allow_all_services: true`, no rate limit override, no credential bindings. Existing auth paths (JWT, session, service account) carry `None` for all isolation fields and are unaffected.

## Related guides

- [Set up Claude Code, Cursor & Codex](/docs/ai/guides/claude-code-cursor-codex)
- [Isolate agents with scoped keys](/docs/ai/guides/agent-isolation)
- [Endpoints, keys & services](/docs/shared/concepts/endpoints-keys-services)
- [The broker model](/docs/shared/concepts/broker-model)
