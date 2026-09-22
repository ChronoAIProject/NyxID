# AI Services Architecture

Admin service creation, provider linking, and legacy vendor retirement are documented in [SERVICE_CONFIGURATION.md](SERVICE_CONFIGURATION.md).

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
        bool allow_auto_connected_services
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

### Authentication classes

General API keys can GET `/keys`, `/keys/{id_or_slug}`, `/keys/{id_or_slug}/authorization`,
`/user-services`, `/endpoints`, `/endpoints/{id}/authorization`,
`/endpoints/{id}/openapi-endpoints`, `/api-keys/external`, and
`/api-keys/external/{id}/authorization` under `/api/v1`, without an extra scope.
API-key reads never auto-provision or reconcile services and never lazily reconcile
pending OAuth placeholders. Sessions, access JWTs, and delegated `account:read`
tokens retain their existing behavior, including provisioning and reconciliation.

Restricted keys see only their effective `UserService` allowlist, including the
existing auto-connected expansion. Endpoint and external-credential reads require
a backing allowed service. Resolvable out-of-allowlist details return 403
`ApiKeyScopeForbidden`; missing resources retain their normal 404 behavior.
Personal keys list personal and org-shared services through active Member/Admin
memberships and effective role scopes; Viewer-only org services are excluded for
API keys. Org-owned keys act as the org and list its own rows with the existing
`credential_source.type: "personal"` tag because the actor is the owner.
`/endpoints?org_id=` still requires Direct or org-admin access.
The endpoint and external-credential lists retain their existing owner selection:
`/endpoints` defaults to the actor's own endpoints, and `/api-keys/external` lists
the actor's own credentials. Org-shared service discovery uses `/keys` and
`/user-services`; backing-resource detail reads enforce membership ACLs and key scope.

All inventory writes and the entire NyxID `/api-keys` management router remain
human-only for API keys. Relay and scheduled-invocation tokens remain denied on inventory reads; delegated read parity is unchanged. Service accounts have a separate exact-UUID `GET /keys/{id}` metadata projection, requiring `user-services:read`, a live admin-issued key read grant, and current owner access. It performs no credential resolution or reconciliation; other inventory routes remain denied. See `SERVICE_ACCOUNTS.md` → Connection metadata reads.

### API contract for consumers

`GET /keys` returns disabled services, subject to API-key allowlist filtering.
**Anything consuming it must read
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

Because a disabled row keeps its slug while a new active service may reuse it,
this listing can contain two rows with the same slug. Consumers that resolve by
slug should prefer the active row.

Automatic catalog provisioning checks active rows and user-disabled rows whose
endpoint still exists, for both personal and org owners. Disabled connections
continue to block provisioning, preserving the user's choice and allowing Enable
on the original slug. A deleted tombstone (inactive non-auto row with its endpoint
gone) does not block a fresh automatic connection; the active-only slug lookup and
partial `(user_id, slug)` index let it reuse the catalog slug. We retain non-auto
tombstones rather than reviving deleted endpoints or changing a user's binding.
Inactive automatic rows are reconciled away to release their unique
`(source, source_id)` before recreation.

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
        LOGIN["nyxid login<br/>device-code (default) / --callback / --password"]
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

    SCOPED -->|"allow_auto_connected_services: true"| PLATFORM["Active same-owner auto-connected services<br/>including future additions"]
    SCOPED --> S1["UserService: llm-openai"]
    SCOPED --> S2["UserService: api-github"]
    SCOPED -.-x S3["UserService: llm-anthropic (blocked)"]

    AK -->|"allow_all_nodes: true"| ALL_N["Can route via ALL nodes"]
    AK -->|"allow_all_nodes: false"| SCOPED_N["Restricted to specific nodes"]

    style S3 fill:#f66,stroke:#333,stroke-dasharray: 5
```

For a restricted key, `allowed_service_ids` stores explicit selections.
`allow_auto_connected_services` (default false) adds the IDs of active
`UserService` rows with the same owner and `source = "auto_provision"` whenever
NyxID constructs its auth scope. This union is evaluated on each key
request, so reconciliation can replace row UUIDs and new platform services can
appear without rewriting the key. No provisioning runs on the authentication
path. `allow_all_services` takes precedence, while both flags may be stored.

Service pickers show platform rows separately and offer both individual
selection and an “Allow all auto-connected platform services (includes ones
added later)” control. Turning that control off restores the explicit choices.
Org keys only see their own rows; personal platform services cannot cross the
owner boundary. API responses badge explicit selections with
`allowed_services[].auto_connected`. Scope plans expose the durable flag and
annotate their current service preview, while preserving the historical digest
for false/absent flags.

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

## Platform credential binding (0.20)

`UserService.credential_binding` is optional (`platform` or `user`). Absent keeps
legacy resolution: no `api_key_id` plus `source=auto_provision` selects the historical
platform path; other rows use the user path. Catalog `platform_key` grants are live,
owner-scoped, and independent of catalog provider linkage. Person UUIDs grant that
person; org UUIDs grant proxy-capable active members and org-owned connections.
Public grants auto-connect active people in their personal section only. Restricted
grants auto-connect directly allowlisted people and org owners reached through active
`can_proxy()` memberships. Reconciliation and the idempotent startup sweep remove
public-audience org automatic rows and orphan endpoints. Explicit org platform
bindings retain public execution access. Explicit connections remain manageable
after revocation but cannot execute.

`POST /keys {service_slug, label, use_platform_key:true}` creates a server-held
connection without credential, OAuth, destination override or node inputs.
`PUT /keys/{id} {use_platform_key:true}` switches an existing row after live ACL
validation. `false` requires a fresh credential or the existing OAuth-provider flow.
The previous personal key row is retained when switching to platform. Disable/Enable
and Delete keep their existing meanings. Platform-bound connections use the live
catalog URL/auth, never a user-controlled destination or credential node. Normal
routing edits require switching back to BYOK.

`GET /keys` adds `credential_binding`, `platform_key_available`,
`platform_key_pricing`, and `byok_pricing`. Agent keys with
`allow_auto_connected_services` include active same-owner platform-bound rows in
their effective service union, including explicit selections. Grants do not override
normal org membership, operation policy or delegated execution checks. See
[the design](PLATFORM_KEYS_AND_INFERENCE.md) for complete pricing and upgrade rules.

### Credential replacement and admin catalog editing

Admin catalog `PUT /services/{catalog-id}` accepts a write-only master `credential`
through envelope encryption and metadata-only auditing; it never returns credential
bytes or lengths. Absent legacy public master configurations display as “enabled,
public (implicit)”. See [credential replacement and audit](PLATFORM_KEYS_AND_INFERENCE.md#credential-replacement-and-audit).

### Editing platform connections

Explicit platform connections allow label, admin-only visibility, recommended skills,
User-Agent and default-header edits, plus Disable/Enable. Endpoint/auth/node/identity/
delegation settings require switching to a user key; automatic rows stay managed.

### Node transport hardening

Platform master credentials, including legacy internal master rows, always use server
transport. Existing owner-node bindings for those rows are ignored as intentional
hardening; nodes inject their own credentials only.

### Org provisioning and reconciliation

Human and delegated key listing, Agent Key login delivery (login options), and device-code
approval/onboarding for the acting person's own account invoke shared provisioning,
which may idempotently create org-owned auto-connected rows only through that
person's own active Member/Admin memberships with `can_proxy()` and explicit
platform-key grants. The 0.19.0 guarantee remains: org-targeted device
approval/onboarding resolves existing org services and never provisions rows for
the target org as a side effect of targeting. Org views identify
them as auto-connected. Removing the org grant immediately blocks execution and the
next owner reconciliation removes automatic rows and orphan endpoints. This side
effect is limited to explicit platform configurations; inherited legacy no-auth
provisioning remains personal-only. JWT/API-key authentication itself never provisions rows.

Key listing shares one membership and active-owner grant snapshot across org row
loading and availability rendering. Human and delegated callers also reuse it for
personal/org provisioning and stale-row reconciliation; API-key reads skip both.
Provider eligibility is batch-loaded once for the request. Catalog, MCP and LLM
listings likewise reuse grants and provider rows rather than issuing ACL queries per
service. These snapshots last for one request only; the next request rechecks live
membership, owner activity, provider eligibility and catalog configuration.

## Aurinko account credentials

`api-aurinko` is a normal owner-scoped catalog connection backed by an encrypted account bearer credential. Existing active-service, agent-binding, scope, and approval rules apply. The AI Services UI and CLI support account-token entry and the authenticated `/v1/account` probe. The email channel bot stores its own encrypted account token plus the separate application signing secret; credential rotation and deletion are independent across these surfaces. Managed OAuth is not exposed because official Aurinko contracts do not document the PKCE support required by NyxID. See [Aurinko integration](./AURINKO_INTEGRATION.md) for the documented contracts and decision.

## Service authorship and history

Service cards and tables include authorized creator/latest-editor summaries. Instance detail pages, including platform-managed instances, have a History tab. Deleted UUID histories remain discoverable from Services → Deleted service history under current personal-owner/org-admin/resource-scope checks. The transactional journal covers service, endpoint and credential writers; ordinary timestamps, usage and routine refresh do not count as configuration edits. See [SERVICE_HISTORY.md](SERVICE_HISTORY.md) for capture, safe values, writer inventory, audit publication and required MongoDB replica-set migration.
