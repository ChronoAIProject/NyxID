# Agent Isolation

## Overview

Agent isolation lets different AI agents (Claude Code, Codex, custom bots, etc.) belonging to the same NyxID user operate with independent credentials, rate limits, scopes, and audit trails. There is no separate "agent" model -- an **API key is the agent identity**.

```mermaid
graph TD
    subgraph Same NyxID User
        K1[API Key: claude-coding<br/>scope: openai, github<br/>rate: 10 req/s]
        K2[API Key: codex-research<br/>scope: openai<br/>credential override: premium key]
        K3[API Key: support-bot<br/>scope: openai, anthropic<br/>rate: 5 req/s]
    end

    subgraph NyxID Backend
        AUTH[Auth Middleware<br/>extracts api_key_id]
        SCOPE[Scope Check<br/>allowed_service_ids]
        RATE[Per-Agent Rate Limiter<br/>token bucket per key]
        BIND[Credential Override<br/>agent_service_bindings]
        PROXY[Proxy / LLM Gateway]
        AUDIT[Audit Log<br/>api_key_id attribution]
    end

    K1 --> AUTH
    K2 --> AUTH
    K3 --> AUTH
    AUTH --> SCOPE --> RATE --> BIND --> PROXY --> AUDIT
```

## How It Works

### Proxy Request Flow

```mermaid
sequenceDiagram
    participant Agent as AI Agent (API Key)
    participant Auth as Auth Middleware
    participant Scope as Scope Check
    participant Rate as Rate Limiter
    participant Bind as Binding Lookup
    participant Proxy as Proxy Handler
    participant Audit as Audit Log

    Agent->>Auth: Request with API key
    Auth->>Auth: Load ApiKey from DB
    Auth->>Scope: AuthUser { api_key_id, allowed_service_ids, ... }

    alt Service not in allowed_service_ids
        Scope-->>Agent: 403 ApiKeyScopeForbidden
    end

    Scope->>Rate: Check per-agent rate limit
    alt rate_limit_per_second exceeded
        Rate-->>Agent: 429 Too Many Requests
    end

    Rate->>Bind: Lookup agent_service_bindings(api_key_id, service_id)
    alt Binding found
        Bind->>Proxy: Use override credential
    else No binding
        Bind->>Proxy: Use default UserService credential
    end

    Proxy->>Proxy: Forward request to downstream
    Proxy->>Audit: Log with api_key_id + api_key_name
    Proxy-->>Agent: Response + X-NyxID-Agent-Id header
```

### Credential Override

The core new capability. Two agents using the same service (e.g., OpenAI) can inject different API keys:

```mermaid
graph LR
    subgraph agent_service_bindings
        B1["api_key: claude-coding<br/>service: openai<br/>credential: openai-standard ($50/mo)"]
        B2["api_key: codex-research<br/>service: openai<br/>credential: openai-premium ($500/mo)"]
    end

    subgraph Proxy Resolution
        R[Resolve Credential]
    end

    B1 --> R
    B2 --> R
    R -->|claude-coding| S1[Inject $50/mo key]
    R -->|codex-research| S2[Inject $500/mo key]
```

Without a binding, the proxy falls back to the default credential on the `UserService` (existing behavior).

## Data Model

```mermaid
erDiagram
    User ||--o{ ApiKey : owns
    ApiKey ||--o{ AgentServiceBinding : "overrides credentials via"
    AgentServiceBinding }o--|| UserService : "targets"
    AgentServiceBinding }o--|| UserApiKey : "injects"

    ApiKey {
        string id PK
        string user_id FK
        string name "human-readable label"
        string scopes "space-separated"
        string platform "optional: claude-code, codex, etc."
        array allowed_service_ids "service scope"
        array allowed_node_ids "node scope"
        bool allow_all_services "default: true"
        bool allow_all_nodes "default: true"
        int rate_limit_per_second "optional per-key override"
        int rate_limit_burst "optional per-key override"
    }

    AgentServiceBinding {
        string id PK
        string api_key_id FK "the agent"
        string user_service_id FK "which service"
        string user_api_key_id FK "which credential to inject"
        string user_id "denormalized"
    }
```

### Key fields on `ApiKey` (added by this feature)

| Field | Type | Default | Purpose |
|---|---|---|---|
| `platform` | `Option<String>` | `None` | Display label (claude-code, codex, openclaw, cursor, generic) |
| `rate_limit_per_second` | `Option<u32>` | `None` | Per-key rate limit (falls back to user-level when `None`) |
| `rate_limit_burst` | `Option<u32>` | `None` | Per-key burst capacity |

### Key fields on `AuthUser` (added by this feature)

| Field | Type | Default | Purpose |
|---|---|---|---|
| `api_key_id` | `Option<String>` | `None` | Populated when auth is via API key |
| `api_key_name` | `Option<String>` | `None` | Human-readable label for audit |
| `rate_limit_per_second` | `Option<u32>` | `None` | Copied from ApiKey for middleware |
| `rate_limit_burst` | `Option<u32>` | `None` | Copied from ApiKey for middleware |

All new fields are `Option` with `serde(default)`. Existing API keys and auth paths are unaffected.

## API Endpoints

### Credential Bindings

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v1/api-keys/{key_id}/bindings` | Create credential binding |
| `GET` | `/api/v1/api-keys/{key_id}/bindings` | List bindings for a key |
| `DELETE` | `/api/v1/api-keys/{key_id}/bindings/{id}` | Remove a binding |

### Usage

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/v1/api-keys/usage` | Per-key usage stats (requests, errors, top services) |
| `GET` | `/api/v1/api-keys/{id}/usage` | Usage for a specific key |

### Existing (modified)

| Method | Path | Change |
|---|---|---|
| `POST` | `/api/v1/api-keys` | Accepts optional `platform` field |
| `PUT` | `/api/v1/api-keys/{id}` | Accepts `rate_limit_per_second`, `rate_limit_burst`, `platform` |

## CLI

### API Key Commands

```bash
# Create with optional platform label and service scope
nyxid api-key create --name "coding-agent" --platform claude-code \
  --allowed-services "svc-1,svc-2" --allow-all-services false

# Bind a specific credential to a key for a service
nyxid api-key bind <ID_OR_NAME> --service <SLUG> --credential <LABEL>

# All existing commands unchanged
nyxid api-key list / show / rotate / delete
```

### CLI Profiles

For running multiple agent identities on one machine:

```bash
nyxid login --base-url https://... --profile coding-agent
nyxid proxy request openai /chat/completions --profile coding-agent
NYXID_PROFILE=coding-agent nyxid service list
```

```mermaid
graph LR
    subgraph Token Storage
        D["~/.nyxid/<br/>access_token<br/>refresh_token<br/>base_url"]
        P1["~/.nyxid/profiles/coding-agent/<br/>access_token<br/>refresh_token<br/>base_url"]
        P2["~/.nyxid/profiles/research-agent/<br/>..."]
    end

    NO["No --profile"] --> D
    F1["--profile coding-agent"] --> P1
    F2["--profile research-agent"] --> P2
```

No `--profile` = default path (`~/.nyxid/`). Full backward compatibility.

### Node Multi-Instance

Each profile gets its own daemon process and config directory:

```bash
nyxid node register --token nyx_nreg_... --profile coding-agent
nyxid node daemon install --profile coding-agent
nyxid node daemon start --profile coding-agent
```

```mermaid
graph LR
    subgraph macOS LaunchAgents
        L1["dev.nyxid.node<br/>(default)"]
        L2["dev.nyxid.node.coding-agent"]
        L3["dev.nyxid.node.research-agent"]
    end

    subgraph Config Dirs
        C1["~/.nyxid-node/"]
        C2["~/.nyxid-node/profiles/coding-agent/"]
        C3["~/.nyxid-node/profiles/research-agent/"]
    end

    L1 --- C1
    L2 --- C2
    L3 --- C3
```

### Docker

```bash
# Auto-register + start (no host setup needed)
docker run --user "$(id -u):$(id -g)" \
  -v ~/.nyxid-node:/app/config \
  -e NYXID_NODE_TOKEN=nyx_nreg_... \
  -e NYXID_NODE_URL=wss://... \
  nyxid-node

# Or mount existing config
docker run --user "$(id -u):$(id -g)" \
  -v ~/.nyxid-node:/app/config \
  nyxid-node
```

Containers use the file backend (AES-GCM encrypted). OS keychain is not available in Docker.

## Frontend

- **API key detail page**: platform selector, rate limit editor, credential bindings CRUD, usage stats
- **Keys page**: per-key usage dashboard (requests, errors, error rate, top services, 7-day activity)
- **Admin audit log page**: filterable by `api_key_id`

## Per-Agent Rate Limiting

```mermaid
graph TD
    REQ[Incoming Request] --> CHECK{api_key_id present<br/>AND rate_limit_per_second set?}
    CHECK -->|No| GLOBAL[Use global rate limiter]
    CHECK -->|Yes| BUCKET[Per-key token bucket]
    BUCKET -->|Tokens available| ALLOW[Allow request]
    BUCKET -->|Empty| REJECT[429 Too Many Requests]
    GLOBAL --> ALLOW
```

Implementation: in-memory `PerAgentRateLimiter` with token-bucket per API key. Background cleanup evicts idle buckets after 120 seconds.

When `rate_limit_per_second` is `None` on the key, the per-agent check is a no-op and the global rate limiter applies (unchanged behavior).

## Audit Attribution

Every proxy and LLM gateway request logs `api_key_id` and `api_key_name` in the audit event. This enables:
- Per-agent usage dashboards
- Admin audit log filtering by API key
- `X-NyxID-Agent-Id` response header for downstream observability

## Backward Compatibility

All changes are additive. No breaking changes for existing users:

| Area | Guarantee |
|---|---|
| Existing API keys | `allow_all_services=true`, `allow_all_nodes=true`, no rate limit override, no bindings. Behavior identical to before. |
| Existing auth paths (JWT, session, SA) | New `AuthUser` fields are `None`. No scope enforcement, no rate limit override. |
| No `--profile` flag | Reads from `~/.nyxid/` (unchanged). |
| No `agent_service_bindings` | Proxy uses default `UserService.api_key_id` (unchanged). |
| API responses | New optional fields use `skip_serializing_if`. Existing clients see no new fields unless they opt in. |

## Key Files

| File | Purpose |
|---|---|
| `backend/src/mw/auth.rs` | AuthUser with api_key_id, scopes, rate limits |
| `backend/src/mw/rate_limit.rs` | PerAgentRateLimiter (token bucket) |
| `backend/src/models/agent_service_binding.rs` | Credential override model |
| `backend/src/services/agent_binding_service.rs` | Binding CRUD + lookup |
| `backend/src/services/proxy_service.rs` | resolve_agent_credential_override() |
| `backend/src/handlers/agent_bindings.rs` | Binding REST endpoints |
| `backend/src/handlers/api_keys.rs` | Usage endpoints, bindings_count |
| `cli/src/auth.rs` | Profile-aware token storage |
| `cli/src/commands/api_key.rs` | bind command, --platform flag |
| `cli/docker-entrypoint.sh` | Auto-register in Docker |
