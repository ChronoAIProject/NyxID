# AI Services Architecture

## Overview

NyxID's AI Services system lets users manage external API credentials, SSH services, and proxy routing through a unified interface. Users interact via the **AI Services page** (`/keys`) or the **`nyxid` CLI**.

---

## System Components

```mermaid
graph TB
    subgraph "User Tools"
        CLI["nyxid CLI<br/>Login, manage services,<br/>API keys, proxy requests"]
        WEB["AI Services Page<br/>/keys<br/>2 tabs: External Services + API Keys"]
        AI["AI Agent<br/>Uses nyxid CLI via<br/>playbook/skills"]
    end

    subgraph "NyxID Backend"
        API["REST API<br/>/api/v1/*"]
        PROXY["Proxy Engine<br/>/proxy/s/{slug}/*"]
        CATALOG["Service Catalog<br/>Default-seeded + custom"]
        AUTH["Auth + JWT<br/>SSO, MFA, Sessions"]
    end

    subgraph "User Data (4 collections)"
        UE["UserEndpoint<br/>Target URLs"]
        UAK["UserApiKey<br/>Credentials"]
        US["UserService<br/>Routing Config"]
        ASB["AgentServiceBinding<br/>Per-agent credential overrides"]
    end

    subgraph "Node Infrastructure"
        NODE_CLI["nyxid node subcommand<br/>Register, credentials,<br/>OAuth, SSH"]
        NODE["Node Agent<br/>WebSocket connection<br/>Local credential store"]
        TARGET["Target Service<br/>API endpoint or<br/>SSH server"]
    end

    CLI --> API
    WEB --> API
    AI --> CLI

    API --> UE & UAK & US
    API --> CATALOG
    API --> AUTH

    PROXY --> US
    US --> UE
    US --> UAK

    PROXY -->|"Direct"| TARGET
    PROXY -->|"Via Node"| NODE
    NODE --> TARGET

    NODE_CLI --> NODE

    ASB --> US
    ASB --> UAK
```

## Service-Pool Routing Boundary

NyxID#974 was narrowed to a routing proof before adding a user-facing pool
surface. The proof is recorded in
[SERVICE_POOL_ROUTING_PROOF.md](SERVICE_POOL_ROUTING_PROOF.md).

The important boundary is that `UserService` remains the concrete proxy target
member, while any future `ServicePool` must be selected inside
`proxy_service::resolve_proxy_target_from_user_service()`. The existing
`node_routing_service::resolve_node_route()` / `fallback_node_ids` layer remains
node failover below a selected `UserService`; it is not sufficient by itself to
balance multiple endpoint/credential instances behind one stable slug.

## Data Model Relationships

```mermaid
erDiagram
    ServiceCatalog ||--o{ UserEndpoint : "defaults from"
    ServiceCatalog ||--o{ UserService : "catalog_service_id"

    UserEndpoint ||--o{ UserService : "endpoint_id"
    UserApiKey ||--o{ UserService : "api_key_id"

    ApiKey ||--o{ UserService : "scope controls access"

    UserService {
        string id PK
        string user_id
        string slug "auto-generated"
        string endpoint_id FK
        string api_key_id FK
        string auth_method "bearer, header, query, etc"
        string auth_key_name "Authorization, X-API-Key, etc"
        string node_id FK "optional: route via node"
        string service_type "http or ssh"
        string catalog_service_id FK "optional: from catalog"
        bool is_active
    }

    UserEndpoint {
        string id PK
        string user_id
        string url "target URL (may be empty on NyxID when the node stores it locally)"
        string label
        string catalog_service_id FK "optional"
    }

    UserApiKey {
        string id PK
        string user_id
        string credential_type "api_key, oauth2, bearer, node_managed, ssh_certificate"
        bytes credential_encrypted "optional if node-managed"
        string status "active, expired, revoked, pending_auth"
    }

    ApiKey {
        string id PK
        string user_id
        string name
        string scopes "proxy read write"
        bool allow_all_services
        bool allow_all_nodes
        string allowed_service_ids "UserService IDs"
        string allowed_node_ids "Node IDs"
        int rate_limit_per_second "optional per-agent"
        int rate_limit_burst "optional per-agent"
        string platform "claude-code, codex, etc"
    }

    AgentServiceBinding {
        string id PK
        string api_key_id FK
        string user_id
        string user_service_id FK
        string user_api_key_id FK
        datetime created_at
        datetime updated_at
    }
    ApiKey ||--o{ AgentServiceBinding : "has bindings"
    UserService ||--o{ AgentServiceBinding : "bound to"
    UserApiKey ||--o{ AgentServiceBinding : "overrides with"

    ServiceCatalog {
        string slug PK
        string name "OpenAI, Anthropic, etc"
        string base_url "default endpoint"
        string service_type "http or ssh"
        string provider_type "api_key, oauth2, device_code"
        string auth_method "default auth method"
    }
```

## Service Lifecycle: Disable vs Delete

There are exactly **two** lifecycle actions on a connection, and they are
exposed under exactly two names on every surface. Use these words — the UI
previously carried four (`Deactivate`/`Activate`/`Pause` and `Revoke`/`Delete`)
for these two actions, which made them read as more than two things.

| | Verb | Reversible | Effect |
|---|---|---|---|
| Pause | **Disable** / **Enable** | Yes | `UserService.is_active = false`. Nothing else is touched. |
| Remove | **Delete** | No | Hard-deletes `UserApiKey` + `UserEndpoint`, cleans agent bindings and org role scopes, optionally revokes upstream. |

Both make the service unusable: the proxy, MCP catalog, discovery and scope
checks all resolve through active-only queries, so a disabled service 404s
exactly like a deleted one.

Naming note: `revoked` is a **credential status** (`UserApiKey.status`) that a
card can render *while the service is otherwise fine*. Keep it out of button
labels for the delete action or the two meanings collide.

### `DELETE /user-services/{id}` is a misnamed disable

Despite the verb it calls `deactivate_user_service`: it sets `is_active = false`
and cleans agent bindings and org role scopes, but **keeps the credential and
endpoint**. Nothing in the product calls it — not the frontend, CLI, mobile or
SDK — and it is reachable only by direct API use.

Before disabled services were listed, this endpoint looked like a delete: the
row vanished while its credential stayed stored. It now correctly shows up as
`Disabled`. Prefer `PUT /user-services/{id} {is_active: false}` to disable and
`DELETE /keys/{id}` to actually delete.

### Delete leaves a tombstone

`Delete` soft-deletes the `UserService` row (`is_active = false`) and hard-deletes
the credential and endpoint. The tombstone is invisible everywhere, including
the management listing, because `list_keys` drops any row whose endpoint is
missing.

### Two listings, deliberately different

| Function | Includes disabled? | Used by |
|---|---|---|
| `list_user_services_with_sources` | No | proxy discovery, MCP catalog, OAuth resource indicators, API-key scope, assistant readiness |
| `list_user_services_with_sources_including_disabled` | **Yes** | `unified_key_service::list_keys` → `GET /keys` **only** |

A disabled service must stay in the management listing or the pause is
unreversible in the product — it would vanish from the screen carrying the
Enable control. It must stay out of every other listing or a disabled service
becomes reachable by an agent.

**Never** point a credential-resolving or catalog path at the
`_including_disabled` variant.

### Resolution asymmetry (do not "fix" this)

`find_user_service_for_actor` (`handlers/keys.rs`, the `/keys/{id_or_slug}`
resolver) matches a **disabled row by UUID but not by slug**:

- **UUID → resolves.** `/keys/{id}` is the management path that hosts Enable.
- **Slug → does not.** Slugs are unique only among *active* rows (the
  `user_services` unique index is partial on `is_active: true`), so a disabled
  slug match is ambiguous — and slug is the shape the proxy uses.

This asymmetry has been removed once before, by `c63ab733`, on the reasoning
that the UUID path was the odd one out. The paths it compared against were
already closed, which is precisely why this one was load-bearing: it was the
last route to the Enable control, so closing it made Disable a one-way door for
five months. `get_key_resolves_disabled_service_by_uuid_but_not_by_slug` asserts
both halves.

### API contract for consumers

`GET /keys` returns disabled services. **Anything consuming it must read
`is_active`** rather than assuming every row is usable — including when
rendering status, since `status` is the *credential's* status and stays healthy
(`active`) while the service is disabled. The CLI centralises this in
`commands::service::display_status`; the frontend renders a `Disabled` badge in
the card, table and chat-plugin surfaces.

Both `GET /keys` and `GET /keys/{id}` keep a service visible when its stored
`api_key_id` no longer resolves. Such rows return `credential_missing: true`;
consumers must present that separately from `credential_type: "none"`, which is
also the healthy representation for a service that requires no credential.
The degraded row may be reconnected or deleted, but it cannot be enabled until
a replacement credential has been attached.

Auto-connected rows (`auto_connected: true`) are platform managed. Their
`endpoint_url` is omitted from both key-list and key-detail responses; clients
must render a neutral platform-managed label rather than treating the missing
field as an unknown or empty user endpoint. `GET /endpoints` follows the same
contract by returning `auto_connected: true` and omitting `url`. The stored
`UserEndpoint.url` remains intact for proxy routing and MCP configuration, and
user-facing endpoint or service mutation routes reject changes to these rows.

## Platform-managed (master-credential) catalog services

A catalog row is platform managed when it is active, public, internal,
provider-less HTTP; has a non-`none`, non-`token_exchange` auth method; does not
require a user credential; contains an encrypted platform credential; and has
no `ServiceProviderRequirement`. This is the master-credential branch of
`is_auto_provisionable_catalog_service`. NyxID auto-provisions one
`UserEndpoint` and `UserService` per owner with `source = "auto_provision"` and
no `UserApiKey`. The proxy resolves the credential from the catalog row.

The route's identity policy is also catalog-owned. An admin update to any of
the eight identity fields (`identity_propagation_mode`, the three
`identity_include_*` flags, `identity_jwt_audience`, `forward_access_token`,
`inject_delegation_token`, and `delegation_token_scope`) calls
`catalog_identity_service::propagate_catalog_update`. Inherited instances are
backfilled and the reconciliation audit reports `matched_count`,
`modified_count`, and `skipped_customized_count`. A same-value PUT is a no-op
because the computed `changes` set is empty. The admin-only
`POST /api/v1/services/{id}/resync-identity` recovery operation instead
overwrites all eight fields on every instance, including customized rows.

`POST /api/v1/keys` refuses a user-created alias of one of these services with
403 / `platform_managed_catalog_service` (11800). A user-created key may carry
an `endpoint_url`; serving the platform credential through that alias would
therefore send the platform secret to a user-controlled destination. Supplying
a user credential, node route, endpoint override, reserved service ID, or org
owner does not weaken the refusal.

Consumers detect these rows through `GET /keys` and its `auto_connected`
field. `GET /user-services` deliberately does not carry that projection; a
consumer starting there must join by the exact `UserService._id`, never infer
platform management from a slug or a missing API-key ID.

Privilege can be narrowed today with `proxy_operation_policy`: every rule is
an explicit HTTP method plus a root-anchored path template whose `{param}`
placeholders match exactly one segment. Wildcards are not supported. Replace a
policy with an object; roll it back with
`{"proxy_operation_policy": null}`. An empty `rules` array is rejected because
it would deny every operation. `visibility = "private"` is not a narrowing
tool for this class: the predicate requires `public`, so flipping visibility
de-provisions every auto-connected row. Per-caller route flags also do not
narrow the platform credential's privilege: forwarded bearer and delegation
tokens are minted for the current user on each request.

### Operator runbook

1. Read the catalog row and its exact auto-connected `UserService` IDs. Before
   tightening a policy, migrate any existing consumer that still holds a
   user-created alias to deterministic exact-ID resolution.
2. PUT only the intended identity fields and/or a non-empty
   `proxy_operation_policy` to `/api/v1/services/{id}`.
3. Review the identity reconciliation audit counts. Investigate every
   `skipped_customized_count`; use `resync-identity` only when deliberately
   overwriting all customized instances.
4. Re-read the catalog row and `GET /keys` projections, then verify allowed and
   denied method/path pairs through the exact auto-connected routes.

Because a disabled row keeps its slug while a new active service may reuse it,
this listing can contain two rows with the same slug. Consumers that resolve by
slug should prefer the active row.

### Known gaps

1. Creating a service on a disabled row's slug is accepted, then re-enabling the
   old row fails with a raw `E11000` surfaced as a 500. Should be a clean 409 at
   create time.
2. A deleted tombstone can be flipped back to `is_active = true` via
   `PUT /user-services/{id}`, which does not check for the hard-deleted endpoint
   and credential. The result is a zombie: listed in `/user-services`, absent
   from `/keys`, 500 at the proxy.

## Proxy Request Flow

```mermaid
sequenceDiagram
    participant User as User / AI Agent
    participant CLI as nyxid CLI
    participant API as NyxID API
    participant Approval as Approval Check
    participant US as UserService
    participant UE as UserEndpoint
    participant UAK as UserApiKey
    participant Node as Node Agent
    participant Target as Target Service

    User->>CLI: nyxid proxy request openai /chat/completions -d '{...}'
    CLI->>API: POST /proxy/s/openai/chat/completions<br/>Authorization: Bearer {access_token}

    API->>US: Find UserService by slug + user_id
    US-->>API: endpoint_id, api_key_id, auth_method, node_id

    API->>Approval: Check approval requirement + mode
    alt Approval required (per_request mode, default)
        Approval-->>API: Build action_description, create request, notify user
        API-->>CLI: 403 approval_required (request_id, action_description)
        Note over User: User approves via mobile/Telegram
        User->>CLI: (retry after approval)
        CLI->>API: (retry request)
    else Approval required (grant mode)
        Approval-->>API: Check for existing grant
        Note over Approval: If no grant, create request + notify
    else No approval required
        Note over Approval: Pass through
    end

    alt Direct Routing (no node_id)
        API->>UE: Get endpoint URL
        API->>UAK: Decrypt credential
        API->>Target: Forward request with credential injected
        Target-->>API: Response
    else Via Node (node_id set)
        API->>Node: Send proxy request via WebSocket
        Note over Node: Node resolves URL + credential locally
        Node->>Target: Forward request
        Target-->>Node: Response
        Node-->>API: Forward response
    end

    API-->>CLI: Response
    CLI-->>User: Display result
```

## Two Routing Modes

```mermaid
graph LR
    subgraph "Direct Routing"
        D_USER["User"] -->|"credential on NyxID"| D_NYXID["NyxID Backend"]
        D_NYXID -->|"injects credential"| D_TARGET["Target API"]
    end

    subgraph "Node Routing"
        N_USER["User"] -->|"no credential on NyxID"| N_NYXID["NyxID Backend"]
        N_NYXID -->|"WebSocket"| N_NODE["Node Agent"]
        N_NODE -->|"injects credential locally"| N_TARGET["Target API"]
    end
```

| Aspect | Direct | Via Node |
|--------|--------|----------|
| Credential stored on | NyxID backend (encrypted) | Node agent (local, encrypted) |
| Endpoint URL | NyxID (UserEndpoint) | Node agent (local config) |
| OAuth refresh | NyxID backend | Node agent locally |
| Use case | Cloud services, simple setup | Self-hosted, privacy-sensitive |

## CLI Tools

```mermaid
graph TB
    subgraph "nyxid CLI (user operations)"
        LOGIN["nyxid login<br/>Browser SSO / --device / --password"]
        CATALOG["nyxid catalog list/show<br/>Browse services"]
        SERVICE["nyxid service add/list/show/delete<br/>Manage AI services"]
        APIKEY["nyxid api-key create/list/rotate/delete<br/>Manage API keys with scope"]
        PROXY["nyxid proxy request/discover<br/>Make proxy requests"]
        SSH_CMD["nyxid ssh exec/terminal/issue-cert<br/>SSH operations"]
        MCP["nyxid mcp config<br/>Generate AI tool configs"]
        NODE_CMD["nyxid node list/show/register-token<br/>Manage nodes"]
        OPENCLAW["nyxid openclaw setup<br/>OpenClaw integration"]
    end

    subgraph "nyxid node subcommand (node agent)"
        REGISTER["nyxid node register<br/>Register with NyxID"]
        START["nyxid node start<br/>Start WS connection"]
        SETUP["nyxid node credentials setup<br/>Catalog-guided local setup"]
        CREDS["nyxid node credentials add<br/>Add API key credentials"]
        OAUTH_NODE["nyxid node credentials add-oauth<br/>Local OAuth flow"]
        OC_NODE["nyxid node openclaw connect<br/>OpenClaw via node"]
    end

    LOGIN --> SERVICE
    CATALOG --> SERVICE
    SERVICE --> PROXY
    NODE_CMD -.->|"register-token"| REGISTER
    REGISTER --> START
    START --> SETUP & CREDS & OAUTH_NODE & OC_NODE
```

## API Key Scoping

```mermaid
graph TB
    AK["API Key<br/>nyxid_abc123..."]

    AK -->|"allow_all_services: true"| ALL["Can access ALL services"]
    AK -->|"allow_all_services: false"| SCOPED["Restricted to specific services"]

    SCOPED --> S1["UserService: llm-openai"]
    SCOPED --> S2["UserService: api-github"]
    SCOPED -.-x S3["UserService: llm-anthropic (blocked)"]

    AK -->|"allow_all_nodes: true"| ALL_N["Can route via ALL nodes"]
    AK -->|"allow_all_nodes: false"| SCOPED_N["Restricted to specific nodes"]

    style S3 fill:#f66,stroke:#333,stroke-dasharray: 5
```

## Adding a Service: User Flows

```mermaid
flowchart TD
    START["User wants to add an AI service"]

    START --> HOW{"How?"}
    HOW -->|"CLI"| CLI_ADD["nyxid service add llm-openai"]
    HOW -->|"Web UI"| UI_ADD["AI Services page > + Add Service"]
    HOW -->|"AI Agent"| AI_ADD["Paste prompt into AI assistant"]

    CLI_ADD --> ROUTE{"Routing?"}
    UI_ADD --> ROUTE
    AI_ADD -->|"AI runs CLI"| CLI_ADD

    ROUTE -->|"Direct"| DIRECT["Enter credential<br/>(API key, OAuth, device code)"]
    ROUTE -->|"Via Node"| NODE["Select node<br/>Configure on node agent"]

    DIRECT --> DONE["Service created<br/>Ready to proxy"]
    NODE --> NODE_SETUP["Run on node:<br/>nyxid node credentials setup --service <slug><br/>or use add/add-oauth for manual setup"]
    NODE_SETUP --> DONE

    style DONE fill:#4f8,stroke:#333
```

## Agent Isolation Data Flow

When a proxy request arrives with an agent-scoped API key (`nyxid_ag_` prefix):

1. **Auth middleware** (`mw/auth.rs`) resolves the API key, extracts `api_key_id` and `api_key_name` into `AuthUser`
2. **Per-agent rate limiter** (`mw/rate_limit.rs`) checks per-agent rate limits from `ApiKey.rate_limit_per_second` / `rate_limit_burst` if configured, using a separate bucket per `api_key_id`
3. **Proxy handler** (`handlers/proxy.rs`) passes `AuthUser` to credential resolution
4. **Credential resolution** checks `agent_service_bindings` for a binding matching `(api_key_id, user_service_id)`
5. **If binding exists**: Uses the override `user_api_key_id` instead of the service's default credential
6. **If no binding**: Falls back to the service's default `api_key_id`
7. **Response header**: `X-NyxID-Agent-Id` is returned on proxy responses when the request was made with an API key
8. **Audit logging** includes `api_key_id` and `api_key_name` in event data for per-agent attribution

```mermaid
sequenceDiagram
    participant Agent as AI Agent
    participant API as NyxID API
    participant RL as Rate Limiter
    participant US as UserService
    participant ASB as AgentServiceBinding
    participant UAK as UserApiKey
    participant Target as Target Service

    Agent->>API: POST /proxy/s/llm-openai/chat/completions<br/>X-API-Key: nyxid_ag_...
    API->>API: AuthUser { api_key_id, api_key_name, rate_limit_* }
    API->>RL: Check per-agent rate limit
    RL-->>API: Allowed
    API->>US: Find UserService by slug + user_id
    API->>ASB: Lookup (api_key_id, user_service_id)
    alt Binding found
        ASB-->>API: Override user_api_key_id
        API->>UAK: Decrypt override credential
    else No binding
        API->>UAK: Decrypt default credential
    end
    API->>Target: Forward request with credential
    Target-->>API: Response
    API->>API: Audit log { api_key_id, api_key_name }
    API-->>Agent: Response + X-NyxID-Agent-Id header
```
