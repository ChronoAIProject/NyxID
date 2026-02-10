# Architecture: Service Proxy Overhaul

## Overview

This document describes the redesign of NyxID's service model, connection flows, and proxy layer. The goals are:

1. **Separate OIDC provider configurations from connectable services** so OIDC services no longer appear in the user "Connections" page.
2. **Make "connect" meaningful** by collecting per-user credentials appropriate to each service's auth type.
3. **Support internal services** that use the master credential without requiring per-user keys.
4. **Enhance the MCP proxy** with proper credential validation, tool filtering, and search/discovery.
5. **Enforce connection-awareness in the REST proxy** so requests are only proxied for properly connected users.

---

## 1. Data Model Changes

### 1.1 `DownstreamService` - Add `service_category`

**File:** `backend/src/models/downstream_service.rs`

Add a `service_category` field to distinguish service roles. This approach avoids creating a separate collection while cleanly partitioning behavior.

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownstreamService {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub base_url: String,
    pub auth_method: String,
    pub auth_key_name: String,
    pub credential_encrypted: Vec<u8>,
    #[serde(default)]
    pub auth_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_spec_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_id: Option<String>,

    // --- NEW FIELDS ---

    /// "provider" | "connection" | "internal"
    /// - provider: OIDC services where NyxID is the identity provider (not user-connectable)
    /// - connection: external services users connect to with their own credentials
    /// - internal: internal services using master credential (users just enable access)
    #[serde(default = "default_service_category")]
    pub service_category: String,

    /// Whether this service requires per-user credentials to connect.
    /// - true for connection services (api_key, bearer, basic)
    /// - false for internal services and provider services
    /// Derived from service_category + auth_type but stored for query efficiency.
    #[serde(default = "default_true")]
    pub requires_user_credential: bool,

    pub is_active: bool,
    pub created_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

fn default_service_category() -> String {
    "connection".to_string()
}

fn default_true() -> bool {
    true
}
```

**Rationale for `service_category` as a field vs. separate collection:**
- Keeps all services in one collection for simplicity
- Existing queries, indexes, and the `ServiceEndpoint` foreign key all stay valid
- Clean filtering by category in list queries
- No schema migration beyond adding fields with defaults

**Category rules:**
| `service_category` | `auth_type` values | `requires_user_credential` | User can connect? |
|---|---|---|---|
| `provider` | `oidc` | `false` | No (admin-managed) |
| `connection` | `api_key`, `bearer`, `basic`, `oauth2` | `true` | Yes (must supply credential) |
| `internal` | `api_key`, `bearer`, `basic`, `header` | `false` | Yes (enable only, no credential) |

### 1.2 `UserServiceConnection` - Add credential fields

**File:** `backend/src/models/user_service_connection.rs`

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserServiceConnection {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub service_id: String,
    /// Per-user encrypted credential for this service.
    /// - For "connection" services: required, contains the user's own key/token/password
    /// - For "internal" services: None (master credential used)
    pub credential_encrypted: Option<Vec<u8>>,
    /// Stores what kind of credential is stored, for display purposes.
    /// e.g., "api_key", "bearer", "basic"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_type: Option<String>,
    /// Optional label the user gives their credential (e.g., "Production Key")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_label: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub is_active: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
```

### 1.3 MongoDB Indexes

**File:** `backend/src/db.rs` (in `ensure_indexes()`)

Add a new index for category-filtered service queries:

```rust
// Existing indexes stay. Add:
// downstream_services: category + is_active for filtered listing
downstream_services_coll.create_index(
    IndexModel::builder()
        .keys(doc! { "service_category": 1, "is_active": 1 })
        .build()
).await?;
```

### 1.4 No new collections

All changes are additive fields on existing collections. No new collections needed.

---

## 2. Service Layer Changes

### 2.1 Connection Service (new file)

**File:** `backend/src/services/connection_service.rs` (new)

Extract connection business logic from the handler into a dedicated service. This enables reuse by the proxy handler and MCP config handler.

```rust
pub struct ConnectionResult {
    pub connection_id: String,
    pub service_name: String,
    pub connected_at: DateTime<Utc>,
}

/// Connect a user to a service with credential validation.
///
/// For "connection" category services: `credential` is required.
/// For "internal" category services: `credential` must be None.
/// For "provider" category services: returns error (not connectable).
pub async fn connect_user(
    db: &Database,
    encryption_key: &[u8],
    user_id: &str,
    service_id: &str,
    credential: Option<&str>,
    credential_label: Option<&str>,
) -> AppResult<ConnectionResult> { ... }

/// Update the credential on an existing connection.
pub async fn update_credential(
    db: &Database,
    encryption_key: &[u8],
    user_id: &str,
    service_id: &str,
    credential: &str,
    credential_label: Option<&str>,
) -> AppResult<()> { ... }

/// Disconnect a user from a service.
/// Securely zeroes the credential_encrypted field before deactivating.
pub async fn disconnect_user(
    db: &Database,
    user_id: &str,
    service_id: &str,
) -> AppResult<()> { ... }

/// Check if a user has a valid connection (with credential if required).
pub async fn validate_connection(
    db: &Database,
    user_id: &str,
    service_id: &str,
) -> AppResult<UserServiceConnection> { ... }
```

### 2.2 Proxy Service Changes

**File:** `backend/src/services/proxy_service.rs`

The existing `resolve_proxy_target` must be updated to enforce connection-awareness:

```rust
pub async fn resolve_proxy_target(
    db: &Database,
    encryption_key: &[u8],
    user_id: &str,
    service_id: &str,
) -> AppResult<ProxyTarget> {
    let service = /* fetch service */;

    if !service.is_active {
        return Err(AppError::BadRequest("Service is inactive"));
    }

    // Provider services cannot be proxied to
    if service.service_category == "provider" {
        return Err(AppError::BadRequest("Provider services are not proxyable"));
    }

    // Require an active user connection
    let user_conn = db.collection::<UserServiceConnection>(CONNECTIONS)
        .find_one(doc! {
            "user_id": user_id,
            "service_id": service_id,
            "is_active": true,
        })
        .await?
        .ok_or_else(|| AppError::Forbidden(
            "You must connect to this service before making requests".to_string()
        ))?;

    // Determine which credential to use
    let credential_encrypted = if service.requires_user_credential {
        // Connection services: must have per-user credential
        user_conn.credential_encrypted.ok_or_else(|| {
            AppError::BadRequest("Connection is missing credential. Please reconnect with your API key.".to_string())
        })?
    } else {
        // Internal services: use master credential
        service.credential_encrypted
    };

    let credential = String::from_utf8(
        aes::decrypt(&credential_encrypted, encryption_key)?
    )?;

    Ok(ProxyTarget {
        base_url: service.base_url,
        auth_method: service.auth_method,
        auth_key_name: service.auth_key_name,
        credential,
    })
}
```

**Key behavioral change:** Previously, proxy requests worked without any connection record. Now, an active `UserServiceConnection` is always required. For `connection` services, the per-user credential is used. For `internal` services, the master credential is used but the connection record gates access.

---

## 3. Handler / API Changes

### 3.1 Services Handler Changes

**File:** `backend/src/handlers/services.rs`

#### `POST /api/v1/services` (create)

Update `CreateServiceRequest` to accept `service_category`:

```rust
#[derive(Debug, Deserialize)]
pub struct CreateServiceRequest {
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub base_url: String,
    #[serde(alias = "auth_type")]
    pub auth_method: Option<String>,
    pub auth_key_name: Option<String>,
    pub credential: Option<String>,
    /// "provider", "connection", or "internal". Defaults to "connection".
    pub service_category: Option<String>,
}
```

**Validation logic in handler:**
- If `auth_type` is `"oidc"`, force `service_category = "provider"` (regardless of what the client sent).
- If `service_category` is `"internal"`, set `requires_user_credential = false`.
- If `service_category` is `"connection"` (or absent), set `requires_user_credential = true`.
- Reject `service_category = "provider"` with non-OIDC auth types.

#### `GET /api/v1/services` (list)

Add optional query parameter for filtering:

```
GET /api/v1/services?category=connection
GET /api/v1/services?category=provider
GET /api/v1/services?category=internal
```

If no `category` param is provided, return all active services (for backward compatibility in admin views).

#### `ServiceResponse` - add new fields

```rust
#[derive(Debug, Serialize)]
pub struct ServiceResponse {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub base_url: String,
    pub auth_method: String,
    pub auth_type: Option<String>,
    pub auth_key_name: String,
    pub is_active: bool,
    pub oauth_client_id: Option<String>,
    pub api_spec_url: Option<String>,
    pub service_category: String,           // NEW
    pub requires_user_credential: bool,      // NEW
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}
```

### 3.2 Connections Handler Changes

**File:** `backend/src/handlers/connections.rs`

#### `POST /api/v1/connections/{service_id}` (connect)

Change from path-only to accepting a JSON body with credentials:

```rust
#[derive(Debug, Deserialize)]
pub struct ConnectRequest {
    /// The user's credential for this service.
    /// Required for "connection" category services.
    /// Must be None/absent for "internal" category services.
    pub credential: Option<String>,
    /// Optional label for the credential (e.g., "Production Key")
    pub credential_label: Option<String>,
}

pub async fn connect_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Json(body): Json<ConnectRequest>,
) -> AppResult<Json<ConnectResponse>> {
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    let result = connection_service::connect_user(
        &state.db,
        &encryption_key,
        &auth_user.user_id.to_string(),
        &service_id,
        body.credential.as_deref(),
        body.credential_label.as_deref(),
    ).await?;

    Ok(Json(ConnectResponse {
        service_id,
        service_name: result.service_name,
        connected_at: result.connected_at.to_rfc3339(),
    }))
}
```

**Validation in `connection_service::connect_user`:**
- If `service.service_category == "provider"`: return `400 Bad Request` ("Provider services are not connectable")
- If `service.service_category == "connection"` and `credential.is_none()`: return `400 Bad Request` ("Credential is required for this service type")
- If `service.service_category == "connection"`: encrypt and store the credential
- If `service.service_category == "internal"` and `credential.is_some()`: return `400 Bad Request` ("Internal services do not accept user credentials")
- If `service.service_category == "internal"`: create connection with `credential_encrypted = None`

#### `PUT /api/v1/connections/{service_id}/credential` (update credential - NEW)

New endpoint to update a credential on an existing connection without disconnecting/reconnecting:

```rust
#[derive(Debug, Deserialize)]
pub struct UpdateCredentialRequest {
    pub credential: String,
    pub credential_label: Option<String>,
}

pub async fn update_connection_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Json(body): Json<UpdateCredentialRequest>,
) -> AppResult<Json<UpdateCredentialResponse>> { ... }
```

**Route:** Add to `backend/src/routes.rs`:
```rust
let connection_routes = Router::new()
    .route("/", get(handlers::connections::list_connections))
    .route("/{service_id}", post(handlers::connections::connect_service))
    .route("/{service_id}", delete(handlers::connections::disconnect_service))
    .route("/{service_id}/credential", put(handlers::connections::update_connection_credential));
```

#### `GET /api/v1/connections` (list)

Update `ConnectionItem` to include more information for the frontend:

```rust
#[derive(Debug, Serialize)]
pub struct ConnectionItem {
    pub service_id: String,
    pub service_name: String,
    pub service_category: String,
    pub auth_type: Option<String>,
    pub has_credential: bool,
    pub credential_label: Option<String>,
    pub connected_at: String,
}
```

#### `DELETE /api/v1/connections/{service_id}` (disconnect)

Update to securely clear the credential:

```rust
pub async fn disconnect_service(...) -> AppResult<Json<DisconnectResponse>> {
    // Set credential_encrypted to null AND is_active to false in a single update
    // This ensures credentials are cleaned up on disconnect
    state.db.collection::<UserServiceConnection>(CONNECTIONS)
        .update_one(
            doc! {
                "user_id": &user_id,
                "service_id": &service_id,
                "is_active": true,
            },
            doc! { "$set": {
                "is_active": false,
                "credential_encrypted": bson::Bson::Null,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;
    ...
}
```

### 3.3 MCP Config Handler Changes

**File:** `backend/src/handlers/mcp.rs`

Update `get_mcp_config` to filter out services without valid credentials:

```rust
pub async fn get_mcp_config(...) -> AppResult<Json<McpConfigResponse>> {
    // 1. Get user's active connections
    let connections = /* same as before */;

    // 2. Fetch matching active downstream services
    let services = /* same as before */;

    // 3. Filter: only include services where credentials are satisfied
    let valid_services: Vec<_> = services.into_iter().filter(|svc| {
        let conn = connections.iter().find(|c| c.service_id == svc.id);
        match conn {
            Some(c) => {
                if svc.requires_user_credential {
                    c.credential_encrypted.is_some()
                } else {
                    true // internal services don't need per-user cred
                }
            }
            None => false,
        }
    }).collect();

    // 4. Fetch endpoints only for valid services
    // 5. Build response (same as before, using valid_services)
    ...
}
```

Update the `McpConfigResponse` to include the total count for discovery:

```rust
#[derive(Debug, Serialize)]
pub struct McpConfigResponse {
    pub user_id: String,
    pub proxy_base_url: String,
    pub services: Vec<McpServiceConfig>,
    pub total_services: usize,     // NEW: total connected services
    pub total_endpoints: usize,    // NEW: total tool count
}
```

### 3.4 Proxy Handler Changes

**File:** `backend/src/handlers/proxy.rs`

No structural changes needed. The `proxy_service::resolve_proxy_target` update (Section 2.2) handles the new behavior. The handler already delegates to the service layer.

---

## 4. MCP Proxy Changes

### 4.1 Types Update

**File:** `mcp-proxy/src/types.ts`

```typescript
export interface McpService {
  readonly id: string;
  readonly name: string;
  readonly slug: string;
  readonly description: string | null;
  readonly serviceCategory: string;     // NEW
  readonly endpoints: readonly McpEndpoint[];
}

export interface McpConfig {
  readonly services: readonly McpService[];
  readonly totalServices: number;       // NEW
  readonly totalEndpoints: number;      // NEW
}
```

### 4.2 Tool Discovery / Search

**File:** `mcp-proxy/src/server.ts`

When a user has many connected services (tens or hundreds of tools), MCP clients struggle with large tool lists. Add a built-in `nyxid__search_tools` meta-tool that searches across all available tools:

```typescript
export function createMcpServer(
  mcpConfig: McpConfig,
  accessToken: string,
  nyxidClient: NyxIdClient,
): Server {
  const server = new Server(
    { name: 'nyxid-mcp-proxy', version: '0.2.0' },
    { capabilities: { tools: {} } },
  );

  const tools = generateToolDefinitions(mcpConfig);

  // Add meta-tool for searching when tool count is high
  const TOOL_SEARCH_THRESHOLD = 20;
  const allTools = tools.length > TOOL_SEARCH_THRESHOLD
    ? [createSearchToolDefinition(), ...tools]
    : tools;

  server.setRequestHandler(ListToolsRequestSchema, async () => ({
    tools: allTools.map((t) => ({
      name: t.name,
      description: t.description,
      inputSchema: t.inputSchema,
    })),
  }));

  server.setRequestHandler(CallToolRequestSchema, async (request) => {
    const { name, arguments: args } = request.params;

    // Handle search meta-tool
    if (name === 'nyxid__search_tools') {
      return handleToolSearch(tools, (args ?? {}) as Record<string, unknown>);
    }

    // ... existing tool call resolution ...
  });

  return server;
}
```

**File:** `mcp-proxy/src/tools.ts` - Add search functionality:

```typescript
export function createSearchToolDefinition(): ToolDefinition {
  return {
    name: 'nyxid__search_tools',
    description: 'Search available API tools by name or description. Use this to find the right tool when you have many services connected.',
    inputSchema: {
      type: 'object',
      properties: {
        query: {
          type: 'string',
          description: 'Search query to match against tool names and descriptions',
        },
        service: {
          type: 'string',
          description: 'Optional: filter by service slug',
        },
      },
      required: ['query'],
    },
  };
}

export function handleToolSearch(
  tools: readonly ToolDefinition[],
  args: Record<string, unknown>,
): { content: Array<{ type: 'text'; text: string }>; isError: boolean } {
  const query = String(args.query ?? '').toLowerCase();
  const serviceFilter = args.service ? String(args.service).toLowerCase() : null;

  const matches = tools.filter((t) => {
    if (serviceFilter && !t.name.toLowerCase().startsWith(serviceFilter)) {
      return false;
    }
    return (
      t.name.toLowerCase().includes(query) ||
      t.description.toLowerCase().includes(query)
    );
  });

  if (matches.length === 0) {
    return {
      content: [{ type: 'text', text: `No tools found matching "${query}"` }],
      isError: false,
    };
  }

  const lines = matches.map((t) => `- **${t.name}**: ${t.description}`);
  return {
    content: [{ type: 'text', text: `Found ${matches.length} tools:\n\n${lines.join('\n')}` }],
    isError: false,
  };
}
```

### 4.3 Session Refresh

**File:** `mcp-proxy/src/index.ts`

No structural changes needed for the session management. The MCP config is already fetched per session initialization, so newly connected services appear on the next session.

---

## 5. Frontend Changes

### 5.1 Updated Types

**File:** `frontend/src/types/api.ts`

```typescript
export interface DownstreamService {
  readonly id: string;
  readonly name: string;
  readonly slug: string;
  readonly description: string | null;
  readonly base_url: string;
  readonly auth_method: string;
  readonly auth_type: string | null;
  readonly auth_key_name: string;
  readonly is_active: boolean;
  readonly oauth_client_id: string | null;
  readonly api_spec_url: string | null;
  readonly service_category: string;           // NEW
  readonly requires_user_credential: boolean;   // NEW
  readonly created_by: string;
  readonly created_at: string;
  readonly updated_at: string;
}

export interface UserServiceConnection {
  readonly service_id: string;
  readonly service_name: string;
  readonly service_category: string;    // NEW
  readonly auth_type: string | null;    // NEW
  readonly has_credential: boolean;     // NEW
  readonly credential_label: string | null; // NEW
  readonly connected_at: string;
}
```

### 5.2 Updated Schemas

**File:** `frontend/src/schemas/services.ts`

```typescript
export const SERVICE_CATEGORIES = ["provider", "connection", "internal"] as const;
export type ServiceCategory = (typeof SERVICE_CATEGORIES)[number];

// Auth types that are connectable (non-OIDC)
export const CONNECTABLE_AUTH_TYPES = ["api_key", "oauth2", "basic", "bearer"] as const;

export const createServiceSchema = z.object({
  name: z.string().min(1).max(200),
  description: z.string().max(500).optional(),
  base_url: z.string().min(1).url(),
  auth_type: z.enum(AUTH_TYPES),
  service_category: z.enum(SERVICE_CATEGORIES).optional(), // NEW
});
```

### 5.3 Updated Constants

**File:** `frontend/src/lib/constants.ts`

```typescript
export const SERVICE_CATEGORY_LABELS: Readonly<Record<string, string>> = {
  provider: "SSO Provider",
  connection: "External Service",
  internal: "Internal Service",
};

export function isConnectable(service: DownstreamService): boolean {
  return service.service_category === "connection" || service.service_category === "internal";
}

export function isProvider(service: DownstreamService): boolean {
  return service.service_category === "provider";
}

/// Returns what credential input the user needs to provide for a "connect" flow.
export function getCredentialInputType(service: DownstreamService): {
  type: "api_key" | "bearer" | "basic" | "none";
  label: string;
  placeholder: string;
} {
  if (!service.requires_user_credential) {
    return { type: "none", label: "", placeholder: "" };
  }
  const authType = service.auth_type ?? service.auth_method;
  switch (authType) {
    case "api_key":
      return { type: "api_key", label: "API Key", placeholder: "sk-..." };
    case "bearer":
      return { type: "bearer", label: "Bearer Token", placeholder: "eyJ..." };
    case "basic":
      return { type: "basic", label: "Username:Password", placeholder: "user:pass" };
    case "oauth2":
      return { type: "bearer", label: "Access Token", placeholder: "oauth2 token" };
    default:
      return { type: "api_key", label: "Credential", placeholder: "Enter credential" };
  }
}
```

### 5.4 Updated Hooks

**File:** `frontend/src/hooks/use-services.ts`

Update `useConnectService` to send credential data:

```typescript
interface ConnectServiceParams {
  readonly serviceId: string;
  readonly credential?: string;
  readonly credentialLabel?: string;
}

export function useConnectService() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (params: ConnectServiceParams): Promise<UserServiceConnection> => {
      return api.post<UserServiceConnection>(`/connections/${params.serviceId}`, {
        credential: params.credential,
        credential_label: params.credentialLabel,
      });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["connections"] });
    },
  });
}

export function useUpdateCredential() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (params: {
      readonly serviceId: string;
      readonly credential: string;
      readonly credentialLabel?: string;
    }): Promise<void> => {
      return api.put<void>(`/connections/${params.serviceId}/credential`, {
        credential: params.credential,
        credential_label: params.credentialLabel,
      });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["connections"] });
    },
  });
}
```

### 5.5 Connection Grid Overhaul

**File:** `frontend/src/components/dashboard/connection-grid.tsx`

Major changes:
1. **Filter out provider services** - only show `connection` and `internal` category services
2. **Show credential input dialog** when connecting to `connection` category services
3. **Show simple "Enable" button** for `internal` category services
4. **Show credential status** (has credential, credential label) for connected services
5. **Add "Update Credential" action** for connected `connection` services

**Pseudo-structure:**

```tsx
export function ConnectionGrid() {
  const { data: services } = useServices();
  const { data: connections } = useConnections();
  const connectMutation = useConnectService();
  const [connectDialog, setConnectDialog] = useState<{
    service: DownstreamService;
    credential: string;
    label: string;
  } | null>(null);

  // Filter: only connectable services (exclude providers)
  const connectableServices = services?.filter(isConnectable) ?? [];

  return (
    <>
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {connectableServices.map((service) => {
          const isConnected = connectedIds.has(service.id);
          const connection = connections?.find(c => c.service_id === service.id);

          return (
            <Card key={service.id}>
              {/* Header with service name, category badge */}
              <CardHeader>
                <Badge>{SERVICE_CATEGORY_LABELS[service.service_category]}</Badge>
                <CardTitle>{service.name}</CardTitle>
              </CardHeader>

              <CardContent>
                {isConnected ? (
                  {/* Connected state: show credential status, update/disconnect buttons */}
                ) : (
                  {/* Disconnected state: Connect button */}
                  {/* For connection services: opens credential dialog */}
                  {/* For internal services: direct enable */}
                )}
              </CardContent>
            </Card>
          );
        })}
      </div>

      {/* Credential input dialog for connection services */}
      {connectDialog && (
        <CredentialDialog
          service={connectDialog.service}
          onSubmit={handleConnectWithCredential}
          onCancel={() => setConnectDialog(null)}
        />
      )}
    </>
  );
}
```

### 5.6 New Component: Credential Dialog

**File:** `frontend/src/components/dashboard/credential-dialog.tsx` (new)

A dialog/modal that collects the appropriate credential based on the service's auth type:

```tsx
interface CredentialDialogProps {
  readonly service: DownstreamService;
  readonly onSubmit: (credential: string, label?: string) => void;
  readonly onCancel: () => void;
  readonly isPending: boolean;
}

export function CredentialDialog({ service, onSubmit, onCancel, isPending }: CredentialDialogProps) {
  const inputConfig = getCredentialInputType(service);
  const [credential, setCredential] = useState("");
  const [label, setLabel] = useState("");

  return (
    <Dialog open onOpenChange={onCancel}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Connect to {service.name}</DialogTitle>
          <DialogDescription>
            Enter your {inputConfig.label} to connect to this service.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div>
            <Label>{inputConfig.label}</Label>
            <Input
              type="password"
              placeholder={inputConfig.placeholder}
              value={credential}
              onChange={(e) => setCredential(e.target.value)}
            />
          </div>

          {inputConfig.type === "basic" && (
            <p className="text-xs text-muted-foreground">
              Format: username:password
            </p>
          )}

          <div>
            <Label>Label (optional)</Label>
            <Input
              placeholder="e.g., Production Key"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
            />
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>Cancel</Button>
          <Button
            onClick={() => onSubmit(credential, label || undefined)}
            disabled={!credential || isPending}
          >
            Connect
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
```

### 5.7 Services Admin Page

The admin services page (create/edit forms) needs minor updates:
- Add `service_category` dropdown to the create service form
- Auto-select `"provider"` when auth type is `"oidc"`
- Hide `service_category` selector when editing (category is immutable after creation)
- Show category badge on service list/detail views

---

## 6. Migration Strategy

### 6.1 Data Migration Script

All existing services need the new fields populated. This can be done as a one-time migration script or as part of the application startup.

**Recommended approach:** MongoDB migration script run before deployment.

```javascript
// migration-001-service-categories.js
// Run against the MongoDB database

// 1. OIDC services -> provider category
db.downstream_services.updateMany(
  { $or: [{ auth_method: "oidc" }, { auth_type: "oidc" }] },
  { $set: {
    service_category: "provider",
    requires_user_credential: false
  }}
);

// 2. All other services -> connection category (default)
db.downstream_services.updateMany(
  { service_category: { $exists: false } },
  { $set: {
    service_category: "connection",
    requires_user_credential: true
  }}
);

// 3. Add credential_type and credential_label to existing connections
db.user_service_connections.updateMany(
  { credential_type: { $exists: false } },
  { $set: {
    credential_type: null,
    credential_label: null
  }}
);

// 4. Create new index
db.downstream_services.createIndex(
  { service_category: 1, is_active: 1 }
);
```

### 6.2 Backward Compatibility

- The `#[serde(default)]` attributes on new fields ensure existing documents deserialize correctly without migration.
- The migration script is recommended but not strictly required for the backend to start.
- Existing connections (all with `credential_encrypted: None`) will continue to work for services that get categorized as `internal`, but will show as "missing credential" for `connection` services.
- The frontend should show a "credential required" badge on connections that are missing credentials so users know to re-connect.

### 6.3 Deployment Order

1. Run migration script
2. Deploy backend (new fields handled via `serde(default)`)
3. Deploy MCP proxy (additive type changes)
4. Deploy frontend (uses new API fields)

---

## 7. Security Considerations

### 7.1 Credential Handling

- **Encryption:** Per-user credentials are encrypted with AES-256-GCM using the same `encryption_key` as service-level credentials. The existing `crypto::aes` module is reused.
- **At-rest security:** Credentials are never stored in plaintext. The `credential_encrypted` field is `Vec<u8>` (binary).
- **Credential cleanup:** On disconnect, `credential_encrypted` is explicitly set to `Null` in MongoDB (not just `is_active = false`). This prevents credential leakage from deactivated connections.
- **No credential in responses:** The `ConnectionItem` response includes `has_credential: bool` and `credential_label`, but never the credential itself. There is no API endpoint to retrieve a stored credential.
- **Input validation:** Credentials are validated for length (max 8192 bytes) before encryption to prevent abuse.

### 7.2 Authorization

- **Connect endpoint:** Any authenticated user can connect to `connection` and `internal` services. Provider services reject connections.
- **Proxy endpoint:** Requires active connection. For `connection` services, the per-user credential is used (never the master credential). For `internal` services, the master credential is used.
- **Update credential:** Only the connection owner can update their credential (user_id must match).
- **Service creation:** Remains admin-only. The `service_category` field is set by admins during creation.

### 7.3 SSRF Prevention

- The existing `validate_base_url` function continues to block private/internal addresses for `connection` services.
- For `internal` services: consider relaxing the SSRF check since these are intentionally internal. Add a separate `validate_internal_base_url` that still blocks `169.254.x.x` (cloud metadata) but allows RFC 1918 addresses. This requires the admin to explicitly mark a service as `internal`.

### 7.4 Rate Limiting

- The `POST /api/v1/connections/{service_id}` endpoint should have stricter rate limiting (e.g., 10 requests/minute) since it involves encryption operations.
- The `PUT /api/v1/connections/{service_id}/credential` endpoint should have similar rate limiting.

### 7.5 Audit Logging

Add audit log entries for:
- `connection_created` (with service_id, category, has_credential)
- `connection_credential_updated` (with service_id)
- `connection_removed` (with service_id)
- `proxy_request_denied` (when connection or credential is missing)

---

## 8. Implementation Phases

### Phase 1: Data Model + Migration (Backend)
1. Add `service_category` and `requires_user_credential` fields to `DownstreamService` model
2. Add `credential_type` and `credential_label` fields to `UserServiceConnection` model
3. Write and test the migration script
4. Update `ensure_indexes()` with the new index
5. Update `service_to_response` helper to include new fields
6. Update `ServiceResponse` struct

### Phase 2: Connection Service + Handler (Backend)
1. Create `backend/src/services/connection_service.rs`
2. Update `handlers/connections.rs` - connect with credentials
3. Add `PUT /connections/{service_id}/credential` endpoint
4. Update disconnect to clear credentials
5. Update `ConnectionItem` response struct
6. Update route registration in `routes.rs`

### Phase 3: Proxy + MCP Updates (Backend)
1. Update `proxy_service::resolve_proxy_target` to enforce connections
2. Update `handlers/mcp.rs` to filter services without valid credentials
3. Add `total_services` and `total_endpoints` to MCP config response

### Phase 4: Service Creation (Backend)
1. Update `CreateServiceRequest` to accept `service_category`
2. Add validation logic (OIDC -> provider, etc.)
3. Update `list_services` to support `?category=` filter
4. Update `create_service` handler

### Phase 5: MCP Proxy (TypeScript)
1. Update `types.ts` with new fields
2. Add `createSearchToolDefinition` and `handleToolSearch` to `tools.ts`
3. Update `server.ts` to include search tool when above threshold

### Phase 6: Frontend
1. Update `types/api.ts` with new fields
2. Update `schemas/services.ts` with category support
3. Update `lib/constants.ts` with category helpers
4. Update `hooks/use-services.ts` with credential-aware connect
5. Create `components/dashboard/credential-dialog.tsx`
6. Overhaul `components/dashboard/connection-grid.tsx`
7. Update service creation form with category selector

---

## 9. API Endpoint Summary

| Method | Path | Change | Description |
|--------|------|--------|-------------|
| `GET` | `/api/v1/services` | Modified | Add `?category=` filter param |
| `POST` | `/api/v1/services` | Modified | Accept `service_category` field |
| `GET` | `/api/v1/connections` | Modified | Response includes category, credential status |
| `POST` | `/api/v1/connections/{id}` | Modified | Accept JSON body with credential |
| `PUT` | `/api/v1/connections/{id}/credential` | **New** | Update credential on existing connection |
| `DELETE` | `/api/v1/connections/{id}` | Modified | Clears credential on disconnect |
| `GET` | `/api/v1/mcp/config` | Modified | Filters by credential validity, adds counts |
| `ANY` | `/api/v1/proxy/{id}/{path}` | Modified | Enforces active connection |

---

## 10. Post-Implementation Notes

The following changes were made during code review and security review (after the original architecture was drafted). This section documents deviations from the plan and hardening measures applied.

### 10.1 Reconnection Fix (CR-C1 / SEC-C1)

The original plan used `insert_one` for all new connections. Due to the unique compound index on `(user_id, service_id)`, reconnecting after a disconnect would fail with a duplicate key error because soft-deleted records remain in the collection.

**Resolution:** `connect_user` now checks for an inactive connection record first. If found, it reactivates the existing document via `update_one` and updates the credential fields. Only truly new connections use `insert_one`.

### 10.2 Path Traversal Prevention (SEC-H3)

The proxy URL construction (`forward_request`) now rejects paths containing `..` or `//` to prevent path traversal attacks that could reach unintended endpoints on downstream services.

```rust
if path.contains("..") || path.contains("//") {
    return Err(AppError::BadRequest("Invalid proxy path".to_string()));
}
```

The MCP proxy also URL-encodes path parameter values via `encodeURIComponent()` to prevent injection through user-supplied parameter values.

### 10.3 Custom Debug Implementations (SEC-H2)

All request structs that carry credential data (`ConnectRequest`, `UpdateCredentialRequest`, `CreateServiceRequest`) implement custom `Debug` traits that redact the `credential` field to `[REDACTED]`. This prevents plaintext credentials from appearing in application logs via debug formatting.

### 10.4 Credential Metadata Cleanup on Disconnect (SEC-M5)

`disconnect_user` now clears all credential-related fields, not just `credential_encrypted`:

```rust
doc! { "$set": {
    "is_active": false,
    "credential_encrypted": Bson::Null,
    "credential_type": Bson::Null,
    "credential_label": Bson::Null,
    "updated_at": ...,
}}
```

This prevents `credential_label` (which may contain sensitive identifiers) from persisting after disconnect.

### 10.5 Service Deactivation Cascade (SEC-M2)

When a service is deactivated via `DELETE /api/v1/services/{service_id}`, all active user connections for that service are now also deactivated, and their credentials are wiped. This ensures encrypted credentials do not persist for deactivated services.

### 10.6 Credential Label Validation (CR-H1)

Both `connect_user` and `update_credential` now validate that `credential_label` does not exceed 200 characters.

### 10.7 Tool Search Result Limit (CR-M4)

The MCP proxy `handleToolSearch` function limits results to 25 matches and includes a truncation notice when results are capped.

### 10.8 Generic Error Messages (SEC-M4)

Proxy error messages no longer expose internal details. Detailed errors are logged server-side via `tracing::error!` while the client receives generic messages like "Proxy request failed" or "Failed to decode credential".

---

## 11. Future Considerations

1. **OAuth2 authorization code flow:** The current implementation treats OAuth2 like bearer token storage (user pastes a token). A full authorization code flow (where NyxID redirects the user to the service's OAuth provider) would require callback endpoints, token refresh logic, and a more complex connection flow. Deferred to a future iteration.

2. **Credential rotation / expiry:** Per-user credentials may expire. Consider adding an optional `credential_expires_at` field to `UserServiceConnection` to support proactive refresh reminders.

3. **Internal service SSRF:** The current design keeps strict SSRF checks for all service categories and requires admins to use publicly-resolvable URLs. If private networking is needed, this can be revisited with an allowlist approach.

4. **DNS rebinding protection (SEC-H1):** Base URL validation currently only runs at service creation/update time. A DNS rebinding attack could change the resolved IP after validation. A future improvement would re-validate the resolved IP at proxy time using a custom DNS resolver or reqwest's `resolve` feature. Risk is mitigated by the fact that only admins can create services.

5. **Per-endpoint rate limiting (SEC-M1):** Credential-sensitive endpoints (`POST /connections`, `PUT /connections/.../credential`, OIDC credential endpoints) share the global rate limiter. Adding per-endpoint rate limits (e.g., 5 requests/minute for credential operations) would provide additional protection against abuse.
