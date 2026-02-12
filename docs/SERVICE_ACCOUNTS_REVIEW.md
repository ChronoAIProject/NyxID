# Service Accounts Implementation Review

## Summary

The service accounts implementation is well-structured and closely follows the architecture plan. The code adheres to existing patterns (layer separation, MongoDB conventions, serde annotations, dedicated response types). The implementation covers the full CRUD lifecycle, client credentials authentication, token revocation, audit logging, and a functional frontend admin UI.

There are **1 critical**, **6 high**, **7 medium**, and **6 low** issues that should be addressed before merge. The critical issue is a timing attack vulnerability in the secret comparison. The high issues involve missing input validation, OAuth introspection/revocation gaps, inconsistent constraints between frontend and backend, and gaps in security logging.

---

## Critical Issues

### [CRITICAL] C1: Timing attack on client secret hash comparison

- **File:** `backend/src/services/service_account_service.rs:292`
- **Description:** The client secret hash is compared using standard string equality (`!=`), which performs byte-by-byte comparison with early termination. An attacker can measure response time differences to progressively determine the correct hash value.
- **Impact:** While the practical risk is reduced because SHA-256 hashes are being compared (not raw secrets), constant-time comparison is a security best practice for all authentication credential comparisons. Sophisticated attackers with precise timing measurements could potentially extract hash information.
- **Fix:** Use constant-time comparison. Add `subtle` crate to dependencies and use:

```rust
use subtle::ConstantTimeEq;

// Replace:
if sa.client_secret_hash != secret_hash {

// With:
if sa.client_secret_hash.as_bytes().ct_eq(secret_hash.as_bytes()).unwrap_u8() != 1 {
```

Alternatively, use `ring::constant_time::verify_slices_are_equal` if the `ring` crate is already a dependency.

---

## High Issues

### [HIGH] H1: OAuth introspection does not check SA token revocation

- **File:** `backend/src/handlers/oauth.rs:746-760`
- **Description:** The `introspect` handler checks refresh token revocation but has no branch for service account token revocation. When a service account's tokens are revoked (via admin panel or secret rotation), the introspection endpoint still reports them as `active: true`.
- **Impact:** Resource servers relying on token introspection (RFC 7662) to validate SA tokens would accept revoked tokens until they expire naturally, defeating the purpose of the revocation infrastructure.
- **Fix:** Add a revocation check for SA tokens after the refresh token check:

```rust
// After the refresh token revocation check, add:
if claims.sa == Some(true) {
    let sa_token = state
        .db
        .collection::<ServiceAccountToken>(SA_TOKENS)
        .find_one(doc! { "jti": &claims.jti })
        .await;
    match sa_token {
        Ok(Some(t)) if t.revoked => return Json(inactive),
        Err(_) => return Json(inactive),
        _ => {}
    }
}
```

### [HIGH] H2: OAuth revocation does not handle SA tokens

- **File:** `backend/src/handlers/oauth.rs:823-841`
- **Description:** The `revoke` handler only handles refresh token revocation. Service account access tokens (with `sa: true`) are treated as regular access tokens and silently ignored ("cannot be directly revoked without a blacklist"). However, SA tokens DO have a revocation mechanism via the `service_account_tokens` collection.
- **Impact:** External clients calling `POST /oauth/revoke` with an SA token get 200 OK (per RFC 7009), but the token remains active. The revocation infrastructure exists but is unreachable from the standard revocation endpoint.
- **Fix:** Add an SA token revocation branch before the access token comment:

```rust
// After the refresh token check:
if claims.sa == Some(true) {
    let _ = state
        .db
        .collection::<ServiceAccountToken>(SA_TOKENS)
        .update_one(
            doc! { "jti": &claims.jti, "revoked": false },
            doc! { "$set": { "revoked": true } },
        )
        .await;
    return StatusCode::OK;
}
```

### [HIGH] H3: No validation that role_ids exist in the roles collection

- **File:** `backend/src/services/service_account_service.rs:76` (create) and `:176` (update)
- **Description:** The plan (Section 9.5) specifies "each ID verified to exist in `roles` collection", but the implementation accepts any string values for `role_ids` without verifying they correspond to actual roles.
- **Impact:** Service accounts could be assigned non-existent role IDs. When permissions are resolved at request time, these phantom IDs would silently fail to match any roles, potentially causing confusion when debugging authorization issues.
- **Fix:** Add role existence validation in `create_service_account` and `update_service_account`:

```rust
if !role_ids.is_empty() {
    let existing_count = db
        .collection::<crate::models::role::Role>(crate::models::role::COLLECTION_NAME)
        .count_documents(doc! { "_id": { "$in": role_ids } })
        .await?;
    if existing_count != role_ids.len() as u64 {
        return Err(AppError::ValidationError(
            "One or more role IDs do not exist".to_string(),
        ));
    }
}
```

### [HIGH] H4: Frontend name max length mismatch with backend

- **File:** `frontend/src/schemas/service-accounts.ts:7` vs `backend/src/services/service_account_service.rs:50`
- **Description:** The Zod schema allows names up to 200 characters (`.max(200)`), but the backend validates 1-100 characters. Users entering 101-200 character names will pass frontend validation but be rejected by the backend.
- **Impact:** Users see an unhelpful server-side validation error instead of immediate client-side feedback.
- **Fix:** Align the frontend schema with the backend:

```typescript
// frontend/src/schemas/service-accounts.ts - both create and update schemas
name: z
  .string()
  .min(1, "Name is required")
  .max(100, "Name must be 100 characters or less"),
```

### [HIGH] H5: No audit logging for failed client credentials authentication

- **File:** `backend/src/services/service_account_service.rs:270-296` and `backend/src/handlers/oauth.rs:595-631`
- **Description:** Failed authentication attempts (invalid client_id, inactive account, wrong secret) are not audit logged. Only successful authentications are logged (`sa.token_issued`). Additionally, the successful log at `oauth.rs:610-620` passes `None` for both IP and user agent since the `token()` handler doesn't extract `headers: HeaderMap`.
- **Impact:** Security teams cannot detect brute-force attempts against service account credentials. Failed login monitoring is a fundamental security requirement. Source IPs of successful authentications are also lost.
- **Fix:** Add `headers: HeaderMap` to the `token()` handler signature, and log both successes and failures:

```rust
"client_credentials" => {
    let client_id = body.client_id.as_deref()
        .ok_or_else(|| AppError::BadRequest("Missing client_id".to_string()))?;
    let client_secret = body.client_secret.as_deref()
        .ok_or_else(|| AppError::BadRequest("Missing client_secret".to_string()))?;

    let result = service_account_service::authenticate_client_credentials(
        &state.db, &state.config, &state.jwt_keys,
        client_id, client_secret, body.scope.as_deref(),
    ).await;

    match result {
        Ok(response) => {
            audit_service::log_async(
                state.db.clone(), None, "sa.token_issued".to_string(),
                Some(serde_json::json!({ "client_id": client_id, "scope": &response.scope })),
                extract_ip(&headers), extract_user_agent(&headers),
            );
            Ok(Json(TokenResponse { /* ... */ }))
        }
        Err(e) => {
            audit_service::log_async(
                state.db.clone(), None, "sa.auth_failed".to_string(),
                Some(serde_json::json!({ "client_id": client_id })),
                extract_ip(&headers), extract_user_agent(&headers),
            );
            Err(e)
        }
    }
}
```

### [HIGH] H6: No max length validation for description on backend

- **File:** `backend/src/services/service_account_service.rs:163`
- **Description:** Neither `create_service_account` nor `update_service_account` validates the description length. The plan (Section 9.5) specifies "0-500 characters, optional", and the frontend allows up to 1000 characters. There is no server-side validation.
- **Impact:** Arbitrarily long descriptions could be stored, potentially causing storage abuse or UI rendering issues.
- **Fix:** Add description length validation in the service layer (for both create and update):

```rust
if let Some(d) = description {
    if d.len() > 500 {
        return Err(AppError::ValidationError(
            "Description must be 500 characters or less".to_string(),
        ));
    }
}
```

And update the frontend schema to match (500 instead of 1000):

```typescript
description: z
  .string()
  .max(500, "Description must be 500 characters or less")
  .optional()
  .or(z.literal("")),
```

---

## Medium Issues

### [MEDIUM] M1: Information disclosure via distinct error for inactive accounts

- **File:** `backend/src/services/service_account_service.rs:288-289`
- **Description:** An inactive service account returns `ServiceAccountInactive` (HTTP 403, error code 5001), while an invalid client_id returns `AuthenticationFailed` (HTTP 401, error code 2000). This allows an attacker to enumerate valid-but-inactive client_ids.
- **Impact:** Minor information disclosure. An attacker can determine which client_ids are registered in the system by distinguishing 401 vs 403 responses.
- **Fix:** Return the same generic "Invalid client credentials" error for both cases:

```rust
if !sa.is_active {
    return Err(AppError::AuthenticationFailed(
        "Invalid client credentials".to_string(),
    ));
}
```

### [MEDIUM] M2: Service account scopes are not validated against a known scope set

- **File:** `backend/src/services/service_account_service.rs:56-60`
- **Description:** The `allowed_scopes` field accepts any non-empty string without validating that the scopes are recognized by the system. The plan (Section 6.3) defines a specific scope vocabulary (`proxy:*`, `llm:proxy`, `connections:read`, etc.) but scopes are not validated against this set.
- **Impact:** Administrators could assign meaningless scopes (e.g., typos like `porxy:*`), which would silently fail to grant any access.
- **Fix:** Either validate against a known set, or document that scopes are free-form strings. If validating:

```rust
const KNOWN_SCOPES: &[&str] = &[
    "openid", "profile", "proxy:*", "llm:proxy", "llm:status",
    "connections:read", "connections:write",
    "providers:read", "providers:write",
];

fn validate_scope(scope: &str) -> bool {
    scope.split_whitespace().all(|s| {
        KNOWN_SCOPES.contains(&s) || s.starts_with("proxy:")
    })
}
```

### [MEDIUM] M3: Frontend detail page bypasses TanStack Router type safety

- **File:** `frontend/src/pages/admin-service-account-detail.tsx:57`
- **Description:** Uses `useParams({ strict: false }) as { saId: string }` which bypasses TanStack Router's compile-time type safety for route parameters.
- **Impact:** If the route parameter name is changed (e.g., from `saId` to `serviceAccountId`), this code would fail at runtime instead of being caught at compile time.
- **Fix:** Use the typed route reference:

```typescript
const { saId } = useParams({ from: "/admin/service-accounts/$saId" });
```

### [MEDIUM] M4: `rate_limit_override` accepts zero

- **File:** `backend/src/services/service_account_service.rs:79` and `backend/src/handlers/admin_service_accounts.rs:23`
- **Description:** The `rate_limit_override` field is `Option<u64>`, which accepts `0` as a valid value. A rate limit of 0 requests per second effectively blocks the service account entirely, which is different from deactivating it and produces no clear error.
- **Impact:** An admin could accidentally set rate limit to 0, causing silent denial of service for the service account.
- **Fix:** Validate `rate_limit_override > 0` when provided:

```rust
if let Some(rl) = rate_limit_override {
    if rl == 0 {
        return Err(AppError::ValidationError(
            "Rate limit override must be greater than 0".to_string(),
        ));
    }
}
```

### [MEDIUM] M5: Duplicate import in detail page

- **File:** `frontend/src/pages/admin-service-account-detail.tsx:16-17`
- **Description:** Two separate import statements from the same module:

```typescript
import { formatDate } from "@/lib/utils";
import { copyToClipboard } from "@/lib/utils";
```

- **Impact:** Code style issue. Should be a single import.
- **Fix:** Combine into one import:

```typescript
import { formatDate, copyToClipboard } from "@/lib/utils";
```

### [MEDIUM] M6: No way to clear description to `None`

- **File:** `backend/src/services/service_account_service.rs:163-165`
- **Description:** The update function treats `description: Some("")` by setting the field to an empty string, not `None`. There's no way to clear the description back to `null`.
- **Impact:** Once a description is set, it can only be replaced with another non-null value, not removed.
- **Fix:** Treat empty string as `None` for description:

```rust
if let Some(d) = description {
    if d.is_empty() {
        set_doc.insert("description", bson::Bson::Null);
    } else {
        set_doc.insert("description", d);
    }
}
```

### [MEDIUM] M7: Duplicate service account names allowed

- **File:** `backend/src/services/service_account_service.rs:41-91`
- **Description:** There's no uniqueness check or index on the `name` field. Multiple service accounts can be created with identical names.
- **Impact:** Admins could accidentally create duplicate service accounts, leading to confusion when managing them. The name is the primary identifier in the UI list view.
- **Fix:** Either add a unique index on `name` in `db.rs`, or check for existing accounts with the same name before creation, returning `AppError::Conflict`. A unique index is simpler but prevents future use of soft-deleted name reuse.

---

## Low Issues

### [LOW] L1: `AdminActionResponse` could be shared with existing admin handlers

- **File:** `backend/src/handlers/admin_service_accounts.rs:84-86`
- **Description:** The `AdminActionResponse` struct (single `message: String` field) is likely a duplicate of a similar struct in other admin handlers.
- **Impact:** Minor code duplication.
- **Fix:** Check if there's an existing `AdminActionResponse` in the admin handler module and reuse it.

### [LOW] L2: Frontend Zod schema uses string type for role_ids and rate_limit_override

- **File:** `frontend/src/schemas/service-accounts.ts:17-23`
- **Description:** The Zod schema defines `role_ids` and `rate_limit_override` as strings that are manually parsed in the handler (`split(",")`, `Number()`). The plan specified `role_ids` as `z.array(z.string())` and `rate_limit_override` as `z.number()`.
- **Impact:** Validation logic is split between the schema and the handler, reducing the effectiveness of schema-based validation.
- **Fix:** Acceptable for form-based inputs. Could be improved with `z.transform()` for the conversion.

### [LOW] L3: `create-service-account-dialog.tsx` not created as separate component

- **File:** Plan Section 11.2 vs `frontend/src/pages/admin-service-accounts.tsx`
- **Description:** The plan specified a dedicated dialog component at `components/dashboard/create-service-account-dialog.tsx`, but the create dialog was inlined into the list page. The list page is 487 lines.
- **Impact:** Within the 200-800 line guideline but extracting the dialog would improve maintainability.
- **Fix:** Acceptable as-is since the dialog is only used in one place. Extract if the file grows.

### [LOW] L4: No `is_active` toggle in the update form

- **File:** `frontend/src/pages/admin-service-account-detail.tsx:99-148`
- **Description:** The edit dialog does not include an `is_active` toggle. The only way to deactivate a service account is via the Delete button (soft-delete). While the backend supports updating `is_active` via PUT, the frontend doesn't expose this.
- **Impact:** Administrators cannot temporarily disable a service account and later re-enable it without API calls.
- **Fix:** Add a toggle/switch for `is_active` in the edit form, or add a separate "Disable"/"Enable" button on the detail page.

### [LOW] L5: `ConfirmDialog` component could be shared

- **File:** `frontend/src/pages/admin-service-account-detail.tsx:533-572`
- **Description:** The `ConfirmDialog` component at the bottom of the detail page is a generic confirmation dialog that could be reused across the application.
- **Impact:** Other pages likely need similar confirmation dialogs and would duplicate this code.
- **Fix:** Move to `components/shared/confirm-dialog.tsx` for reuse.

### [LOW] L6: `handleEdit` builds payload with `Record<string, unknown>`

- **File:** `frontend/src/pages/admin-service-account-detail.tsx:101`
- **Description:** The edit handler builds a `Record<string, unknown>` payload, bypassing TypeScript's type checking.
- **Impact:** Type errors in the payload construction won't be caught at compile time.
- **Fix:** Use `Partial<UpdateServiceAccountRequest>` or a dedicated type.

---

## Positive Observations

The following aspects of the implementation are done well:

1. **Layer separation**: Clean handler -> service -> model separation. Handlers use dedicated response types, never serializing model structs directly.

2. **MongoDB conventions**: Correct use of `#[serde(rename = "_id")]`, `bson_datetime` helpers, `COLLECTION_NAME` constants, and BSON roundtrip tests.

3. **Audit logging**: All admin operations (create, update, delete, rotate, revoke) are comprehensively audit logged with relevant context.

4. **Token revocation**: Proper design with JTI tracking, bulk revocation, and TTL indexes for automatic cleanup.

5. **Secret handling**: SHA-256 hashing, prefix storage for UI identification, one-time secret display with clear warnings.

6. **Middleware layering**: `reject_service_account_tokens` middleware correctly prevents SA tokens from reaching human-only endpoints. Defense-in-depth: even if middleware is bypassed, `require_admin` fails for SA tokens since they aren't in the `users` collection.

7. **Frontend UX**: One-time credential display with copy buttons and clear warning messages. Confirmation dialogs for destructive actions.

8. **Scope subsetting**: The `authenticate_client_credentials` function properly validates that requested scopes are a subset of the service account's allowed scopes.

9. **Test coverage**: Both model files have BSON roundtrip tests, the service has format/uniqueness tests, JWT has SA-specific tests, and the auth middleware has SA detection tests.

10. **Soft delete**: Delete operation deactivates rather than hard-deleting, preserving audit trail and preventing accidental data loss.

11. **Immutable frontend types**: All TypeScript interfaces use `readonly` modifiers, consistent with the project's immutability guidelines.

12. **Route grouping**: The three-tier route grouping (`api_v1_delegated`, `api_v1_shared`, `api_v1_human_only`) is clean and well-organized.

---

## Summary Table

| Severity | Count | Issues |
|----------|-------|--------|
| CRITICAL | 1 | C1 (timing attack on secret comparison) |
| HIGH | 6 | H1 (introspection gap), H2 (revocation gap), H3 (role_ids not validated), H4 (name length mismatch), H5 (missing failure audit logs + IP/UA), H6 (no description length validation) |
| MEDIUM | 7 | M1-M7 (info disclosure, scope validation, unsafe params, rate limit zero, duplicate import, description clearing, duplicate names) |
| LOW | 6 | L1-L6 (shared response type, schema types, component extraction, is_active toggle, shared dialog, untyped payload) |
