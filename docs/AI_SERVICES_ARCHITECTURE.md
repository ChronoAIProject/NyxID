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
        CATALOG["Service Catalog<br/>19 seeded + custom"]
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
        LOGIN["nyxid login<br/>Browser SSO"]
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
