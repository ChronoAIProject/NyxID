# Implementation Plan: Admin Provider Management for Service Accounts

## Overview

Allow admins to connect, list, and disconnect providers on behalf of service accounts from the admin panel. The existing `user_token_service` functions already accept `user_id: &str`, so we can pass the service account's ID directly -- no new service layer is needed. Three connection methods are supported: API key (direct), OAuth redirect (admin-initiated browser flow), and device code (admin-initiated device authorization grant). See `docs/ADMIN_SA_OAUTH_PLAN.md` for the OAuth and device-code extension plan.

## Requirements

- Admin can list all provider tokens connected to a service account
- Admin can connect an API key provider to a service account
- Admin can disconnect a provider from a service account
- All actions require `require_admin` authentication
- All actions produce audit log entries with `admin.sa.provider.*` event types
- OAuth redirect connections supported (admin-initiated browser flow)
- Device code connections supported (admin-initiated device authorization grant)

## Architecture: Backend

### Key Insight -- No New Service Layer Needed

The `user_token_service` functions take `user_id: &str` (not `AuthUser`). Service accounts have UUID string IDs stored in `user_provider_tokens.user_id`. We reuse:

- `user_token_service::store_api_key(db, encryption_key, user_id, provider_id, api_key, label)`
- `user_token_service::list_user_tokens(db, user_id)`
- `user_token_service::disconnect_provider(db, user_id, provider_id)`

Passing `sa.id` as the `user_id` parameter.

### New File: `backend/src/handlers/admin_sa_providers.rs`

```rust
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::handlers::admin_helpers::{extract_ip, extract_user_agent, require_admin};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, service_account_service, user_token_service};
use crate::AppState;
```

#### Request/Response Types

```rust
#[derive(Debug, Deserialize)]
pub struct AdminConnectApiKeyRequest {
    pub provider_id: String,
    pub api_key: String,
    pub label: Option<String>,
}

// Manual Debug to redact api_key
impl std::fmt::Debug for AdminConnectApiKeyRequest { ... } // redact api_key

#[derive(Debug, Serialize)]
pub struct AdminSaProviderTokenResponse {
    pub provider_id: String,
    pub provider_name: String,
    pub provider_slug: String,
    pub provider_type: String,
    pub status: String,
    pub label: Option<String>,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub connected_at: String,
}

#[derive(Debug, Serialize)]
pub struct AdminSaProviderListResponse {
    pub tokens: Vec<AdminSaProviderTokenResponse>,
}

#[derive(Debug, Serialize)]
pub struct AdminSaProviderActionResponse {
    pub status: String,
    pub message: String,
}
```

#### Handler Functions

**1. List SA Provider Tokens**

```rust
/// GET /api/v1/admin/service-accounts/{sa_id}/providers
pub async fn list_sa_providers(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(sa_id): Path<String>,
) -> AppResult<Json<AdminSaProviderListResponse>> {
    require_admin(&state, &auth_user).await?;

    // Verify SA exists
    let _sa = service_account_service::get_service_account(&state.db, &sa_id).await?;

    let summaries = user_token_service::list_user_tokens(&state.db, &sa_id).await?;

    let tokens: Vec<AdminSaProviderTokenResponse> = summaries
        .into_iter()
        .map(|s| AdminSaProviderTokenResponse {
            provider_id: s.provider_config_id,
            provider_name: s.provider_name,
            provider_slug: s.provider_slug,
            provider_type: s.token_type,
            status: s.status,
            label: s.label,
            expires_at: s.expires_at,
            last_used_at: s.last_used_at,
            connected_at: s.connected_at,
        })
        .collect();

    Ok(Json(AdminSaProviderListResponse { tokens }))
}
```

**2. Connect API Key on Behalf of SA**

```rust
/// POST /api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/api-key
pub async fn connect_api_key_for_sa(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path((sa_id, provider_id)): Path<(String, String)>,
    Json(body): Json<AdminConnectApiKeyRequest>,
) -> AppResult<Json<AdminSaProviderActionResponse>> {
    require_admin(&state, &auth_user).await?;

    // Verify SA exists and is active
    let sa = service_account_service::get_service_account(&state.db, &sa_id).await?;
    if !sa.is_active {
        return Err(AppError::BadRequest(
            "Cannot connect providers to an inactive service account".to_string(),
        ));
    }

    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    if body.api_key.is_empty() {
        return Err(AppError::ValidationError(
            "API key must not be empty".to_string(),
        ));
    }
    if body.api_key.len() > 4096 {
        return Err(AppError::ValidationError(
            "API key exceeds maximum length".to_string(),
        ));
    }

    // Reuse existing service -- pass sa.id as user_id
    user_token_service::store_api_key(
        &state.db,
        &encryption_key,
        &sa_id,
        &provider_id,
        &body.api_key,
        body.label.as_deref(),
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.sa.provider_connected".to_string(),
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "provider_id": &provider_id,
            "token_type": "api_key",
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(AdminSaProviderActionResponse {
        status: "connected".to_string(),
        message: "API key stored for service account".to_string(),
    }))
}
```

**3. Disconnect Provider from SA**

```rust
/// DELETE /api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/disconnect
pub async fn disconnect_sa_provider(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path((sa_id, provider_id)): Path<(String, String)>,
) -> AppResult<Json<AdminSaProviderActionResponse>> {
    require_admin(&state, &auth_user).await?;

    // Verify SA exists
    let _sa = service_account_service::get_service_account(&state.db, &sa_id).await?;

    user_token_service::disconnect_provider(&state.db, &sa_id, &provider_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.sa.provider_disconnected".to_string(),
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "provider_id": &provider_id,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(AdminSaProviderActionResponse {
        status: "disconnected".to_string(),
        message: "Provider disconnected from service account".to_string(),
    }))
}
```

### Changes to `backend/src/handlers/mod.rs`

Add one line:

```rust
pub mod admin_sa_providers;
```

### Changes to `backend/src/routes.rs`

Add new routes nested under the existing `sa_admin_routes`:

```rust
let sa_admin_routes = Router::new()
    .route("/", get(handlers::admin_service_accounts::list_service_accounts)
        .post(handlers::admin_service_accounts::create_service_account))
    .route("/{sa_id}", get(handlers::admin_service_accounts::get_service_account)
        .put(handlers::admin_service_accounts::update_service_account)
        .delete(handlers::admin_service_accounts::delete_service_account))
    .route("/{sa_id}/rotate-secret",
        post(handlers::admin_service_accounts::rotate_secret))
    .route("/{sa_id}/revoke-tokens",
        post(handlers::admin_service_accounts::revoke_tokens))
    // NEW: Provider management for SAs
    .route("/{sa_id}/providers",
        get(handlers::admin_sa_providers::list_sa_providers))
    .route("/{sa_id}/providers/{provider_id}/connect/api-key",
        post(handlers::admin_sa_providers::connect_api_key_for_sa))
    .route("/{sa_id}/providers/{provider_id}/disconnect",
        delete(handlers::admin_sa_providers::disconnect_sa_provider));
```

## Architecture: Frontend

### New API Endpoints (Summary)

| Method | Path | Description |
|--------|------|-------------|
| `GET`  | `/api/v1/admin/service-accounts/{sa_id}/providers` | List SA provider tokens |
| `POST` | `/api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/api-key` | Connect API key for SA |
| `DELETE` | `/api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/disconnect` | Disconnect provider from SA |

### New Types in `frontend/src/types/service-accounts.ts`

```typescript
export interface SaProviderToken {
  readonly provider_id: string;
  readonly provider_name: string;
  readonly provider_slug: string;
  readonly provider_type: string;
  readonly status: string;
  readonly label: string | null;
  readonly expires_at: string | null;
  readonly last_used_at: string | null;
  readonly connected_at: string;
}

export interface SaProviderListResponse {
  readonly tokens: readonly SaProviderToken[];
}

export interface SaProviderActionResponse {
  readonly status: string;
  readonly message: string;
}
```

### New Hooks in `frontend/src/hooks/use-service-accounts.ts`

```typescript
export function useSaProviders(saId: string) {
  return useQuery({
    queryKey: ["admin", "service-accounts", saId, "providers"],
    queryFn: async (): Promise<readonly SaProviderToken[]> => {
      const res = await api.get<SaProviderListResponse>(
        `/admin/service-accounts/${saId}/providers`,
      );
      return res.tokens;
    },
    enabled: saId.length > 0,
  });
}

export function useConnectApiKeyForSa() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      saId,
      providerId,
      apiKey,
      label,
    }: {
      readonly saId: string;
      readonly providerId: string;
      readonly apiKey: string;
      readonly label?: string;
    }): Promise<SaProviderActionResponse> => {
      return api.post<SaProviderActionResponse>(
        `/admin/service-accounts/${saId}/providers/${providerId}/connect/api-key`,
        { api_key: apiKey, label },
      );
    },
    onSuccess: (_, { saId }) => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "service-accounts", saId, "providers"],
      });
    },
  });
}

export function useDisconnectSaProvider() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      saId,
      providerId,
    }: {
      readonly saId: string;
      readonly providerId: string;
    }): Promise<SaProviderActionResponse> => {
      return api.delete<SaProviderActionResponse>(
        `/admin/service-accounts/${saId}/providers/${providerId}/disconnect`,
      );
    },
    onSuccess: (_, { saId }) => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "service-accounts", saId, "providers"],
      });
    },
  });
}
```

### Frontend Component Changes

#### Modified: `frontend/src/pages/admin-service-account-detail.tsx`

Add a new "Connected Providers" section between the existing "Service Account Information" and "Actions" sections.

**New imports:**

```typescript
import {
  useSaProviders,
  useConnectApiKeyForSa,
  useDisconnectSaProvider,
} from "@/hooks/use-service-accounts";
import { useProviders } from "@/hooks/use-providers";
import type { ProviderConfig } from "@/types/api";
import type { SaProviderToken } from "@/types/service-accounts";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Plug, Unlink, KeyRound } from "lucide-react";
```

**New state and queries in the component:**

```typescript
const { data: saProviders, isLoading: providersLoading } = useSaProviders(saId);
const { data: allProviders } = useProviders();
const connectApiKeyMutation = useConnectApiKeyForSa();
const disconnectSaProviderMutation = useDisconnectSaProvider();

const [connectDialogProvider, setConnectDialogProvider] = useState<ProviderConfig | null>(null);
```

**New JSX section (between "Service Account Information" and "Actions"):**

```tsx
<Separator />

<DetailSection title="Connected Providers">
  {providersLoading ? (
    <Skeleton className="h-24 w-full" />
  ) : saProviders && saProviders.length > 0 ? (
    <div className="rounded-md border">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Provider</TableHead>
            <TableHead>Type</TableHead>
            <TableHead>Status</TableHead>
            <TableHead>Label</TableHead>
            <TableHead>Connected</TableHead>
            <TableHead />
          </TableRow>
        </TableHeader>
        <TableBody>
          {saProviders.map((token) => (
            <TableRow key={token.provider_id}>
              <TableCell className="font-medium">{token.provider_name}</TableCell>
              <TableCell>
                <Badge variant="outline">
                  {token.provider_type === "api_key" ? "API Key" : "OAuth"}
                </Badge>
              </TableCell>
              <TableCell>
                <Badge variant={token.status === "active" ? "success" : "secondary"}>
                  {token.status}
                </Badge>
              </TableCell>
              <TableCell className="text-muted-foreground">
                {token.label ?? "-"}
              </TableCell>
              <TableCell className="text-muted-foreground">
                {formatDate(token.connected_at)}
              </TableCell>
              <TableCell>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => void handleDisconnectSaProvider(token.provider_id)}
                  disabled={disconnectSaProviderMutation.isPending}
                >
                  <Unlink className="mr-1 h-3 w-3" />
                  Disconnect
                </Button>
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  ) : (
    <p className="text-sm text-muted-foreground">
      No providers connected to this service account.
    </p>
  )}

  {/* Connect button - only show API key providers */}
  <div className="mt-3">
    <ConnectProviderDropdown
      providers={availableApiKeyProviders}
      onSelect={(provider) => setConnectDialogProvider(provider)}
    />
  </div>
</DetailSection>
```

**Helper: Available API key providers**

Filter `allProviders` to only show active API key providers that the SA is not already connected to:

```typescript
const connectedProviderIds = new Set(saProviders?.map((t) => t.provider_id) ?? []);
const availableApiKeyProviders = (allProviders ?? []).filter(
  (p) => p.is_active && p.provider_type === "api_key" && !connectedProviderIds.has(p.id),
);
```

**Connect Provider Dropdown Component (inline in same file):**

```tsx
function ConnectProviderDropdown({
  providers,
  onSelect,
}: {
  readonly providers: readonly ProviderConfig[];
  readonly onSelect: (provider: ProviderConfig) => void;
}) {
  if (providers.length === 0) {
    return null;
  }

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
            <KeyRound className="mr-2 h-4 w-4" />
            {p.name}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
```

**API Key Connect Dialog (reuse pattern from existing `api-key-dialog.tsx`):**

```tsx
{connectDialogProvider !== null && (
  <ApiKeyConnectDialog
    provider={connectDialogProvider}
    saId={saId}
    onSuccess={() => {
      setConnectDialogProvider(null);
      toast.success(`Connected ${connectDialogProvider.name}`);
    }}
    onCancel={() => setConnectDialogProvider(null)}
    isPending={connectApiKeyMutation.isPending}
    onSubmit={(apiKey, label) => {
      void connectApiKeyMutation
        .mutateAsync({
          saId,
          providerId: connectDialogProvider.id,
          apiKey,
          label,
        })
        .then(() => {
          setConnectDialogProvider(null);
          toast.success(`Connected ${connectDialogProvider.name}`);
        })
        .catch((err) => {
          if (err instanceof ApiError) {
            toast.error(err.message);
          } else {
            toast.error("Failed to connect provider");
          }
        });
    }}
  />
)}
```

The `ApiKeyConnectDialog` can either:
1. Reuse the existing `ApiKeyDialog` component directly (it takes `provider`, `onSubmit`, `onCancel`, `isPending`)
2. Or inline a similar dialog

**Recommended:** Reuse the existing `ApiKeyDialog` from `components/dashboard/api-key-dialog.tsx` directly since its props already match what we need:

```tsx
{connectDialogProvider !== null && (
  <ApiKeyDialog
    provider={connectDialogProvider}
    onSubmit={(apiKey, label) => void handleConnectApiKey(apiKey, label)}
    onCancel={() => setConnectDialogProvider(null)}
    isPending={connectApiKeyMutation.isPending}
  />
)}
```

**Handler functions to add:**

```typescript
async function handleConnectApiKey(apiKey: string, label?: string) {
  if (!connectDialogProvider) return;
  try {
    await connectApiKeyMutation.mutateAsync({
      saId,
      providerId: connectDialogProvider.id,
      apiKey,
      label,
    });
    toast.success(`Connected ${connectDialogProvider.name}`);
    setConnectDialogProvider(null);
  } catch (err) {
    if (err instanceof ApiError) {
      toast.error(err.message);
    } else {
      toast.error("Failed to connect provider");
    }
  }
}

async function handleDisconnectSaProvider(providerId: string) {
  try {
    await disconnectSaProviderMutation.mutateAsync({ saId, providerId });
    toast.success("Provider disconnected");
  } catch (err) {
    if (err instanceof ApiError) {
      toast.error(err.message);
    } else {
      toast.error("Failed to disconnect provider");
    }
  }
}
```

**New imports needed:**

```typescript
import { ApiKeyDialog } from "@/components/dashboard/api-key-dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
```

## Implementation Steps

### Phase 1: Backend (Task #2)

1. **Create `backend/src/handlers/admin_sa_providers.rs`** (~120 lines)
   - Request/response structs
   - `list_sa_providers` handler
   - `connect_api_key_for_sa` handler
   - `disconnect_sa_provider` handler
   - Dependencies: None
   - Risk: Low (reuses existing service layer)

2. **Register module in `backend/src/handlers/mod.rs`**
   - Add `pub mod admin_sa_providers;`
   - Dependencies: Step 1
   - Risk: Low

3. **Add routes in `backend/src/routes.rs`**
   - Add 3 new routes to `sa_admin_routes`
   - Dependencies: Step 2
   - Risk: Low

4. **Verify compilation**
   - `cargo build`
   - Dependencies: Step 3
   - Risk: Low

### Phase 2: Frontend (Task #3)

5. **Add types to `frontend/src/types/service-accounts.ts`**
   - `SaProviderToken`, `SaProviderListResponse`, `SaProviderActionResponse`
   - Dependencies: None
   - Risk: Low

6. **Add hooks to `frontend/src/hooks/use-service-accounts.ts`**
   - `useSaProviders`, `useConnectApiKeyForSa`, `useDisconnectSaProvider`
   - Dependencies: Step 5
   - Risk: Low

7. **Modify `frontend/src/pages/admin-service-account-detail.tsx`**
   - Add "Connected Providers" section with table
   - Add connect dropdown (API key providers only)
   - Add disconnect button per row
   - Reuse `ApiKeyDialog` component
   - Dependencies: Steps 5, 6
   - Risk: Medium (largest change)

8. **Verify frontend build**
   - `npm run build` from frontend/
   - Dependencies: Step 7
   - Risk: Low

## API Endpoint Specifications

### GET `/api/v1/admin/service-accounts/{sa_id}/providers`

**Auth:** Admin required (session cookie)
**Path params:** `sa_id` (UUID string)
**Response 200:**
```json
{
  "tokens": [
    {
      "provider_id": "uuid-string",
      "provider_name": "OpenAI",
      "provider_slug": "openai",
      "provider_type": "api_key",
      "status": "active",
      "label": "Production",
      "expires_at": null,
      "last_used_at": "2024-01-15T10:30:00Z",
      "connected_at": "2024-01-01T00:00:00Z"
    }
  ]
}
```
**Error 404:** SA not found

### POST `/api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/api-key`

**Auth:** Admin required (session cookie)
**Path params:** `sa_id` (UUID string), `provider_id` (UUID string)
**Request body:**
```json
{
  "api_key": "sk-...",
  "label": "Production Key"
}
```
**Validation:**
- `api_key`: non-empty, max 4096 chars
- `label`: optional, max 200 chars

**Response 200:**
```json
{
  "status": "connected",
  "message": "API key stored for service account"
}
```
**Errors:**
- 400: SA inactive, empty API key, key too long, wrong provider type
- 404: SA or provider not found

### DELETE `/api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/disconnect`

**Auth:** Admin required (session cookie)
**Path params:** `sa_id` (UUID string), `provider_id` (UUID string)
**Response 200:**
```json
{
  "status": "disconnected",
  "message": "Provider disconnected from service account"
}
```
**Error 404:** SA not found or no active token for provider

## Security Considerations

1. **Admin-only access**: All endpoints use `require_admin()` which checks the `is_admin` flag on the user record
2. **SA validation**: Every request verifies the SA exists via `service_account_service::get_service_account()`
3. **Inactive SA guard**: Connect endpoint rejects inactive SAs
4. **API key encryption**: Reuses existing AES-256 encryption via `user_token_service::store_api_key`
5. **Audit logging**: All actions logged with admin user ID, target SA ID, and provider ID
6. **No secret leakage**: Response structs never include decrypted keys
7. **Route placement**: Routes are under `admin_routes` which has `reject_service_account_tokens` middleware, preventing SAs from managing their own providers through admin endpoints

## Edge Cases

1. **OAuth providers for SAs**: Supported via the admin OAuth redirect flow (`GET .../connect/oauth`) and device code flow (`POST .../connect/device-code/initiate` + `POST .../connect/device-code/poll`). See `docs/ADMIN_SA_OAUTH_PLAN.md` for details
2. **Duplicate connections**: `store_api_key` already handles upserts -- if a token already exists for the SA+provider pair, it updates rather than creating a duplicate
3. **Connecting to inactive providers**: `store_api_key` already checks `is_active: true` on the provider config
4. **Deleted SA**: `get_service_account` returns `NotFound` for deleted (soft-deleted) SAs
5. **Concurrent admin operations**: MongoDB atomic operations handle concurrency. No additional locking needed.

## Testing Strategy

### Backend Unit/Integration Tests
- `list_sa_providers` returns empty list for SA with no providers
- `connect_api_key_for_sa` stores encrypted API key in `user_provider_tokens` with `user_id = sa_id`
- `connect_api_key_for_sa` rejects inactive SA
- `connect_api_key_for_sa` rejects empty API key
- `connect_api_key_for_sa` rejects API key > 4096 chars
- `connect_api_key_for_sa` rejects OAuth-type providers
- `disconnect_sa_provider` revokes token and clears encrypted data
- `disconnect_sa_provider` returns 404 for non-existent connection
- All endpoints return 403 for non-admin users

### Frontend Tests
- Hook tests: verify correct API URLs and query key patterns
- Component tests: "Connected Providers" section renders table with tokens
- Component tests: Connect dropdown only shows API key providers
- Component tests: Disconnect button calls mutation

## Success Criteria

- [ ] `GET /admin/service-accounts/{sa_id}/providers` returns provider token list
- [ ] `POST .../connect/api-key` stores encrypted API key with SA's ID as user_id
- [ ] `DELETE .../disconnect` revokes and clears provider token
- [ ] Admin panel shows "Connected Providers" section on SA detail page
- [ ] Admin can connect an API key provider to a SA via UI
- [ ] Admin can disconnect a provider from a SA via UI
- [ ] All actions produce audit log entries
- [ ] `cargo build` and `npm run build` succeed
- [ ] No security vulnerabilities (admin-only access enforced)
