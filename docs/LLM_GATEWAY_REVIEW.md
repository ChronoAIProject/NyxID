# LLM Gateway Code & Security Review

## Summary

The LLM gateway implementation is well-structured and follows the project's layered architecture (handlers -> services -> models). Authentication is enforced on all endpoints, credentials are injected server-side (never exposed to clients), and audit logging is present. The code includes good unit test coverage for the translation layer and model resolution.

However, there are **3 HIGH**, **7 MEDIUM**, **6 LOW**, and **5 INFO** findings. The most significant issues are: (1) no SSE streaming support which will break `stream: true` requests, (2) the Anthropic response translator silently drops non-text content blocks, and (3) no size limit on upstream response bodies.

**Verdict: Merge with fixes for HIGH issues documented; MEDIUM issues should be tracked.**

---

## Issues Found

### CRITICAL

No critical issues found.

### HIGH

#### H-1: No SSE streaming support -- `stream: true` will break

**Files:** `backend/src/handlers/llm_gateway.rs:104,167,300-365`

Both `llm_proxy_request` and `gateway_request` read the entire upstream response body into memory via `downstream_response.bytes().await` (proxy path, line 504 of proxy helper) and `downstream_response.bytes().await` (gateway translated path, line 306). LLM APIs use Server-Sent Events (SSE) for `stream: true` requests. The current implementation:

- Buffers the entire SSE stream in memory before returning
- Returns the concatenated SSE chunks as a single response blob
- The client cannot parse this as a proper SSE stream

**Impact:** Any request with `"stream": true` will fail or return garbled output. This is the standard mode for most LLM client libraries.

**Suggested fix:** For Phase 1, reject `stream: true` requests with a clear error message. For Phase 2, implement proper SSE passthrough using `axum::body::Body::from_stream()` for passthrough providers and buffered translation for translated providers.

```rust
// Phase 1: Reject streaming
if body_json.get("stream").and_then(|v| v.as_bool()) == Some(true) {
    return Err(AppError::BadRequest(
        "Streaming is not yet supported via the gateway. Set stream: false.".to_string(),
    ));
}
```

---

#### H-2: Anthropic response translation silently drops non-text content blocks

**File:** `backend/src/services/llm_gateway_service.rs:326-338`

The `AnthropicTranslator::translate_response` only extracts the first `text` content block:

```rust
let content = body.get("content").and_then(|c| c.as_array())
    .and_then(|arr| arr.iter().find_map(|block| {
        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
            block.get("text").and_then(|t| t.as_str()).map(String::from)
        } else {
            None
        }
    }))
    .unwrap_or_default();
```

This silently drops:
- `tool_use` content blocks (function calling responses)
- Multiple text content blocks (only first is kept)
- `image` content blocks
- Any future content block types

**Impact:** Tool/function calling through the Anthropic gateway will silently lose responses. Users will see empty or partial results with no error.

**Suggested fix:** Concatenate all text blocks and map tool_use blocks to the OpenAI function call format:

```rust
// Collect all text content
let text_parts: Vec<String> = content_blocks.iter()
    .filter_map(|b| {
        if b.get("type")?.as_str()? == "text" {
            b.get("text")?.as_str().map(String::from)
        } else { None }
    })
    .collect();
let content = text_parts.join("");

// Map tool_use to OpenAI tool_calls format
let tool_calls: Vec<_> = content_blocks.iter()
    .filter_map(|b| { /* map to OpenAI tool_call format */ })
    .collect();
```

---

#### H-3: No size limit on upstream response body reads

**Files:** `backend/src/handlers/llm_gateway.rs:306,504`

When reading upstream responses, there is no size limit:

```rust
// Line 306 (gateway translated path)
let resp_bytes = downstream_response.bytes().await.map_err(|e| { ... })?;

// Line 504 (build_filtered_response)
let response_body = downstream_response.bytes().await.map_err(|e| { ... })?;
```

A misbehaving or compromised upstream provider could return an extremely large response, exhausting server memory. The request body has a 10MB limit but responses have none.

**Impact:** Memory exhaustion DoS if upstream returns unexpectedly large payloads.

**Suggested fix:** Use `response.bytes()` with a size check, or use `to_bytes` with a limit similar to the request body handling:

```rust
let resp_bytes = downstream_response
    .bytes()
    .await
    .map_err(|e| AppError::Internal(format!("Failed to read response: {e}")))?;

if resp_bytes.len() > 50 * 1024 * 1024 {
    return Err(AppError::Internal("Upstream response too large".to_string()));
}
```

---

### MEDIUM

#### M-1: Unused `_encryption_key` parameter in `resolve_provider_slug_with_fallback`

**File:** `backend/src/handlers/llm_gateway.rs:391`

```rust
async fn resolve_provider_slug_with_fallback(
    db: &mongodb::Database,
    _encryption_key: &[u8],  // unused
    user_id: &str,
    primary_slug: &str,
) -> AppResult<String> {
```

The `_encryption_key` parameter is accepted but never used (indicated by the leading underscore). This suggests either an incomplete implementation or a refactoring leftover.

**Suggested fix:** Remove the parameter if it's not needed, or add a TODO explaining the intended use.

---

#### M-2: Dead code -- `body_bytes.is_empty()` check unreachable in `gateway_request`

**File:** `backend/src/handlers/llm_gateway.rs:172-175,265-268`

At line 172, the function returns an error if the body is empty:
```rust
let body_json: serde_json::Value = if body_bytes.is_empty() {
    return Err(AppError::ValidationError(
        "Request body is required with a 'model' field".to_string(),
    ));
```

Then at line 265, there's another empty check that can never be true:
```rust
let body = if body_bytes.is_empty() {
    None
} else {
    Some(body_bytes)
};
```

**Suggested fix:** Remove the dead branch at line 265 and use `Some(body_bytes)` directly.

---

#### M-3: Forwarded Content-Length may mismatch after body translation

**File:** `backend/src/handlers/llm_gateway.rs:274, backend/src/services/proxy_service.rs:167-172`

The `convert_headers` function (line 464) converts all incoming headers including `content-length`. The `proxy_service::forward_request` allowlist includes `"content-length"`. When the Anthropic translator modifies the request body, the original `content-length` header is forwarded but the body is now a different size.

While `reqwest` typically recalculates `content-length` when setting a body, explicitly forwarding the wrong value could cause issues with strict upstream servers.

**Suggested fix:** Either remove `content-length` from `ALLOWED_FORWARD_HEADERS` in `proxy_service.rs` (let reqwest calculate it), or strip it in the LLM handler when translation occurs.

---

#### M-4: Duplicate CopyField / CopyableUrl components in frontend

**Files:** `frontend/src/components/dashboard/llm-ready-badge.tsx:19-64`, `frontend/src/components/dashboard/gateway-info-card.tsx:20-63`

`CopyField` in `llm-ready-badge.tsx` and `CopyableUrl` in `gateway-info-card.tsx` are nearly identical components with the same copy-to-clipboard logic. This violates DRY.

**Suggested fix:** Extract a shared `CopyableField` component into `components/ui/` or `components/shared/` and import it in both files.

---

#### M-5: Google AI translator logic flow is non-obvious

**File:** `backend/src/services/llm_gateway_service.rs:401-428`, `backend/src/handlers/llm_gateway.rs:242-271`

`GoogleAiTranslator::needs_translation()` returns `false`, but `gateway_base_url()` returns a custom URL. In `gateway_request`, the base URL override happens in the `else` branch (line 260-262) which handles "no translation needed". This split makes the code hard to follow -- the reader must understand that "no translation" still means "different base URL".

**Suggested fix:** Add a comment explaining the design, or refactor to separate "needs URL override" from "needs body translation":

```rust
// Google AI uses OpenAI-compatible format but at a different base URL.
// No body translation needed, but the base URL must be overridden.
```

---

#### M-6: Anthropic error responses forwarded in native format, not OpenAI format

**File:** `backend/src/handlers/llm_gateway.rs:359-362`

When the upstream Anthropic API returns a non-2xx response, the raw Anthropic error format is passed through:

```rust
} else {
    // For error responses, pass through as-is
    build_filtered_response(downstream_response).await?
}
```

Clients using the gateway expect OpenAI-format errors (e.g., `{"error": {"message": "...", "type": "...", "code": "..."}}`) but receive Anthropic-format errors (e.g., `{"type": "error", "error": {"type": "...", "message": "..."}}`).

**Suggested fix:** Translate error responses too, or wrap them in a gateway error envelope:

```rust
// Wrap upstream error in gateway format
Ok(serde_json::json!({
    "error": {
        "message": format!("Upstream provider error: {}", status),
        "type": "gateway_error",
        "code": status.as_u16(),
    }
}))
```

---

#### M-7: No LLM-specific rate limiting

**Files:** `backend/src/routes.rs:114-123`, `backend/src/main.rs:128-135`

LLM proxy requests trigger external API calls that cost money and have their own rate limits. The LLM endpoints share the global rate limiter (default 10 req/s, burst 30). This means:

1. An attacker could burn through a user's provider API quota rapidly
2. LLM requests compete with lightweight auth requests for rate limit budget
3. No per-user rate limiting on LLM endpoints specifically

**Suggested fix:** Add a dedicated, more restrictive rate limiter for LLM routes (e.g., 5 req/s per user) or at minimum document that the global rate limiter applies.

---

### LOW

#### L-1: No `staleTime` on `useLlmStatus` query

**File:** `frontend/src/hooks/use-llm-gateway.ts:6-11`

The TanStack Query hook uses default staleTime (0), causing refetches on every component mount/focus. LLM status changes infrequently (only when providers are connected/disconnected).

**Suggested fix:** Add `staleTime: 30_000` (30 seconds) to reduce unnecessary API calls.

---

#### L-2: Frontend `LlmProviderStatus.status` includes "error" not produced by backend

**Files:** `frontend/src/types/api.ts:271`, `backend/src/services/llm_gateway_service.rs:118-122`

Frontend type: `status: "ready" | "not_connected" | "expired" | "error"`
Backend produces: `"ready"`, `"expired"`, `"not_connected"` (never `"error"`)

**Suggested fix:** Either remove `"error"` from the frontend type, or add `"error"` handling in the backend for completeness.

---

#### L-3: `supported_models` list is hardcoded and may go stale

**File:** `backend/src/services/llm_gateway_service.rs:135-146`

The supported model patterns are hardcoded as static strings. As providers release new model families, this list won't update.

**Suggested fix:** Consider deriving this from the model resolution logic, or add a comment noting it needs manual updates.

---

#### L-4: Mutable `target` and `delegated` in `gateway_request`

**File:** `backend/src/handlers/llm_gateway.rs:218,227,247-248,277-284`

```rust
let mut target = proxy_service::resolve_proxy_target(...)?;
let mut delegated = delegation_service::resolve_delegated_credentials(...)?;
// Later:
target.base_url = base.to_string();
delegated.push(...);
```

This mutates variables in place, which goes against the project's coding style preference for immutability. While this is pragmatic Rust, it could be refactored to use shadow bindings or builder patterns.

---

#### L-5: Hardcoded model-to-provider mapping requires code changes for new models

**File:** `backend/src/services/llm_gateway_service.rs:155-184`

`resolve_provider_for_model()` uses hardcoded prefix matching. Adding support for a new provider (e.g., xAI/Grok) requires modifying this function.

**Suggested fix:** Consider a data-driven approach where model prefixes are stored in the database alongside provider configs.

---

#### L-6: Example curl templates use generic placeholder token

**Files:** `frontend/src/components/dashboard/llm-ready-badge.tsx:67-70`, `frontend/src/components/dashboard/gateway-info-card.tsx:72-78`

The example curl uses `YOUR_NYXID_TOKEN` which users might not understand how to replace. The curl example also doesn't show how to obtain the token.

**Suggested fix:** Add a brief note below the example: "Replace YOUR_NYXID_TOKEN with your NyxID access token from the login response."

---

### INFO

#### I-1: Architecture follows project conventions correctly

The LLM gateway follows the handler -> service -> model pattern. Handlers (`llm_gateway.rs`) handle HTTP concerns, services (`llm_gateway_service.rs`) contain business logic, and models are queried via proper collection constants.

#### I-2: Good unit test coverage for service layer

`llm_gateway_service.rs` includes 12 unit tests covering model resolution (all providers, unknown models, case insensitivity) and Anthropic translation (system message extraction, max_tokens defaulting, stop sequence mapping, response translation with different stop reasons).

#### I-3: Proper response struct usage

The handler returns `LlmStatusResponse` from the service layer rather than serializing model structs directly. This follows the project convention of dedicated response types.

#### I-4: Audit logging present on all LLM endpoints

Both `llm_proxy_request` and `gateway_request` call `audit_service::log_async` with relevant metadata (provider slug, method, path, model name).

#### I-5: Authentication enforced on all endpoints

All three LLM handlers (`llm_status`, `llm_proxy_request`, `gateway_request`) require the `AuthUser` extractor, ensuring unauthenticated requests are rejected.

---

## File-by-File Notes

### `backend/src/services/llm_gateway_service.rs` (new, 629 lines)

- Well-organized with clear section headers
- `LlmTranslator` trait design is clean and extensible
- `AnthropicTranslator` handles system message extraction, max_tokens defaulting, and stop sequence mapping correctly
- **H-2**: Response translation only handles text content blocks
- **L-3**: supported_models hardcoded
- **L-5**: Model resolution hardcoded
- Unit tests are comprehensive (12 tests)

### `backend/src/handlers/llm_gateway.rs` (new, 512 lines)

- **H-1**: No streaming support
- **H-3**: No response body size limit
- **M-1**: Unused `_encryption_key` parameter
- **M-2**: Dead code in body empty check
- **M-3**: Content-Length forwarding after translation
- **M-6**: Error responses not translated
- **L-4**: Mutable state
- Good: `ALLOWED_RESPONSE_HEADERS` allowlist prevents credential/sensitive header leakage
- Good: `build_filtered_response` properly filters response headers
- Good: Both endpoints have audit logging
- The `convert_headers` helper converts all headers; filtering happens in `proxy_service::forward_request`. This is safe but could be more explicit.

### `backend/src/models/downstream_service.rs` (modified)

- New `provider_config_id` field properly uses `#[serde(default, skip_serializing_if = "Option::is_none")]`
- Field is `Option<String>` which is correct for the nullable FK pattern
- Existing tests still pass (field included in roundtrip test at line 124)
- No issues found

### `backend/src/services/provider_service.rs` (modified)

- `seed_default_llm_services()` is well-structured and idempotent
- `LLM_SERVICE_SEEDS` const array is clean
- Empty credential encrypted at line 408 is intentional (LLM services use delegated user credentials, not master credentials)
- `ServiceProviderRequirement` creation at line 441-451 correctly links services to providers
- No issues found

### `backend/src/routes.rs` (modified)

- LLM routes properly nested under `/llm`
- Route ordering is correct: `/gateway/v1/{*path}` before `/{provider_slug}/v1/{*path}` ensures gateway takes priority
- Uses `axum::routing::any()` for proxy routes (correct for method passthrough)
- No issues found

### `backend/src/main.rs` (modified)

- `seed_default_llm_services` call at line 87-89 is properly placed after `seed_default_providers`
- Uses `.expect()` for startup-critical seeding (appropriate for startup)
- No issues found

### `backend/src/db.rs` (modified)

- New `provider_config_id` index (lines 200-212) uses sparse + unique, which is correct for nullable FK fields
- Sparse index means documents without the field are excluded from the unique constraint
- No issues found

### `backend/src/services/mod.rs` (modified)

- Module declaration added: `pub mod llm_gateway_service;`
- No issues found

### `backend/src/handlers/mod.rs` (modified)

- Module declaration added: `pub mod llm_gateway;`
- No issues found

### `backend/src/handlers/services.rs` (modified)

- `provider_config_id: None` added to `DownstreamService` construction at line 352
- Correctly set to `None` for manually created services
- No issues found

### `frontend/src/hooks/use-llm-gateway.ts` (new, 12 lines)

- Clean, follows project conventions
- **L-1**: Missing staleTime
- No other issues

### `frontend/src/components/dashboard/llm-ready-badge.tsx` (new, 106 lines)

- **M-4**: Duplicates CopyableUrl pattern
- **L-6**: Generic placeholder in example curl
- Uses `copyToClipboard` utility (safe, no XSS via clipboard API)
- Proper use of `readonly` on props
- Accessible: includes `sr-only` labels for icon buttons
- Values rendered in `<code>` elements are from API response data (URLs) - no XSS risk as React auto-escapes JSX expressions

### `frontend/src/components/dashboard/gateway-info-card.tsx` (new, 160 lines)

- **M-4**: Duplicates CopyField pattern
- **L-6**: Generic placeholder in example curl
- Proper use of `readonly` on props
- Clean expand/collapse UX pattern
- `String(readyProviders.length)` at line 97 is explicit conversion (good for strict mode)

### `frontend/src/components/ui/popover.tsx` (new, 27 lines)

- Standard shadcn/ui Popover component
- Proper forwardRef usage
- No issues found

### `frontend/src/types/api.ts` (modified)

- **L-2**: `LlmProviderStatus.status` includes `"error"` variant not produced by backend
- `LlmStatusResponse` properly uses `readonly` arrays
- Types are well-defined with proper readonly annotations

### `frontend/src/components/dashboard/provider-card.tsx` (modified)

- New `llmStatus` and `gatewayUrl` props properly typed with `readonly`
- `LlmReadyBadge` conditionally rendered only when `status === "ready"`
- No issues found

### `frontend/src/components/dashboard/provider-grid.tsx` (modified)

- Integrates `useLlmStatus()` hook
- `llmStatusBySlug` Map properly constructed for O(1) lookups
- `gatewayUrl` falls back to empty string when status unavailable
- No issues found

### `frontend/src/pages/providers.tsx` (modified)

- `GatewayInfoCard` conditionally rendered with `llmStatus !== undefined` check
- Clean integration
- No issues found

### `frontend/src/hooks/use-providers.ts` (modified)

- LLM status cache invalidation added to `useConnectApiKey`, `usePollDeviceCode`, `useDisconnectProvider`, `useRefreshProviderToken` mutations
- Uses `queryKey: ["llm-status"]` consistently
- No issues found

---

## Issue Summary

| Severity | Count |
|----------|-------|
| CRITICAL | 0 |
| HIGH | 3 |
| MEDIUM | 7 |
| LOW | 6 |
| INFO | 5 |
| **Total** | **21** |
