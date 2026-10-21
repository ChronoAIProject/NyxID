# Agent Isolation

Per-thread `nyxid-assistant` credentials carry an `AssistantTurnFence`. With
the default-off `assistant:live-turn-gate` rollout flag enabled, execution
requires a live, non-stopped turn; dedicated channel-agent credentials remain
separate and idle-capable. Deploy all replicas before enabling the flag, and
disable it before rollback.

NyxBot service ordering does not enable this rollout flag. While it is off,
personal service calls and existing personal/platform model inference remain
available to keys first authenticated without a live turn, subject to their
current grants, ACLs, operation scopes and permission mode. Actions that need
confirmation and organization connection use require a real live turn. Turn
selection and organization model consent cannot be borrowed by an idle request.
An admission captured in a live turn always retains its same-turn and Stop
fence through final dispatch, independently of the rollout flag.

## Login Credentials Bound to a Key

`nyxid login --agent-key` enrolls a CLI profile through explicit human approval in the web console or the mobile app's QR flow. The operator chooses an existing eligible personal/org key or creates a limited key. Each login receives its own `api_key_credentials` child secret; the parent key's primary secret is never recovered, displayed, or silently rotated. New keys also use child credentials, with their undisclosed primary secret discarded.

Authentication resolves the live parent key for scopes, service/node restrictions, per-agent bindings, rate limits, and audit attribution. `AuthUser.api_key_credential_id` identifies the calling login credential while `api_key_id` and `api_key_name` remain the parent identity. Child expiry cannot exceed parent expiry. Parent revocation or rotation invalidates all children, and live parent expiry is checked on every authentication.

`whoami` and `status` identify Agent Key authentication. `logout` revokes only the calling child and clears the profile; individual credentials can also be revoked in the key detail page's Login credentials section. The profile holds a protected `token`, an `auth_kind` marker, and safe `agent_key.json` metadata, with no human refresh token. Rejected credentials never refresh or prompt for account login. Abandoned approved exchanges are revoked after their 60-second delivery window by poll/preview cleanup or the configured background sweep.

Login credential labels combine the sanitized client hostname and requested profile (up to 96 characters), so separate profiles on one machine remain identifiable. Public preview exposes request context and status only; key metadata is available through authenticated selection and self routes or credential delivery. Approval, denial, and delivery audits retain the actor's IP and user agent, with the request ID linking the events. A failed expiry cleanup is logged and retained for retry while the sweep continues processing other exchanges.

### Channel list credential boundaries

An Agent Key login does not authorize the human management commands `nyxid channel-bot list` or `nyxid channel-event channel list`. These require human account access (session cookie or account access token): personal lists are owner-scoped and `--org` requires org-admin access. Their existing OAuth-client account policy is unchanged; first-party-only routes separately reject OAuth-client tokens. To use a management list, ask a human to run `nyxid login` and choose account access, or explicitly select an existing human profile. Never fall back to another identity, create credentials or widen permissions automatically.

Agents already have a narrower read-only discovery path: `GET /api/v1/channel-relay/conversations?page=1&per_page=50`, authenticated with the assigned active API key. It lists only that exact key's active conversation assignments, including device channels (`platform=device`). It does not list all bots, grant management access or discover other keys' assignments. There is no separate channel-event discovery endpoint. No assignments means an empty list.

Unsupported credential types return the same 403, `forbidden` / `1002` and message as before, with additive `details.reason = credential_type_unsupported`, `credential_type`, `accepted: ["user_session"]` and a safe hint. The accepted value describes human account recovery, not a scope or resource grant. CLI text and JSON expose that reason plus manual guidance; the two list commands also point to the assigned-conversation route. Scope failures use `insufficient_scope`; resource permission failures are not mislabelled as credential-type failures. Authentication failures use 401 with `authentication_failed` or `credential_expired` when expiry is reported. Deny-only route guards run before credential verification, so their type-based 403 makes no claim about validity (even an expired API-key-shaped token is rejected there before authentication). See [Error Format](API.md#error-format).

## Asset ownership transfer

General Agent Keys with `write` or `admin` scope and `allow_all_services=true`
can transfer their owner's catalog definitions and supported channel bots through
`/api/v1/ownership/{kind}/{id}`. An org-owned key acts as that organization;
a personal key can also act through its owner's active, unrestricted org-admin
membership. Platform-admin status on the key owner grants no cross-owner override.
Proxy-only, scheduled-invocation, and limited connected-service keys cannot
transfer these assets. There is no per-transfer mobile approval. Preview, live
permission checks, transactional receipts and agent-attributed audit still apply.
See [Ownership transfers](ADMIN_OWNERSHIP_TRANSFERS.md) for routes and adapter limits.

## Overview

Agent isolation lets different AI agents (Claude Code, Codex, custom bots, etc.) belonging to the same NyxID user operate with independent credentials, rate limits, scopes, and audit trails. There is no separate "agent" model -- an **API key is the agent identity**.

Unattended scheduled writes use the separate, fail-closed
`scheduled_invocation` key purpose documented in
[Durable Operation Grants](DURABLE_OPERATION_GRANTS.md). General agent keys and
legacy `ApprovalGrant` rows never gain that authority.

```mermaid
graph TD
    subgraph Same NyxID User
        K1[API Key: claude-coding<br/>scope: openai, github<br/>rate: 10 req/s]
        K2[API Key: codex-research<br/>scope: openai<br/>credential override: premium key]
        K3[API Key: support-bot<br/>scope: openai, anthropic<br/>rate: 5 req/s]
    end

    subgraph NyxID Backend
        AUTH[Auth Middleware<br/>extracts api_key_id]
        SCOPE[Scope Check<br/>effective allowed_service_ids]
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
    Auth->>Auth: Expand same-owner active platform ids when scoped and opted in
    Auth->>Scope: AuthUser { api_key_id, allowed_service_ids, ... }

    alt Service not in effective allowed_service_ids
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

### Auto-connected platform services

A restricted key can select individual auto-connected `UserService` IDs or set
`allow_auto_connected_services=true`. The effective allowlist is the explicit
`allowed_service_ids` union active rows whose `user_id` equals the key owner and
whose `source` is `auto_provision`. This durable grant follows new services and
replacement row IDs after reconciliation. Disabling the flag leaves explicit
selections intact. `allow_all_services=true` takes precedence; storing both flags
is allowed, and the platform grant becomes effective if the key is later narrowed.

`key_service::effective_allowed_service_ids` expands scope at API-key auth-context
construction, including MCP and channel relay issuance. Proxy, relay JWT, agent
binding and exact-approval consumers use the resulting IDs through their existing
checks. JWT/API-key authentication itself never provisions rows.
The indexed expansion query only runs for restricted, opted-in keys.

Ownership remains authoritative: a personal platform row cannot be selected for
an org key, and expansion never includes a different owner's rows. Version 0.20
includes explicit platform bindings in the auto-connected grant. Human and delegated key listing,
Agent Key login delivery (login options), and device-code approval/onboarding for
the acting person's own account invoke shared provisioning, which may idempotently
create org-owned auto-connected rows only through that person's own active
Member/Admin memberships with `can_proxy()` and explicit platform-key grants.
The 0.19.0 guarantee remains: org-targeted device approval/onboarding resolves
existing org services and never provisions rows for the target org as a side effect
of targeting. These rows appear as
auto-connected in org views. Removing an org grant immediately blocks execution
and the next per-owner reconciliation removes automatic rows and orphan endpoints.
Legacy no-auth/public-master provisioning remains personal-only during inherited
membership traversal.

```bash
nyxid api-key create --name research --scopes proxy --allowed-services github,autoplatform --terminal
nyxid api-key create --name platform-agent --scopes proxy --allow-auto-connected-services --terminal
nyxid api-key update KEY_ID --allow-auto-connected-services true
nyxid api-key update KEY_ID --allow-auto-connected-services false
nyxid device approve USER_CODE --service autoplatform --allow-auto-connected-services
```

`--allowed-services` accepts UUIDs or active service slugs. Slug resolution reads
`/keys`, errors on unknown or ambiguous slugs, and preserves UUID pass-through.
The browser wizard and mobile login approval expose the same individual and
durable selections. Scope plans retain explicit IDs separately from the current
implied service preview, so accepting a plan does not pin implied row identities.
Scheduled-invocation keys continue requiring exact service/operation grants.

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
        bool allow_auto_connected_services "default: false"
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
| `allow_auto_connected_services` | `bool` | `false` | Adds active same-owner auto-connected platform services to a restricted key, including future additions |
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

Optional metadata uses `serde(default)`; the platform-services grant is a defaulted boolean. Legacy keys keep their existing scope.

## API Endpoints

General agent keys can read org membership through `GET /api/v1/orgs`, `/orgs/{key}`, `/orgs/{key}/authorization`, `/orgs/{org_id}/members`, `/orgs/{org_id}/members/{member_id}/authorization`, and `/orgs/{org_id}/role-scopes` (all under `/api/v1`; `{key}` accepts UUID or slug). No extra scope or service allowlist is required; active membership governs reads and role scopes require admin. The actor is the key owner: person-owned keys see that person's memberships, while org-owned keys list their own org and receive Direct read access, projected as `your_role: "admin"` without a membership row. All writes, invites (including GET), and primary-org changes remain human-only for API keys. Scheduled-invocation keys remain rejected; delegated `account:read` parity is unchanged.

Agent keys can also GET `/keys`, `/keys/{id_or_slug}`, `/keys/{id_or_slug}/authorization`, `/user-services`, `/endpoints` (including authorized `?org_id=`), `/endpoints/{id}/authorization`, `/endpoints/{id}/openapi-endpoints`, `/api-keys/external`, `/api-keys/external/{id}/authorization`, and `/mcp/config`, plus `/catalog`, `/catalog/{slug}`, and `/catalog/{slug}/endpoints`. Catalog GETs require no proxy scope; template metadata and live platform availability do not expose instance overrides or connection state. Instance-backed private catalog access and mounted-spec fallback require an allowed backing UserService and the same Member/Admin org scopes as inventory. MCP `nyx__discover_services` gives unrestricted API keys the same discovery result as the owner's session; credential-free auto-connected templates remain suppressed for all callers. For restricted keys, instance-based suppression uses only visible instances. MCP discovery applies the same visibility rule as `GET /api/v1/catalog` for every caller: public and legacy rows, rows the actor created, and platform-key-enabled rows subject to live availability. `/mcp/config` requires `proxy` or `proxy:*` scope and matches stateless MCP discovery, including mounted instance specs; assistant chat keys discover services before acknowledgement, with grants enforced at execution. These API-key reads perform no auto-provisioning or lazy pending-OAuth reconciliation. Restricted keys are filtered to their effective service allowlist, including auto-connected expansion; endpoints and credentials must back at least one allowed service. Personal keys list personal and org-shared services through active Member/Admin memberships and effective role scopes; Viewer-only org services are excluded. Org-owned keys list their own services with the existing `credential_source.type: "personal"` tag. `/keys` still includes disabled rows within key scope. All inventory writes and the entire NyxID `/api-keys` management router remain human-only for API keys; delegated `account:read` parity is unchanged.

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
| `POST` | `/api/v1/api-keys` | Accepts optional `platform` and `allow_auto_connected_services` fields |
| `PUT` | `/api/v1/api-keys/{id}` | Accepts `rate_limit_per_second`, `rate_limit_burst`, `platform`, `allow_auto_connected_services` |

## CLI

### API Key Commands

```bash
# Create with optional platform label and service scope
nyxid api-key create --name "coding-agent" --platform claude-code \
  --allowed-services "openai,github" --terminal

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
| Platform-services grant | Absent `allow_auto_connected_services` means false. Scope-plan digests remain byte-identical when false or absent; true binds the durable grant and previews current platform rows. |
| Existing API keys | `allow_all_services=true`, `allow_all_nodes=true`, no rate limit override, no bindings. Behavior identical to before. |
| Existing auth paths (JWT, session, SA) | New `AuthUser` fields are `None`. No scope enforcement, no rate limit override. |
| No `--profile` flag | Reads from `~/.nyxid/` (unchanged). |
| No `agent_service_bindings` | Proxy uses default `UserService.api_key_id` (unchanged). |
| API responses | Additive grant fields and `allowed_services[].auto_connected` identify platform access. Existing clients may ignore the new fields. |

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

## Service-bound agent key introspection

A downstream resource server can verify an ordinary agent key directly through
`POST /oauth/introspect`. Send a form with `token`, `token_type_hint=api_key`,
`client_id`, `client_secret` and the target catalog `service_id`. The active
confidential OAuth client must appear in that catalog's `introspection_client_ids`. Administrators set or clear this field through the catalog service create/update API; it accepts active confidential clients for either public or private services and never changes visibility or automatic connections.
Public clients and unrelated confidential clients receive `active: false`.

Successful evidence includes `active`, `token_type: api_key`, `aud` (the catalog
service ID), `sub` (the exact key owner), `scope`, `jti`, `iat`, `iss` and current
RBAC `permissions`. `exp` is the earlier parent/child credential expiry; it is
omitted only for a nonexpiring credential. The resource server must validate
the audience and credential type before accepting permissions.

Verification reuses the live API-key authentication constructor and requires an
active owner, a general-purpose key with `proxy` scope, an active owner-bound
HTTP connection to the catalog, and a current key service grant. Connection IDs
and catalog grants remain distinct; auto-connected and platform-bound connections
also retain their current eligibility and platform access checks. Verification
does not provision connections or widen service scopes.

Only self-authenticating HTTP resource servers are eligible: the catalog must use
`auth_method: none`, must not require a user credential, and must have no provider
configuration or provider requirements. The live connection must likewise use
`auth_method: none` with no attached user credential or node route. Credential-bearing proxy
transports are refused even when their catalog has no explicit operation policy;
introspection cannot substitute for their credential admission. No-auth platform
ACLs still apply.

This is whole-service evidence. Keys with operation scopes, actual assistant
conversation authority, organization-specialist or group authority are refused;
their method, input, turn, guest and approval restrictions cannot be represented
by an audience and owner permissions alone. Restricted key purposes, Tool
offerings, provider services, retired services and services with a proxy operation
policy are also ineligible. This endpoint by itself therefore does **not** enable
NyxBot calls to an external resource server. Those calls require a separate proof
bound to the actual authorized request, preserving the existing proxy admission
and deferred-command authority. An ordinary-key success must not be reported as
NyxBot compatibility.

Invalid, revoked, expired, unsupported and out-of-scope keys return `active: false`;
unexpected database failures in agent-key verification return 503 so consumers
can distinguish an outage from revocation. Existing JWT introspection remains
unchanged, apart from the additive optional audience field.

## Request-bound NyxBot resource reads

The default-off `NYXID_RESOURCE_REQUEST_PROOF_ENABLED` flag adds a narrow read
transport for an owner's live, unstopped personal NyxBot turn. Both the HTTP
proxy and NyxBot's MCP service-call path retain their existing authorization,
approval, billing and final egress gates. Only direct GET/HEAD requests with an
empty body are eligible. The key must have current `proxy` and service access;
the target must be an active, provider-less, no-auth HTTP catalog connection with
JWT identity propagation and a registered confidential introspection client.
The authenticated child credential, when present, remains part of the binding.

After building the final HTTP request, NyxID signs a 60-second RS256 JWT with
`typ=nyxid-resource-request+jwt`. `aud` and `nyx_service_id` identify the immutable
catalog UUID. The dedicated `X-NyxID-Resource-Request` header is generated only
at this point; caller, catalog, user, delegation and node header layers cannot
supply it. Ordinary `X-NyxID-Identity-Token` propagation remains separate and
unchanged. The proof signs the exact final method, encoded path and raw query,
single Content-Type value and SHA-256 of the empty body. An Idempotency-Key,
method override, nonempty/streaming body or non-identity Content-Encoding is
refused. The final destination must match the admitted URL and redirects are
not followed.

A resource server sends the proof to authenticated `POST /oauth/introspect` with
`token_type_hint=resource_request`, its `client_id` and `client_secret`, catalog
`service_id`, `request_method`, `request_path_and_query`,
`request_body_sha256`, and optional `request_content_type`. NyxID checks its own
configured issuer, exact audience, signature, UUID `jti`, expiry, exact request
descriptor, client registration and current live authority. Successful evidence
has `token_type=resource_request`, `scope=proxy`, current subject and complete
RBAC permissions, the exact `resource_request` descriptor, and
`independent_commands=false`. Invalid or unsupported proofs return
`{"active":false}`; unexpected provider failures return 503. There is no
completed-proof verification cache. Identical reads can be retried during the
short lifetime only while all current authority checks still pass.

The initial slice requires the current service approval policy to resolve to
AutoAllow. A new Deny or RequireApproval policy invalidates an issued proof;
explicit per-request approvals are not consumed again or translated into wider
authority. The shared approval evaluator matches method/path/verb and currently
has no query/body predicates; raw query remains bound by the proof. Guest,
specialist, organization-agent, node, mutation, durable-command and independent
continuation authority are unsupported. A resource server must never retain
the proof as a reusable API credential, accept it as ordinary Bearer authority,
or use it for a mutation. When the explicit proof header is present, malformed,
expired, unavailable or disabled proof verification must fail closed without
falling back to ambient bearer/cookie authority.

Deploy all resource verifiers and all NyxID introspection readers before
enabling issuance. Register the catalog's confidential client, enable the
resource verifier, then enable the NyxID flag and validate isolated requests.
Rollback stops issuance first and keeps verification available for the maximum
60-second lifetime. This integration changes no sandbox, PV or running agent
session. Ordinary-key introspection by itself remains insufficient evidence of
NyxBot compatibility; the exact MCP/HTTP request path must also pass acceptance.

### NyxBot access authority

Specialist agents cannot widen access or grant permissions. For work NyxBot assigned, or in a personal group NyxBot is a member of, access requests go through NyxBot and the backend rejects card and connection-link creation from specialist chat keys with the stable `access_request_denied` refusal. When a person talks to a specialist directly, that work is not NyxBot's: an ungranted call returns a neutral `missing_access` result (scope, target, "no request was created", a link to the agent's details) and the specialist continues; it never asks NyxBot to take over. Only the owner, in the agent's details, or a legitimately involved NyxBot under routed mode can widen a specialist's access.
