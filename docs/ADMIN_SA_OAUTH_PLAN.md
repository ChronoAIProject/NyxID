# Implementation Plan: Admin OAuth/Device-Code Provider Connection for Service Accounts

## Overview

Extend the existing admin SA provider management to support OAuth redirect flows and device-code flows (e.g., OpenAI Codex using ChatGPT subscription). Currently only API key connections are supported for SAs. This plan adds the ability for admins to initiate OAuth and device-code flows on behalf of service accounts, storing the resulting tokens under the SA's ID.

## Requirements

- Admin can initiate an OAuth redirect flow for a service account, with tokens stored under the SA's ID
- Admin can initiate a device-code flow for a service account, with tokens stored under the SA's ID
- OAuth callback correctly identifies the target SA from the encrypted server-side state
- Admin is redirected back to the SA detail page (not the user providers page) after OAuth callback
- All flows require admin authentication and validate the SA exists and is active
- Full audit logging for all admin-on-behalf actions
- Frontend exposes all three provider types (API key, OAuth, device code) in the SA detail page

## Architecture Changes

### Data Model: `OAuthState` (File: `backend/src/models/oauth_state.rs`)

Add two optional fields to support "on-behalf-of" flows:

```rust
/// When an admin initiates a flow on behalf of a service account,
/// this holds the SA ID. Tokens are stored under this ID instead of user_id.
#[serde(default)]
pub target_user_id: Option<String>,

/// Custom frontend redirect path after OAuth callback completes.
/// e.g., "/admin/service-accounts/{sa_id}" for admin flows.
#[serde(default)]
pub redirect_path: Option<String>,
```

**Why these fields are safe:** Both are server-side only (encrypted in MongoDB), never exposed in the OAuth `state` URL parameter. The URL `state` parameter is just the UUID primary key of the OAuthState document. An attacker cannot inject or modify `target_user_id` because it's stored server-side and the `user_id` field (which must match the authenticated session) prevents unauthorized access.

### Flow Diagrams

#### OAuth Redirect Flow (Admin-Initiated)

```
Admin Browser          NyxID Backend              External Provider     Frontend
    |                       |                           |                  |
    |-- GET /admin/sa/{id}/ |                           |                  |
    |   providers/{pid}/    |                           |                  |
    |   connect/oauth ----->|                           |                  |
    |                       |-- Create OAuthState:      |                  |
    |                       |   user_id=admin_id        |                  |
    |                       |   target_user_id=sa_id    |                  |
    |                       |   redirect_path=/admin/.. |                  |
    |<-- 200 {auth_url} ----|                           |                  |
    |                       |                           |                  |
    |-- window.location.href = auth_url --------------->|                  |
    |                       |                           |                  |
    |<------- redirect to callback with ?code&state ----|                  |
    |                       |                           |                  |
    |-- GET /api/v1/providers/callback?code&state ----->|                  |
    |                       |-- Peek OAuthState:        |                  |
    |                       |   Verify user_id matches  |                  |
    |                       |   admin session           |                  |
    |                       |                           |                  |
    |                       |-- handle_oauth_callback:  |                  |
    |                       |   effective_user=          |                  |
    |                       |     target_user_id (sa_id) |                  |
    |                       |   Store token under sa_id |                  |
    |                       |                           |                  |
    |<-- 302 redirect to ---|                           |                  |
    |   /admin/service-     |                           |                  |
    |   accounts/{sa_id}    |                           |                  |
    |   ?provider_status=   |                           |                  |
    |   success             |                           |                  |
    |                       |                           |                  |
    |---------------------------------------------------------->|          |
    |                       |                           | SA detail page   |
    |                       |                           | reads query param|
    |                       |                           | shows toast      |
```

#### Device Code Flow (Admin-Initiated)

```
Admin Browser          NyxID Backend              External Provider
    |                       |                           |
    |-- POST /admin/sa/{id}/|                           |
    |   providers/{pid}/    |                           |
    |   connect/device-code/|                           |
    |   initiate ---------->|                           |
    |                       |-- POST device_code_url -->|
    |                       |<-- {user_code, ...} ------|
    |                       |                           |
    |                       |-- Create OAuthState:      |
    |                       |   user_id=admin_id        |
    |                       |   target_user_id=sa_id    |
    |                       |                           |
    |<-- 200 {user_code,    |                           |
    |    verification_uri,  |                           |
    |    state} ------------|                           |
    |                       |                           |
    |  [Admin opens verification_uri in browser,        |
    |   enters user_code, authenticates]                |
    |                       |                           |
    |-- POST /admin/sa/{id}/|                           |
    |   providers/{pid}/    |                           |
    |   connect/device-code/|                           |
    |   poll -------------->|                           |
    |                       |-- POST device_token_url ->|
    |                       |<-- {status/tokens} -------|
    |                       |                           |
    |                       |-- Store token under sa_id |
    |                       |                           |
    |<-- 200 {status:       |                           |
    |    "complete"} -------|                           |
```

## Implementation Steps

### Phase 1: Model Changes

#### 1.1 Add fields to `OAuthState` (File: `backend/src/models/oauth_state.rs`)
- **Action:** Add `target_user_id: Option<String>` and `redirect_path: Option<String>` fields with `#[serde(default)]`
- **Why:** Enables the callback to know (a) who to store tokens for, and (b) where to redirect the admin
- **Dependencies:** None
- **Risk:** Low -- both fields are optional with defaults, backward-compatible with existing documents
- **Test:** Add BSON roundtrip test with the new fields populated

### Phase 2: Service Layer Changes

#### 2.1 Modify `initiate_oauth_connect` (File: `backend/src/services/user_token_service.rs`)
- **Action:** Add optional parameters `on_behalf_of: Option<&str>` and `redirect_path: Option<&str>`
- **Store:** `target_user_id = on_behalf_of.map(String::from)`, `redirect_path = redirect_path.map(String::from)` in the OAuthState
- **Dependencies:** Phase 1.1
- **Risk:** Low -- existing callers pass `None` for both, behavior unchanged

**Current signature:**
```rust
pub async fn initiate_oauth_connect(
    db: &mongodb::Database,
    encryption_key: &[u8],
    base_url: &str,
    user_id: &str,
    provider_id: &str,
) -> AppResult<String>
```

**New signature:**
```rust
pub async fn initiate_oauth_connect(
    db: &mongodb::Database,
    encryption_key: &[u8],
    base_url: &str,
    user_id: &str,
    provider_id: &str,
    on_behalf_of: Option<&str>,
    redirect_path: Option<&str>,
) -> AppResult<String>
```

#### 2.2 Modify `handle_oauth_callback` (File: `backend/src/services/user_token_service.rs`)
- **Action:** Compute `effective_user_id` from `target_user_id` field:
  ```rust
  let effective_user_id = oauth_state
      .target_user_id
      .as_deref()
      .unwrap_or(&oauth_state.user_id);
  let user_id = effective_user_id;
  ```
- **Dependencies:** Phase 1.1
- **Risk:** Low -- for existing flows, `target_user_id` is `None`, so behavior is identical

#### 2.3 Modify `request_device_code` (File: `backend/src/services/user_token_service.rs`)
- **Action:** Add `on_behalf_of: Option<&str>` parameter. Store `target_user_id` in OAuthState.
- **Dependencies:** Phase 1.1
- **Risk:** Low

**Current signature:**
```rust
pub async fn request_device_code(
    db: &mongodb::Database,
    encryption_key: &[u8],
    user_id: &str,
    provider_id: &str,
) -> AppResult<DeviceCodeInitiateResult>
```

**New signature:**
```rust
pub async fn request_device_code(
    db: &mongodb::Database,
    encryption_key: &[u8],
    user_id: &str,
    provider_id: &str,
    on_behalf_of: Option<&str>,
) -> AppResult<DeviceCodeInitiateResult>
```

#### 2.4 Modify `poll_device_code` (File: `backend/src/services/user_token_service.rs`)
- **Action:** Compute `effective_user_id` from `oauth_state.target_user_id`:
  ```rust
  let effective_user_id = oauth_state
      .target_user_id
      .as_deref()
      .unwrap_or(user_id);
  ```
  Pass `effective_user_id` to `store_device_code_tokens` instead of `user_id`.
- **Dependencies:** Phase 1.1
- **Risk:** Low

### Phase 3: Handler Changes

#### 3.1 Update existing user token handlers (File: `backend/src/handlers/user_tokens.rs`)
- **Action:** Update calls to `initiate_oauth_connect` to pass `None, None` for the two new parameters. Update call to `request_device_code` to pass `None`.
- **Dependencies:** Phase 2.1, 2.3
- **Risk:** Low -- purely mechanical

#### 3.2 Modify `generic_oauth_callback` (File: `backend/src/handlers/user_tokens.rs`)
- **Action:** Two changes:
  1. Read `oauth_state.redirect_path` to determine where to redirect after callback
  2. Include `target_user_id` context in audit logs
  3. If `redirect_path` is present, redirect to `{frontend_url}{redirect_path}?provider_status=success|error[&message=...]` instead of `{frontend_url}/providers/callback?status=...`
- **Dependencies:** Phase 1.1, 2.2
- **Risk:** Medium -- must handle both redirect paths correctly. The existing callback continues to work for normal users.

**Modified redirect logic:**
```rust
// After successful token storage:
if let Some(ref redirect_path) = oauth_state.redirect_path {
    // Admin-on-behalf flow: redirect back to SA detail page
    let mut url = url::Url::parse(&format!("{frontend_url}{redirect_path}"))
        .expect("valid URL");
    url.query_pairs_mut().append_pair("provider_status", "success");
    axum::response::Redirect::to(url.as_str())
} else {
    // Normal user flow: existing behavior
    redirect_callback(frontend_url, "success", None)
}
```

#### 3.3 Add admin OAuth initiate handler (File: `backend/src/handlers/admin_sa_providers.rs`)
- **Action:** Add `initiate_oauth_for_sa` handler
- **Endpoint:** `GET /api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/oauth`
- **Dependencies:** Phase 2.1
- **Risk:** Low

```rust
/// GET /api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/oauth
pub async fn initiate_oauth_for_sa(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path((sa_id, provider_id)): Path<(String, String)>,
) -> AppResult<Json<AdminSaOAuthInitiateResponse>> {
    require_admin(&state, &auth_user).await?;

    let sa = service_account_service::get_service_account(&state.db, &sa_id).await?;
    if !sa.is_active {
        return Err(AppError::BadRequest(
            "Cannot connect providers to an inactive service account".to_string(),
        ));
    }

    let admin_id = auth_user.user_id.to_string();
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;
    let redirect_path = format!("/admin/service-accounts/{}", &sa_id);

    let auth_url = user_token_service::initiate_oauth_connect(
        &state.db,
        &encryption_key,
        &state.config.base_url,
        &admin_id,             // The admin initiating the flow
        &provider_id,
        Some(&sa_id),          // on_behalf_of: the SA
        Some(&redirect_path),  // redirect back to SA detail page
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(admin_id),
        "admin.sa.oauth_initiated".to_string(),
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "provider_id": &provider_id,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(AdminSaOAuthInitiateResponse {
        authorization_url: auth_url,
    }))
}
```

**Response type:**
```rust
#[derive(Debug, Serialize)]
pub struct AdminSaOAuthInitiateResponse {
    pub authorization_url: String,
}
```

#### 3.4 Add admin device code initiate handler (File: `backend/src/handlers/admin_sa_providers.rs`)
- **Action:** Add `initiate_device_code_for_sa` handler
- **Endpoint:** `POST /api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/device-code/initiate`
- **Dependencies:** Phase 2.3
- **Risk:** Low

```rust
/// POST /api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/device-code/initiate
pub async fn initiate_device_code_for_sa(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path((sa_id, provider_id)): Path<(String, String)>,
) -> AppResult<Json<AdminSaDeviceCodeInitiateResponse>> {
    require_admin(&state, &auth_user).await?;

    let sa = service_account_service::get_service_account(&state.db, &sa_id).await?;
    if !sa.is_active {
        return Err(AppError::BadRequest(
            "Cannot connect providers to an inactive service account".to_string(),
        ));
    }

    let admin_id = auth_user.user_id.to_string();
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    let result = user_token_service::request_device_code(
        &state.db,
        &encryption_key,
        &admin_id,        // admin is the initiator
        &provider_id,
        Some(&sa_id),     // on_behalf_of: the SA
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(admin_id),
        "admin.sa.device_code_initiated".to_string(),
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "provider_id": &provider_id,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(AdminSaDeviceCodeInitiateResponse {
        user_code: result.user_code,
        verification_uri: result.verification_uri,
        state: result.state,
        expires_in: result.expires_in,
        interval: result.interval,
    }))
}
```

**Response type:**
```rust
#[derive(Debug, Serialize)]
pub struct AdminSaDeviceCodeInitiateResponse {
    pub user_code: String,
    pub verification_uri: String,
    pub state: String,
    pub expires_in: i64,
    pub interval: i32,
}
```

#### 3.5 Add admin device code poll handler (File: `backend/src/handlers/admin_sa_providers.rs`)
- **Action:** Add `poll_device_code_for_sa` handler
- **Endpoint:** `POST /api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/device-code/poll`
- **Dependencies:** Phase 2.4
- **Risk:** Low

```rust
/// POST /api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/device-code/poll
pub async fn poll_device_code_for_sa(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path((sa_id, provider_id)): Path<(String, String)>,
    Json(body): Json<AdminSaDeviceCodePollRequest>,
) -> AppResult<Json<AdminSaDeviceCodePollResponse>> {
    require_admin(&state, &auth_user).await?;

    // Verify SA exists (no active check needed for polling)
    let _sa = service_account_service::get_service_account(&state.db, &sa_id).await?;

    let admin_id = auth_user.user_id.to_string();
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    let result = user_token_service::poll_device_code(
        &state.db,
        &encryption_key,
        &admin_id,       // admin is the session owner (matches oauth_state.user_id)
        &provider_id,
        &body.state,
    )
    .await?;

    if result.status == "complete" {
        audit_service::log_async(
            state.db.clone(),
            Some(admin_id),
            "admin.sa.provider_connected".to_string(),
            Some(serde_json::json!({
                "target_sa_id": &sa_id,
                "provider_id": &provider_id,
                "token_type": "device_code",
            })),
            extract_ip(&headers),
            extract_user_agent(&headers),
        );
    }

    Ok(Json(AdminSaDeviceCodePollResponse {
        status: result.status,
        interval: result.interval,
    }))
}
```

**Request/Response types:**
```rust
#[derive(Debug, Deserialize)]
pub struct AdminSaDeviceCodePollRequest {
    pub state: String,
}

#[derive(Debug, Serialize)]
pub struct AdminSaDeviceCodePollResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval: Option<i32>,
}
```

### Phase 4: Route Registration

#### 4.1 Register new admin SA provider routes (File: `backend/src/routes.rs`)
- **Action:** Add three new routes to the `sa_admin_routes` block:
  ```rust
  .route("/{sa_id}/providers/{provider_id}/connect/oauth",
      get(handlers::admin_sa_providers::initiate_oauth_for_sa))
  .route("/{sa_id}/providers/{provider_id}/connect/device-code/initiate",
      post(handlers::admin_sa_providers::initiate_device_code_for_sa))
  .route("/{sa_id}/providers/{provider_id}/connect/device-code/poll",
      post(handlers::admin_sa_providers::poll_device_code_for_sa))
  ```
- **Dependencies:** Phase 3.3, 3.4, 3.5
- **Risk:** Low

### Phase 5: Frontend Changes

#### 5.1 Add TypeScript types (File: `frontend/src/types/service-accounts.ts`)
- **Action:** Add response types for the new admin SA endpoints:
  ```typescript
  export interface SaOAuthInitiateResponse {
    readonly authorization_url: string;
  }

  export interface SaDeviceCodeInitiateResponse {
    readonly user_code: string;
    readonly verification_uri: string;
    readonly state: string;
    readonly expires_in: number;
    readonly interval: number;
  }

  export interface SaDeviceCodePollRequest {
    readonly state: string;
  }

  export interface SaDeviceCodePollResponse {
    readonly status: string;
    readonly interval?: number;
  }
  ```
- **Dependencies:** None
- **Risk:** Low

#### 5.2 Add hooks (File: `frontend/src/hooks/use-service-accounts.ts`)
- **Action:** Add three new hooks:

```typescript
export function useInitiateOAuthForSa() {
  return useMutation({
    mutationFn: async ({
      saId,
      providerId,
    }: {
      readonly saId: string;
      readonly providerId: string;
    }): Promise<SaOAuthInitiateResponse> => {
      return api.get<SaOAuthInitiateResponse>(
        `/admin/service-accounts/${saId}/providers/${providerId}/connect/oauth`,
      );
    },
  });
}

export function useInitiateDeviceCodeForSa() {
  return useMutation({
    mutationFn: async ({
      saId,
      providerId,
    }: {
      readonly saId: string;
      readonly providerId: string;
    }): Promise<SaDeviceCodeInitiateResponse> => {
      return api.post<SaDeviceCodeInitiateResponse>(
        `/admin/service-accounts/${saId}/providers/${providerId}/connect/device-code/initiate`,
      );
    },
  });
}

export function usePollDeviceCodeForSa() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      saId,
      providerId,
      state,
    }: {
      readonly saId: string;
      readonly providerId: string;
      readonly state: string;
    }): Promise<SaDeviceCodePollResponse> => {
      return api.post<SaDeviceCodePollResponse>(
        `/admin/service-accounts/${saId}/providers/${providerId}/connect/device-code/poll`,
        { state } satisfies SaDeviceCodePollRequest,
      );
    },
    onSuccess: (data, { saId }) => {
      if (data.status === "complete") {
        void queryClient.invalidateQueries({
          queryKey: ["admin", "service-accounts", saId, "providers"],
        });
      }
    },
  });
}
```

- **Dependencies:** Phase 5.1
- **Risk:** Low

#### 5.3 Expand Connect Provider dropdown (File: `frontend/src/pages/admin-service-account-detail.tsx`)
- **Action:** Modify the dropdown to show all provider types, not just `api_key`:
  1. Change `availableApiKeyProviders` filter to include `oauth2` and `device_code` providers
  2. Rename variable to `availableProviders`
  3. Add icons to distinguish provider types in the dropdown (KeyRound for API key, Globe for OAuth, Smartphone for device code)
  4. When selected:
     - `api_key` → open `ApiKeyDialog` (existing behavior)
     - `device_code` → open new `SaDeviceCodeDialog`
     - `oauth2` → call `useInitiateOAuthForSa` → `window.location.href = authorization_url`
- **Dependencies:** Phase 5.2
- **Risk:** Medium -- involves UI flow changes

**Updated filter:**
```typescript
const availableProviders = (allProviders ?? []).filter(
  (p) => p.is_active && !connectedProviderIds.has(p.id),
);
```

**Updated connect handler:**
```typescript
function handleConnect(provider: ProviderConfig) {
  if (provider.provider_type === "api_key") {
    setConnectDialogProvider(provider);
  } else if (provider.provider_type === "device_code") {
    setDeviceCodeDialogProvider(provider);
  } else {
    void handleOAuthConnect(provider);
  }
}

async function handleOAuthConnect(provider: ProviderConfig) {
  try {
    const response = await initiateOAuthForSaMutation.mutateAsync({
      saId,
      providerId: provider.id,
    });
    window.location.href = response.authorization_url;
  } catch (err) {
    if (err instanceof ApiError) toast.error(err.message);
    else toast.error("Failed to initiate OAuth connection");
  }
}
```

#### 5.4 Add SA Device Code Dialog (File: `frontend/src/components/dashboard/sa-device-code-dialog.tsx`)
- **Action:** Create a variant of `DeviceCodeDialog` that uses the admin SA hooks instead of user hooks
- **Why:** The SA device code dialog calls different API endpoints (admin SA routes) and uses different hooks
- **Approach:** Create a new component `SaDeviceCodeDialog` that accepts `saId` and `provider` props, and uses `useInitiateDeviceCodeForSa` / `usePollDeviceCodeForSa` hooks
- **Dependencies:** Phase 5.2
- **Risk:** Low -- follows exact same pattern as existing `DeviceCodeDialog`

**Props:**
```typescript
interface SaDeviceCodeDialogProps {
  readonly saId: string;
  readonly provider: ProviderConfig;
  readonly onClose: () => void;
}
```

The component reuses the same UI (user code display, verification URI button, polling spinner, countdown timer) but calls the admin SA endpoints. The flow step state machine is identical: `requesting` -> `show_code` -> `success`/`error`.

#### 5.5 Handle OAuth callback redirect on SA detail page (File: `frontend/src/pages/admin-service-account-detail.tsx`)
- **Action:** Read `provider_status` and `message` from URL search params. Show a toast on mount if present. Clean up the URL.
- **Dependencies:** Phase 3.2
- **Risk:** Low

```typescript
const search = useSearch({ strict: false }) as {
  readonly provider_status?: string;
  readonly message?: string;
};

useEffect(() => {
  if (search.provider_status === "success") {
    toast.success("Provider connected successfully");
    // Clean URL params
    void navigate({ to: ".", search: {}, replace: true });
  } else if (search.provider_status === "error") {
    toast.error(search.message ?? "Failed to connect provider");
    void navigate({ to: ".", search: {}, replace: true });
  }
}, [search.provider_status]);
```

#### 5.6 Add provider type badges to dropdown (File: `frontend/src/pages/admin-service-account-detail.tsx`)
- **Action:** Update `ConnectProviderDropdown` to show provider type icons/badges to help admins distinguish between API key, OAuth, and device code providers
- **Dependencies:** None
- **Risk:** Low

```typescript
function ConnectProviderDropdown({ providers, onSelect }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm">
          <Plug className="mr-1 h-3 w-3" />
          Connect Provider
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent>
        {providers.map((p) => (
          <DropdownMenuItem key={p.id} onClick={() => onSelect(p)}>
            {p.provider_type === "api_key" && <KeyRound className="mr-2 h-4 w-4" />}
            {p.provider_type === "oauth2" && <Globe className="mr-2 h-4 w-4" />}
            {p.provider_type === "device_code" && <Smartphone className="mr-2 h-4 w-4" />}
            <span>{p.name}</span>
            <Badge variant="outline" className="ml-auto text-xs">
              {p.provider_type === "api_key" ? "API Key" :
               p.provider_type === "oauth2" ? "OAuth" : "Device Code"}
            </Badge>
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
```

## API Endpoints Summary

### New Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/oauth` | Admin initiates OAuth redirect for SA |
| POST | `/api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/device-code/initiate` | Admin initiates device code for SA |
| POST | `/api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/device-code/poll` | Admin polls device code status for SA |

### Modified Endpoints

| Method | Path | Change |
|--------|------|--------|
| GET | `/api/v1/providers/callback` | Supports `redirect_path` from OAuthState for admin flows |

### Existing Endpoints (Unchanged)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/admin/service-accounts/{sa_id}/providers` | List SA's connected providers |
| POST | `/api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/api-key` | Connect API key for SA |
| DELETE | `/api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/disconnect` | Disconnect provider from SA |

## Security Considerations

### 1. OAuth State Integrity (Critical)
The `target_user_id` is stored server-side in MongoDB, not passed through the URL. The URL `state` parameter is only the OAuthState document's UUID `_id`. An attacker cannot inject or modify `target_user_id` because:
- They would need to create a valid OAuthState document (requires DB access)
- The `user_id` field must match the authenticated admin session in the callback

### 2. Admin Authorization
All new endpoints call `require_admin()` before any action. The `require_admin` check reads the user record from MongoDB and verifies `is_admin == true`.

### 3. Service Account Validation
Every flow validates:
- SA exists via `service_account_service::get_service_account`
- SA is active (`is_active == true`) before initiating new connections
- Poll endpoints verify SA exists (may have been deactivated mid-flow; tokens can still complete)

### 4. Session Hijack Prevention
The OAuth callback verifies `oauth_state.user_id == auth_user.user_id`. Even for admin-on-behalf flows, the admin who initiated must be the one receiving the callback. A different admin or user cannot complete another admin's flow.

### 5. CSRF Protection
OAuth CSRF protection is maintained through the `state` parameter (random UUID). The state must match a valid OAuthState document in MongoDB.

### 6. Token Storage Isolation
Tokens are stored under the SA's ID (`target_user_id`), completely isolated from the admin's own tokens. The admin cannot accidentally access or modify their own provider tokens through the admin SA flow.

### 7. Audit Trail
All admin-on-behalf actions are logged with:
- `admin_id` as the actor
- `target_sa_id` as the target
- `provider_id` for the specific provider
- IP address and user agent

### 8. Rate Limiting
All admin endpoints inherit the global rate limiter. Consider adding per-endpoint limits (TODO: SEC-9) for OAuth initiate endpoints.

## Testing Strategy

### Unit Tests
- `OAuthState` BSON roundtrip with `target_user_id` and `redirect_path`
- Verify `effective_user_id` computation in service functions

### Integration Tests
- Admin initiates OAuth for SA → OAuthState has correct `target_user_id`
- OAuth callback with `target_user_id` stores token under SA ID
- OAuth callback redirects to `redirect_path` when present
- Device code initiate for SA → OAuthState has correct `target_user_id`
- Device code poll completes → token stored under SA ID
- Non-admin user cannot access admin SA provider endpoints (403)
- Inactive SA → admin cannot initiate OAuth/device-code (400)

### E2E Tests (if applicable)
- Admin connects OAuth provider for SA → redirected back to SA detail page with success toast
- Admin connects device code provider for SA → dialog shows code → polls → shows success

## Risks & Mitigations

### Risk: OAuth callback redirect to wrong page
- **Mitigation:** `redirect_path` is stored server-side, cannot be tampered with. Default behavior (normal user flow) is unchanged when `redirect_path` is `None`.

### Risk: Orphaned OAuthState documents if admin abandons flow
- **Mitigation:** OAuthState has `expires_at` (10 min for OAuth, variable for device code). Expired states are cleaned up by existing TTL logic or ignored on next access.

### Risk: Race condition between admin flows and user flows for same provider
- **Mitigation:** `user_token_service::handle_oauth_callback` does `find_one_and_delete` atomically. If two callbacks race, only one succeeds.

### Risk: Breaking existing user flows
- **Mitigation:** All service function changes are additive (new optional parameters defaulting to `None`). Existing handlers pass `None`. The `effective_user_id` fallback is the original `user_id`, preserving existing behavior exactly.

## Files Modified (Summary)

### Backend
| File | Change |
|------|--------|
| `models/oauth_state.rs` | Add `target_user_id`, `redirect_path` fields |
| `services/user_token_service.rs` | Add `on_behalf_of`/`redirect_path` params to `initiate_oauth_connect`, `request_device_code`; compute `effective_user_id` in `handle_oauth_callback`, `poll_device_code` |
| `handlers/user_tokens.rs` | Update `initiate_oauth_connect` and `request_device_code` calls to pass `None`; update `generic_oauth_callback` redirect logic |
| `handlers/admin_sa_providers.rs` | Add 3 new handlers + request/response types |
| `routes.rs` | Register 3 new routes under `sa_admin_routes` |

### Frontend
| File | Change |
|------|--------|
| `types/service-accounts.ts` | Add 4 new type interfaces |
| `hooks/use-service-accounts.ts` | Add 3 new hooks |
| `pages/admin-service-account-detail.tsx` | Expand provider dropdown, add OAuth/device-code connect handlers, read `provider_status` query param |
| `components/dashboard/sa-device-code-dialog.tsx` | New component (variant of `device-code-dialog.tsx` for admin SA flows) |

## Success Criteria

- [ ] Admin can connect an OAuth provider (e.g., Google) to a service account via the SA detail page
- [ ] After OAuth callback, admin is redirected back to the SA detail page with a success toast
- [ ] Tokens are stored under the SA's ID, not the admin's ID
- [ ] Admin can initiate a device code flow (e.g., OpenAI Codex) for a service account
- [ ] Device code dialog shows user_code and polls until complete
- [ ] After device code completion, tokens are stored under the SA's ID
- [ ] Non-admin users receive 403 for all new endpoints
- [ ] Inactive SAs cannot have new providers connected (400)
- [ ] All admin-on-behalf actions are logged in the audit log
- [ ] Existing user OAuth and device code flows continue to work unchanged
- [ ] `cargo test` passes with new model tests
- [ ] `npm run build` passes in frontend
