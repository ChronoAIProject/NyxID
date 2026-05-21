---
title: Isolate agents with scoped keys
description: Give each AI agent its own NyxID Agent Key with independent service scope, rate limits, credential overrides, and audit attribution.
---

Agent isolation lets multiple AI agents owned by the same NyxID user operate with completely independent access. Each agent gets its own scoped API key, its own rate limit bucket, its own credential binding per service, and its own row in the audit log. There is no separate "agent" model — **an API key is the agent identity**.

For a walkthrough that applies this to Claude Code and Codex specifically, see [Set up Claude Code, Cursor & Codex](/docs/ai/guides/claude-code-cursor-codex). For the conceptual background, see [Agent isolation](/docs/shared/concepts/agent-isolation).

## What isolation actually prevents

Without per-agent keys, all agents share one upstream credential. One leaked key compromises everything; you cannot tell which tool made which request; you cannot revoke one agent's access without breaking the others; you cannot apply different rate limits or route different agents to different upstream accounts.

With per-agent keys:

- Each Agent Key carries its own allowed service list (`allowed_service_ids`). A key that leaks can only reach the services in that list.
- Rotating a single Agent Key does not affect any other agent.
- The audit log records `api_key_id` and `api_key_name` on every proxied request, so attribution is per-agent.
- Each key can have its own `rate_limit_per_second` / `rate_limit_burst`.
- Each key can be bound to a different upstream credential for the same logical service.

## Creating an Agent Key

```bash
nyxid api-key create \
  --name "claude-coding" \
  --platform claude-code \
  --scopes "proxy"
```

Available `--platform` values: `claude-code`, `codex`, `cursor`, `openclaw`, `generic`.

Save the `nyx_...` value shown at creation — it is displayed once.

## Scoping a key to specific services

By default, a new key can access all of your services (`allow_all_services: true`). To restrict it:

**Web console:** Open **AI Services → Agent Keys → [key] → Service Scope**, uncheck **Allow all services**, select the services this agent needs, and save.

**CLI:**

```bash
# Get service UUIDs
nyxid service list

# Update the key — both flags are required
nyxid api-key update <KEY_ID> \
  --allowed-services "<svc-uuid-1>,<svc-uuid-2>" \
  --allow-all-services false
```

:::warning
Passing `--allowed-services` without `--allow-all-services false` stores the list but does not enforce it. The web console handles both in one save; the CLI requires both flags explicitly.
:::

A key with `allow_all_services: false` and an empty `allowed_service_ids` has no proxy access — useful for management-only or callback-only keys.

## Per-agent credential overrides (credential bindings)

Two agents calling the same service can inject different upstream credentials. For example, `claude-coding` injects a standard-tier OpenAI key while `codex-research` injects a premium key:

```
claude-coding  ──► NyxID ──► openai-standard ($50/mo key)
codex-research ──► NyxID ──► openai-premium ($500/mo key)
```

Each binding is an `AgentServiceBinding` record: `(api_key_id, user_service_id) → user_api_key_id`.

**Web console:** Open **AI Services → Agent Keys → [key] → Bindings**, click **Add binding**, pick the service, and select which stored credential to inject.

**CLI:**

```bash
nyxid api-key bind <KEY_ID_OR_NAME> \
  --service llm-openai \
  --credential "openai-premium"    # label of the credential to inject
```

Without a binding, the proxy falls back to the default credential on the `UserService`.

Binding API endpoints:

```
POST   /api/v1/api-keys/{key_id}/bindings      Create a binding
GET    /api/v1/api-keys/{key_id}/bindings      List bindings
DELETE /api/v1/api-keys/{key_id}/bindings/{id} Remove a binding
```

## Per-agent rate limits

Set per-key rate limits on the **Rate Limits** card in the key's detail page, or via the API:

```bash
curl -X PUT https://nyx-api.chrono-ai.fun/api/v1/api-keys/<KEY_ID> \
  -H "Authorization: Bearer $NYXID_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "rate_limit_per_second": 5,
    "rate_limit_burst": 10
  }'
```

When `rate_limit_per_second` is set, the per-agent token-bucket limiter kicks in. Requests that exceed the rate get `429 Too Many Requests`. When it is not set, the global rate limiter applies (same as before isolation).

## Audit attribution

Every proxied request with an Agent Key gets `api_key_id` and `api_key_name` recorded in the audit log. The proxy response also includes `X-NyxID-Agent-Id: <key-uuid>`.

To filter the audit log by a specific agent:

```bash
# Web console: Admin → Audit Log → filter by API key
```

## CLI profiles for multiple agent sessions

For running multiple agents simultaneously on one machine, use profiles. Each profile has independent token storage:

```bash
nyxid login --base-url https://nyx-api.chrono-ai.fun --profile coding-agent
nyxid proxy request openai /chat/completions --profile coding-agent
NYXID_PROFILE=coding-agent nyxid service list
```

Profile token storage:
- Default (no `--profile`): `~/.nyxid/`
- Named profile: `~/.nyxid/profiles/{name}/`

Full backward compatibility — no `--profile` means the default path.

For node agents, each profile runs its own daemon process:

```bash
nyxid node register --token nyx_nreg_... --profile coding-agent
nyxid node daemon install --profile coding-agent
nyxid node daemon start --profile coding-agent
```

Service labels on macOS: `dev.nyxid.node.{profile}`. On Linux: `nyxid-node-{profile}.service`.

## Backward compatibility

All isolation fields are additive. Existing API keys and auth paths are unaffected:

| Area | Guarantee |
|---|---|
| Existing API keys | `allow_all_services: true`, no rate limit override, no bindings — behavior identical to before |
| Existing auth (JWT, session, service account) | New `AuthUser` fields are `None`. No scope enforcement, no rate limit override |
| No `--profile` | Reads from `~/.nyxid/` (unchanged) |
| No bindings | Proxy uses default `UserService` credential (unchanged) |

## Checklist

1. Create one Agent Key per agent per environment
2. Set `--platform` to record the tool type in the audit log
3. Uncheck **Allow all services** and select only the services that agent needs
4. Optionally bind specific credentials per service if agents need separate upstream accounts
5. Optionally set per-key rate limits to throttle experimental agents or CI traffic
6. Store Agent Keys as environment variables — never commit them
