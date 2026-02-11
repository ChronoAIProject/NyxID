# LLM Gateway Architecture Plan

## Overview

Add four features to NyxID that transform it from an auth/SSO platform into a full LLM gateway:
1. Auto-create downstream services when providers are seeded
2. Unified per-provider LLM endpoint (`/api/v1/llm/{provider_slug}/v1/*path`)
3. OpenAI-compatible gateway (`/api/v1/llm/gateway/v1/*path`) with format translation
4. Frontend "Ready to Use" status on connected providers

## Requirements

- Auto-seeded downstream services created idempotently at startup alongside providers
- Users can immediately proxy LLM requests after connecting a provider (no manual service setup)
- Unified endpoint uses provider slug for routing (no service IDs needed)
- Gateway normalizes all LLM APIs to OpenAI chat completions format
- Gateway determines target provider from the `model` field in the request body
- Frontend shows connection status and copyable proxy URLs
- Must follow existing layered architecture (handlers -> services -> models)
- Must use AppError, dedicated response structs, MongoDB conventions

---

## Architecture Changes

### Data Model Changes

#### 1. `DownstreamService` -- Add `provider_config_id` field

**File:** `backend/src/models/downstream_service.rs`

Add an optional field linking the downstream service to its provider:

```rust
/// Optional link to a ProviderConfig for auto-seeded LLM services.
/// When set, this service was auto-created for the provider's API.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub provider_config_id: Option<String>,
```

This field is used to:
- Find the downstream service for a given provider slug (unified endpoint)
- Identify auto-seeded services vs manually created ones
- Prevent duplicate auto-seeding

Add a compound index in `db::ensure_indexes()`:
```rust
// Index for looking up LLM services by provider_config_id
downstream_services: { "provider_config_id": 1 } (sparse)
```

#### 2. No New Collections

All new functionality uses existing collections (`downstream_services`, `service_provider_requirements`, `provider_configs`, `user_provider_tokens`). No schema migrations needed.

---

### Provider API Details (Research)

| Provider | Base URL | Auth Method | Auth Header/Param | Extra Headers | OpenAI-Compatible |
|----------|----------|-------------|-------------------|---------------|-------------------|
| OpenAI | `https://api.openai.com/v1` | Bearer | `Authorization` | -- | Yes (native) |
| OpenAI Codex | `https://api.openai.com/v1` | Bearer | `Authorization` | -- | Yes (native) |
| Anthropic | `https://api.anthropic.com/v1` | Header | `x-api-key` | `anthropic-version: 2023-06-01` | No (needs translation) |
| Google AI | `https://generativelanguage.googleapis.com/v1beta` | Query | `key` | -- | Yes (via `/v1beta/openai/` endpoint) |
| Mistral | `https://api.mistral.ai/v1` | Bearer | `Authorization` | -- | Yes (native) |
| Cohere | `https://api.cohere.com/v2` | Bearer | `Authorization` | -- | Partial (v2 chat is similar) |

### Model-to-Provider Mapping (for Gateway)

The gateway maps model names to providers using prefix matching:

| Model Prefix | Provider Slug |
|-------------|---------------|
| `gpt-`, `o1-`, `o3-`, `o4-`, `chatgpt-` | `openai` or `openai-codex` (prefer `openai`, fall back to `openai-codex`) |
| `claude-` | `anthropic` |
| `gemini-` | `google-ai` |
| `mistral-`, `codestral-`, `pixtral-`, `ministral-`, `open-mistral-` | `mistral` |
| `command-`, `embed-`, `rerank-` | `cohere` |

Priority: When a user has both `openai` (API key) and `openai-codex` (OAuth token) connected, prefer `openai` for `gpt-*` models since API key access is more reliable.

---

## Implementation Plan

### Phase 1: Auto-Seed Downstream Services

#### 1.1 Modify DownstreamService Model

**File:** `backend/src/models/downstream_service.rs`

- Add `provider_config_id: Option<String>` field (with `#[serde(default, skip_serializing_if = "Option::is_none")]`)
- Update existing tests to include the new field

#### 1.2 Add Seeding Function

**File:** `backend/src/services/provider_service.rs`

Add `seed_default_llm_services()` function called after `seed_default_providers()`:

```rust
pub async fn seed_default_llm_services(
    db: &mongodb::Database,
    encryption_key_hex: &str,
) -> AppResult<()>
```

For each provider, this function:
1. Finds the provider by slug
2. Checks if a downstream service with `provider_config_id == provider.id` already exists
3. If not, creates a `DownstreamService` with:
   - `slug`: `"llm-{provider_slug}"` (e.g., `"llm-openai"`)
   - `name`: `"{ProviderName} API"` (e.g., `"OpenAI API"`)
   - `base_url`: The provider's API base URL
   - `auth_method`: `"none"` (credentials come from delegation)
   - `auth_key_name`: `""` (empty, not used)
   - `credential_encrypted`: Empty string encrypted (field is required `Vec<u8>`)
   - `service_category`: `"internal"`
   - `requires_user_credential`: `false`
   - `provider_config_id`: `Some(provider.id)`
   - `is_active`: `true`
   - `created_by`: `"system"`
4. Creates a `ServiceProviderRequirement` linking the downstream service to the provider:
   - `required`: `true`
   - `injection_method`: provider-specific (see table above)
   - `injection_key`: provider-specific (see table above)

Seeding data:

```rust
struct LlmServiceSeed {
    provider_slug: &'static str,
    service_slug: &'static str,
    service_name: &'static str,
    base_url: &'static str,
    injection_method: &'static str,
    injection_key: &'static str,
}

const LLM_SERVICE_SEEDS: &[LlmServiceSeed] = &[
    LlmServiceSeed {
        provider_slug: "openai",
        service_slug: "llm-openai",
        service_name: "OpenAI API",
        base_url: "https://api.openai.com/v1",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    LlmServiceSeed {
        provider_slug: "openai-codex",
        service_slug: "llm-openai-codex",
        service_name: "OpenAI Codex API",
        base_url: "https://api.openai.com/v1",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    LlmServiceSeed {
        provider_slug: "anthropic",
        service_slug: "llm-anthropic",
        service_name: "Anthropic API",
        base_url: "https://api.anthropic.com/v1",
        injection_method: "header",
        injection_key: "x-api-key",
    },
    LlmServiceSeed {
        provider_slug: "google-ai",
        service_slug: "llm-google-ai",
        service_name: "Google AI API",
        base_url: "https://generativelanguage.googleapis.com/v1beta",
        injection_method: "query",
        injection_key: "key",
    },
    LlmServiceSeed {
        provider_slug: "mistral",
        service_slug: "llm-mistral",
        service_name: "Mistral AI API",
        base_url: "https://api.mistral.ai/v1",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    LlmServiceSeed {
        provider_slug: "cohere",
        service_slug: "llm-cohere",
        service_name: "Cohere API",
        base_url: "https://api.cohere.com/v2",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
];
```

#### 1.3 Call Seeding at Startup

**File:** `backend/src/main.rs`

Add after the existing `seed_default_providers()` call:

```rust
services::provider_service::seed_default_llm_services(&db, &config.encryption_key)
    .await
    .expect("Failed to seed default LLM services");
```

#### 1.4 Add Database Index

**File:** `backend/src/db.rs`

Add a sparse index on `downstream_services.provider_config_id` for efficient lookups:

```rust
downstream_services: { "provider_config_id": 1 } (sparse: true, unique: true)
```

---

### Phase 2: Unified LLM Endpoint

#### 2.1 New Service Module

**File:** `backend/src/services/llm_gateway_service.rs` (new)

Core resolution function:

```rust
/// Resolve a downstream service by provider slug.
/// Returns (DownstreamService, ProviderConfig) or error.
pub async fn resolve_llm_service_by_slug(
    db: &mongodb::Database,
    provider_slug: &str,
) -> AppResult<(DownstreamService, ProviderConfig)> {
    // 1. Find provider by slug
    // 2. Find downstream service where provider_config_id == provider.id
    // 3. Return both
}
```

LLM gateway status function:

```rust
/// Get the LLM gateway status for a user.
/// Returns which providers are ready (user has active token + auto-seeded service exists).
pub async fn get_llm_status(
    db: &mongodb::Database,
    user_id: &str,
    base_url: &str,
) -> AppResult<LlmStatusResponse> {
    // 1. Get all auto-seeded downstream services (provider_config_id is set)
    // 2. Get all user tokens
    // 3. Match services to tokens
    // 4. Build status with proxy URLs
}
```

Response types:

```rust
#[derive(Debug, Serialize)]
pub struct LlmProviderStatus {
    pub provider_slug: String,
    pub provider_name: String,
    pub status: String,           // "ready" | "not_connected" | "expired" | "error"
    pub proxy_url: String,        // e.g., "http://localhost:3001/api/v1/llm/openai/v1"
}

#[derive(Debug, Serialize)]
pub struct LlmStatusResponse {
    pub providers: Vec<LlmProviderStatus>,
    pub gateway_url: String,      // e.g., "http://localhost:3001/api/v1/llm/gateway/v1"
    pub supported_models: Vec<String>,  // list of model prefixes that can be routed
}
```

#### 2.2 New Handler Module

**File:** `backend/src/handlers/llm_gateway.rs` (new)

Three handler functions:

```rust
/// ANY /api/v1/llm/{provider_slug}/v1/{*path}
///
/// Forward the request to the provider's API using the user's stored credential.
/// This is a passthrough proxy -- no request/response translation.
pub async fn llm_proxy_request(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((provider_slug, path)): Path<(String, String)>,
    request: Request<Body>,
) -> AppResult<Response>
```

Flow:
1. Call `llm_gateway_service::resolve_llm_service_by_slug()` to get the downstream service
2. Use existing `proxy_service::resolve_proxy_target()` with the resolved service_id
3. Use existing `delegation_service::resolve_delegated_credentials()` for credential injection
4. Use existing `proxy_service::forward_request()` to forward
5. Return the response with header filtering (reuse existing allowlist)

```rust
/// GET /api/v1/llm/status
///
/// Return which LLM providers the user can use and their proxy URLs.
pub async fn llm_status(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<LlmStatusResponse>>
```

#### 2.3 Add Routes

**File:** `backend/src/routes.rs`

```rust
let llm_routes = Router::new()
    .route("/status", get(handlers::llm_gateway::llm_status))
    .route(
        "/gateway/v1/{*path}",
        axum::routing::any(handlers::llm_gateway::gateway_request),
    )
    .route(
        "/{provider_slug}/v1/{*path}",
        axum::routing::any(handlers::llm_gateway::llm_proxy_request),
    );

// In api_v1:
.nest("/llm", llm_routes)
```

Route ordering matters: `/gateway/v1/*` must be before `/{provider_slug}/v1/*` to avoid "gateway" being treated as a provider slug.

---

### Phase 3: OpenAI-Compatible Gateway

#### 3.1 Gateway Translation Layer

**File:** `backend/src/services/llm_gateway_service.rs` (extend)

##### Model-to-Provider Resolution

```rust
/// Determine which provider to route to based on the model name.
pub fn resolve_provider_for_model(model: &str) -> Option<&'static str> {
    let model_lower = model.to_lowercase();
    if model_lower.starts_with("gpt-")
        || model_lower.starts_with("o1-")
        || model_lower.starts_with("o3-")
        || model_lower.starts_with("o4-")
        || model_lower.starts_with("chatgpt-")
    {
        Some("openai")  // prefer openai, gateway handler falls back to openai-codex
    } else if model_lower.starts_with("claude-") {
        Some("anthropic")
    } else if model_lower.starts_with("gemini-") {
        Some("google-ai")
    } else if model_lower.starts_with("mistral-")
        || model_lower.starts_with("codestral-")
        || model_lower.starts_with("pixtral-")
        || model_lower.starts_with("ministral-")
        || model_lower.starts_with("open-mistral-")
    {
        Some("mistral")
    } else if model_lower.starts_with("command-")
        || model_lower.starts_with("embed-")
        || model_lower.starts_with("rerank-")
    {
        Some("cohere")
    } else {
        None
    }
}
```

##### Translation Trait

```rust
/// Trait for translating between OpenAI format and provider-native format.
pub trait LlmTranslator: Send + Sync {
    /// Translate an OpenAI-format request body to the provider's native format.
    /// Returns (modified_path, translated_body, extra_headers).
    fn translate_request(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> AppResult<TranslatedRequest>;

    /// Translate a provider's native response back to OpenAI format.
    fn translate_response(
        &self,
        body: serde_json::Value,
    ) -> AppResult<serde_json::Value>;

    /// Whether this provider needs request/response translation.
    fn needs_translation(&self) -> bool;

    /// Optional base URL override for gateway mode (e.g., Google's OpenAI-compatible endpoint).
    fn gateway_base_url(&self) -> Option<&str> {
        None
    }
}

pub struct TranslatedRequest {
    pub path: String,
    pub body: serde_json::Value,
    pub extra_headers: Vec<(String, String)>,
}
```

##### Translator Implementations

**PassthroughTranslator** (for OpenAI, OpenAI Codex, Mistral):
- `needs_translation()` -> `false`
- Passes request/response through unchanged

**AnthropicTranslator**:
- `needs_translation()` -> `true`
- `translate_request()`:
  1. Extract and remove `system` messages from `messages` array -> put in top-level `system` field
  2. Map `max_tokens` (default to 4096 if not specified, since Anthropic requires it)
  3. Map `model` field as-is (user specifies Claude model name)
  4. Map `temperature`, `top_p`, `stop` -> `stop_sequences`
  5. Change path: `chat/completions` -> `messages`
  6. Add extra header: `anthropic-version: 2023-06-01`
- `translate_response()`:
  1. Map `content[0].text` -> `choices[0].message.content`
  2. Map `stop_reason` -> `finish_reason` (`"end_turn"` -> `"stop"`, `"max_tokens"` -> `"length"`)
  3. Map `usage.input_tokens` -> `usage.prompt_tokens`, `usage.output_tokens` -> `usage.completion_tokens`
  4. Add `usage.total_tokens` = prompt + completion
  5. Wrap in OpenAI response envelope: `{ id, object: "chat.completion", created, model, choices, usage }`

**GoogleAiTranslator**:
- `needs_translation()` -> `false`
- `gateway_base_url()` -> `Some("https://generativelanguage.googleapis.com/v1beta/openai")`
- Uses Google's own OpenAI-compatible endpoint, so no translation needed
- Only the base URL changes

**CohereTranslator** (future, Cohere v2 is similar to OpenAI):
- `needs_translation()` -> `false` initially (Cohere v2 chat API is OpenAI-similar)
- Can add translation later if needed

##### Request Translation Detail: OpenAI -> Anthropic

Input (OpenAI format):
```json
{
  "model": "claude-sonnet-4-5-20250929",
  "messages": [
    {"role": "system", "content": "You are a helpful assistant."},
    {"role": "user", "content": "Hello"}
  ],
  "max_tokens": 1024,
  "temperature": 0.7,
  "stream": false
}
```

Output (Anthropic format):
```json
{
  "model": "claude-sonnet-4-5-20250929",
  "system": "You are a helpful assistant.",
  "messages": [
    {"role": "user", "content": "Hello"}
  ],
  "max_tokens": 1024,
  "temperature": 0.7,
  "stream": false
}
```

##### Response Translation Detail: Anthropic -> OpenAI

Input (Anthropic format):
```json
{
  "id": "msg_01XFDUDYJgAACzvnptvVoYEL",
  "type": "message",
  "role": "assistant",
  "content": [{"type": "text", "text": "Hello! How can I help?"}],
  "model": "claude-sonnet-4-5-20250929",
  "stop_reason": "end_turn",
  "usage": {"input_tokens": 25, "output_tokens": 10}
}
```

Output (OpenAI format):
```json
{
  "id": "chatcmpl-msg_01XFDUDYJgAACzvnptvVoYEL",
  "object": "chat.completion",
  "created": 1234567890,
  "model": "claude-sonnet-4-5-20250929",
  "choices": [{
    "index": 0,
    "message": {"role": "assistant", "content": "Hello! How can I help?"},
    "finish_reason": "stop"
  }],
  "usage": {"prompt_tokens": 25, "completion_tokens": 10, "total_tokens": 35}
}
```

#### 3.2 Gateway Handler

**File:** `backend/src/handlers/llm_gateway.rs` (extend)

```rust
/// ANY /api/v1/llm/gateway/v1/{*path}
///
/// OpenAI-compatible gateway. Accepts OpenAI-format requests, routes to the
/// correct provider based on the `model` field, translates request/response
/// formats as needed.
pub async fn gateway_request(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(path): Path<String>,
    request: Request<Body>,
) -> AppResult<Response>
```

Flow:
1. Read and parse request body as JSON
2. Extract `model` field from body
3. Call `resolve_provider_for_model(model)` to get provider slug
4. Try to find user's active token for the resolved provider
   - If user has both `openai` and `openai-codex` for GPT models, prefer `openai`
5. Resolve the downstream service for that provider
6. Get the appropriate translator
7. If translator `needs_translation()`:
   - Call `translate_request()` to get translated body and extra headers
   - Possibly override base URL via `gateway_base_url()`
8. Build the upstream request with:
   - Provider's base URL (or gateway override)
   - Translated path and body
   - Credential injected per the `ServiceProviderRequirement`
   - Extra headers from translator
9. Forward the request
10. If translator `needs_translation()`:
    - Parse response body
    - Call `translate_response()` to convert back to OpenAI format
11. Return response

Error cases:
- Missing `model` field -> `AppError::ValidationError("model field is required")`
- Unrecognized model -> `AppError::BadRequest("Unknown model: {model}. Cannot determine provider.")`
- Provider not connected -> `AppError::BadRequest("Provider '{name}' not connected. Connect at /providers.")`
- Downstream service not found -> `AppError::Internal("LLM service not configured for provider")`

---

### Phase 4: Frontend Changes

#### 4.1 New Types

**File:** `frontend/src/types/api.ts`

```typescript
export interface LlmProviderStatus {
  readonly provider_slug: string;
  readonly provider_name: string;
  readonly status: "ready" | "not_connected" | "expired" | "error";
  readonly proxy_url: string;
}

export interface LlmStatusResponse {
  readonly providers: readonly LlmProviderStatus[];
  readonly gateway_url: string;
  readonly supported_models: readonly string[];
}
```

#### 4.2 New Hook

**File:** `frontend/src/hooks/use-llm-gateway.ts` (new)

```typescript
export function useLlmStatus() {
  return useQuery({
    queryKey: ["llm-status"],
    queryFn: async (): Promise<LlmStatusResponse> => {
      return api.get<LlmStatusResponse>("/llm/status");
    },
  });
}
```

#### 4.3 LLM Status Badge Component

**File:** `frontend/src/components/dashboard/llm-ready-badge.tsx` (new)

A small badge shown on the ProviderCard when the provider has an auto-seeded LLM service AND the user has an active token:

- Shows "Ready to Use" badge (green) with a check icon
- On click/hover, shows a popover with:
  - Direct proxy URL (with copy button): `{base}/api/v1/llm/{slug}/v1`
  - Gateway URL (with copy button): `{base}/api/v1/llm/gateway/v1`
  - Quick example: `curl {proxy_url}/chat/completions -H "Authorization: Bearer YOUR_NYXID_TOKEN" -d '{"model": "...", "messages": [...]}'`

#### 4.4 Modify ProviderCard

**File:** `frontend/src/components/dashboard/provider-card.tsx`

- Import `useLlmStatus` hook (via prop or context)
- When the provider is connected AND has a matching LLM status entry with `status === "ready"`:
  - Show `<LlmReadyBadge>` next to the connection status
  - Show proxy URL below the connection info

Changes to `ProviderCardProps`:
```typescript
interface ProviderCardProps {
  // ...existing props...
  readonly llmStatus: LlmProviderStatus | undefined;
}
```

#### 4.5 Modify ProviderGrid

**File:** `frontend/src/components/dashboard/provider-grid.tsx`

- Fetch LLM status alongside providers and tokens
- Pass `llmStatus` prop to each ProviderCard

```typescript
const { data: llmStatus } = useLlmStatus();

// Map by provider slug
const llmStatusBySlug = new Map(
  llmStatus?.providers.map((s) => [s.provider_slug, s]) ?? []
);

// In render, pass to ProviderCard:
llmStatus={llmStatusBySlug.get(provider.slug)}
```

#### 4.6 Gateway Info Section on Providers Page

**File:** `frontend/src/pages/providers.tsx`

Add a collapsible info card at the top of the page showing:
- Gateway URL with copy button
- Brief explanation of how the gateway works
- List of connected providers that are "ready"
- Example curl command using the gateway

---

## File Summary

### New Files (Backend)

| File | Description |
|------|-------------|
| `backend/src/services/llm_gateway_service.rs` | Gateway logic: slug resolution, model mapping, translator trait + implementations |
| `backend/src/handlers/llm_gateway.rs` | HTTP handlers for `/api/v1/llm/*` routes |

### New Files (Frontend)

| File | Description |
|------|-------------|
| `frontend/src/hooks/use-llm-gateway.ts` | TanStack Query hook for LLM status |
| `frontend/src/components/dashboard/llm-ready-badge.tsx` | "Ready to Use" badge with proxy URL popover |

### Modified Files (Backend)

| File | Change |
|------|--------|
| `backend/src/models/downstream_service.rs` | Add `provider_config_id: Option<String>` field |
| `backend/src/services/provider_service.rs` | Add `seed_default_llm_services()` function |
| `backend/src/routes.rs` | Add `/api/v1/llm/*` route group |
| `backend/src/main.rs` | Call `seed_default_llm_services()` at startup |
| `backend/src/services/mod.rs` | Add `pub mod llm_gateway_service;` |
| `backend/src/handlers/mod.rs` | Add `pub mod llm_gateway;` |
| `backend/src/db.rs` | Add sparse index on `downstream_services.provider_config_id` |

### Modified Files (Frontend)

| File | Change |
|------|--------|
| `frontend/src/types/api.ts` | Add `LlmProviderStatus`, `LlmStatusResponse` types |
| `frontend/src/components/dashboard/provider-card.tsx` | Accept and display `llmStatus` prop |
| `frontend/src/components/dashboard/provider-grid.tsx` | Fetch LLM status, pass to cards |
| `frontend/src/pages/providers.tsx` | Add gateway info card |

---

## Error Handling

All new endpoints use `AppResult<T>` and existing `AppError` variants:

| Scenario | Error Variant | HTTP Status |
|----------|--------------|-------------|
| Missing `model` field in gateway request | `ValidationError` | 400 |
| Unrecognized model name | `BadRequest` | 400 |
| Provider not connected | `BadRequest` | 400 |
| Provider token expired | `BadRequest` | 400 |
| Auto-seeded service not found | `Internal` | 500 |
| Upstream provider returned error | Passthrough status | Upstream status |
| Request body too large (>10MB) | `BadRequest` | 400 |
| Translation failure | `Internal` | 500 |

No new `AppError` variants are needed.

---

## Security Considerations

1. **Credential handling**: All provider credentials remain encrypted at rest. Decryption only happens during request forwarding (existing pattern in `delegation_service`).

2. **Auth required**: All `/api/v1/llm/*` endpoints require `AuthUser` (JWT auth middleware).

3. **Rate limiting**: LLM endpoints inherit the global rate limiter. Consider adding stricter per-endpoint rate limiting in a future iteration.

4. **Request body limit**: Gateway reads the full body (up to 10MB, same as existing proxy limit) to extract the model name.

5. **Header allowlisting**: Reuse the existing `ALLOWED_FORWARD_HEADERS` and `ALLOWED_RESPONSE_HEADERS` from proxy handler.

6. **No credential leakage**: Translated responses do not include any credential information. Provider API keys/tokens are injected on the server side only.

7. **Path traversal**: Reuse existing `..` and `//` path checks from `proxy_service::forward_request`.

8. **SSRF protection**: Auto-seeded services use hardcoded, known-good base URLs (OpenAI, Anthropic, etc.). No user-supplied URLs.

---

## Testing Strategy

### Unit Tests

- `llm_gateway_service::resolve_provider_for_model()` -- all model prefix patterns, case insensitivity, unknown models
- `AnthropicTranslator::translate_request()` -- system message extraction, max_tokens default, parameter mapping
- `AnthropicTranslator::translate_response()` -- content extraction, stop_reason mapping, usage mapping
- `GoogleAiTranslator::gateway_base_url()` -- returns OpenAI-compatible endpoint
- Model prefix edge cases: empty string, partial matches, exact boundaries

### Integration Tests

- Auto-seeding creates downstream services and requirements
- Auto-seeding is idempotent (run twice, no duplicates)
- `resolve_llm_service_by_slug()` returns correct service
- LLM status endpoint returns correct provider statuses

### Manual/E2E Tests

- Connect OpenAI API key -> verify "Ready to Use" badge appears
- Call unified endpoint `/api/v1/llm/openai/v1/chat/completions` -> verify proxied correctly
- Call gateway endpoint with GPT model -> verify OpenAI receives request
- Call gateway endpoint with Claude model -> verify Anthropic receives translated request
- Call gateway endpoint with unknown model -> verify 400 error
- Disconnect provider -> verify "Ready to Use" badge disappears

---

## Streaming Support (Future Enhancement)

The initial implementation handles non-streaming requests only. Streaming support requires:

1. **Passthrough providers** (OpenAI, Mistral): Stream bytes directly -- relatively simple since no translation needed. Read the response as a byte stream and forward chunks.

2. **Translated providers** (Anthropic): Parse Anthropic SSE events (`content_block_delta`) and translate to OpenAI SSE format (`data: {"choices":[{"delta":{"content":"..."}}]}`). This requires an SSE parser and transformer.

3. **Detection**: Check for `"stream": true` in the request body to enable streaming mode.

Recommended approach for Phase 2:
- For passthrough providers, enable streaming by switching from `response.bytes()` to streaming the response body directly.
- For Anthropic, implement an SSE event transformer using `tokio::io::AsyncBufReadExt` to process the stream line by line.

---

## Implementation Order

1. **Phase 1** (Auto-seeding) -- Can be implemented and tested independently. No new API endpoints.
2. **Phase 2** (Unified endpoint + status) -- Depends on Phase 1. Adds the passthrough proxy.
3. **Phase 3** (Gateway) -- Depends on Phase 2. Adds the translation layer.
4. **Phase 4** (Frontend) -- Can start after Phase 2 (status endpoint). Gateway info after Phase 3.

Each phase can be committed and deployed independently.

---

## Success Criteria

- [ ] Auto-seeded downstream services created for all 6 providers at startup
- [ ] ServiceProviderRequirements correctly link services to providers
- [ ] Seeding is idempotent (no duplicates on restart)
- [ ] `/api/v1/llm/{slug}/v1/*` proxies requests to correct provider
- [ ] `/api/v1/llm/gateway/v1/chat/completions` routes by model name
- [ ] Anthropic translation produces valid API requests and responses
- [ ] Google AI uses OpenAI-compatible endpoint in gateway mode
- [ ] `/api/v1/llm/status` returns correct per-user provider readiness
- [ ] Frontend shows "Ready to Use" badge on connected providers
- [ ] Frontend shows copyable proxy URLs
- [ ] All new code uses AppError, response structs, and MongoDB conventions
- [ ] Unit tests for model mapping and translation logic
- [ ] No credential leakage in translated responses
